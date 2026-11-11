#include "midi_input.hpp"

#include <algorithm>
#include <cstring>
#include <limits>

namespace dstudio {

MidiInputHub::MidiInputHub(std::unique_ptr<MidiBackend> backend) : backend_(std::move(backend)) {
    for (auto& s : slots_) {
        s = std::make_unique<Slot>();
        s->hub = this;
    }
    slots_[kInjectSlot]->state.store(kOpen, std::memory_order_release);
}

MidiInputHub::~MidiInputHub() {
    std::lock_guard lock(ports_mutex_);
    if (backend_)
        for (uint32_t i = 0; i < kDeviceSlots; ++i)
            if (slots_[i]->state.load(std::memory_order_acquire) == kOpen) backend_->close(slots_[i]->id);
    backend_.reset(); // joins backend threads before the slots go away
}

MidiBackend& MidiInputHub::backend() {
    if (!backend_) backend_ = create_platform_midi_backend();
    return *backend_;
}

void MidiInputHub::receive(void* ctx, const uint8_t* msg, uint32_t len) noexcept {
    auto* s = static_cast<Slot*>(ctx);
    if (len < 2 || msg[0] < 0x80 || msg[0] >= 0xF0) return;
    Msg m;
    m.status = msg[0];
    m.data1 = msg[1] & 0x7F;
    m.data2 = len > 2 ? static_cast<uint8_t>(msg[2] & 0x7F) : 0;
    s->received.fetch_add(1, std::memory_order_relaxed);
    if (!s->queue.push(m)) s->dropped.fetch_add(1, std::memory_order_relaxed);
}

void MidiInputHub::list(std::vector<MidiPortState>& out) {
    std::lock_guard lock(ports_mutex_);
    std::vector<MidiPortInfo> ports;
    backend().list(ports);
    out.clear();
    out.reserve(ports.size());
    for (MidiPortInfo& p : ports) {
        MidiPortState st;
        for (uint32_t i = 0; i < kDeviceSlots; ++i) {
            const Slot& s = *slots_[i];
            if (s.state.load(std::memory_order_acquire) == kOpen && s.id == p.id) {
                st.open = true;
                st.events = s.received.load(std::memory_order_relaxed);
            }
        }
        st.info = std::move(p);
        out.push_back(std::move(st));
    }
}

void MidiInputHub::close_slot(Slot& s) {
    backend().close(s.id); // no more receive() for this slot after this
    s.id.clear();
    // The audio thread sends note-offs for what the slot holds, then frees it.
    s.state.store(kClosing, std::memory_order_release);
}

int32_t MidiInputHub::open_only(const std::vector<std::string>& ids, std::string& err) {
    std::lock_guard lock(ports_mutex_);
    MidiBackend& be = backend();
    std::vector<MidiPortInfo> present;
    be.list(present);
    const auto wanted = [&](const std::string& id) {
        return std::find(ids.begin(), ids.end(), id) != ids.end() &&
               std::any_of(present.begin(), present.end(), [&](const MidiPortInfo& p) { return p.id == id; });
    };
    // Close what is no longer wanted (or gone).
    for (uint32_t i = 0; i < kDeviceSlots; ++i) {
        Slot& s = *slots_[i];
        if (s.state.load(std::memory_order_acquire) == kOpen && !wanted(s.id)) close_slot(s);
    }
    int32_t rc = SE_OK;
    const auto failed = [&](const std::string& what) {
        if (!err.empty()) err += "; ";
        err += what;
        rc = SE_ERR_DEVICE;
    };
    for (const std::string& id : ids) {
        if (!wanted(id)) continue;
        bool is_open = false;
        for (uint32_t i = 0; i < kDeviceSlots; ++i)
            if (slots_[i]->state.load(std::memory_order_acquire) == kOpen && slots_[i]->id == id) is_open = true;
        if (is_open) continue;
        Slot* free_slot = nullptr;
        for (uint32_t i = 0; i < kDeviceSlots && free_slot == nullptr; ++i)
            if (slots_[i]->state.load(std::memory_order_acquire) == kFree) free_slot = slots_[i].get();
        if (free_slot == nullptr) {
            failed(id + ": too many MIDI inputs open (at most " + std::to_string(kDeviceSlots) + ")");
            continue;
        }
        free_slot->received.store(0, std::memory_order_relaxed);
        free_slot->id = id;
        std::string why;
        // Open before publishing kOpen: the RT side only pops, so an early
        // message just waits in the queue.
        if (!be.open(id, &MidiInputHub::receive, free_slot, why)) {
            free_slot->id.clear();
            failed(id + ": " + why);
            continue;
        }
        free_slot->state.store(kOpen, std::memory_order_release);
    }
    return rc;
}

bool MidiInputHub::inject(uint8_t status, uint8_t data1, uint8_t data2) noexcept {
    if (status < 0x80 || status >= 0xF0) return false;
    std::lock_guard lock(inject_mutex_);
    Slot& s = *slots_[kInjectSlot];
    const uint8_t msg[3] = {status, data1, data2};
    const uint64_t before = s.dropped.load(std::memory_order_relaxed);
    receive(&s, msg, 3);
    return s.dropped.load(std::memory_order_relaxed) == before;
}

void MidiInputHub::capture_enable(bool on) noexcept {
    capture_on_.store(false, std::memory_order_release);
    if (!on) return;
    se_midi_captured c;
    while (capture_.pop(c)) {
    }
    capture_restart_.store(true, std::memory_order_release);
    capture_on_.store(true, std::memory_order_release);
}

std::size_t MidiInputHub::capture_read(se_midi_captured* out, std::size_t capacity) noexcept {
    std::size_t n = 0;
    while (n < capacity && capture_.pop(out[n])) ++n;
    return n;
}

void MidiInputHub::capture(const MidiEvent& e, int64_t frame) noexcept {
    const uint8_t kind = e.status & 0xF0;
    if (kind != 0x90 && kind != 0x80) return;
    se_midi_captured c{};
    c.frame = frame;
    c.status = kind;
    c.data1 = e.data1;
    c.data2 = e.data2;
    if (!capture_.push(c)) capture_dropped_.fetch_add(1, std::memory_order_relaxed);
}

void MidiInputHub::stats(se_midi_stats* out) const noexcept {
    *out = se_midi_stats{};
    for (const auto& s : slots_) {
        out->received += s->received.load(std::memory_order_relaxed);
        out->dropped += s->dropped.load(std::memory_order_relaxed);
    }
    out->delivered = delivered_.load(std::memory_order_relaxed);
    out->capture_dropped = capture_dropped_.load(std::memory_order_relaxed);
}

uint32_t MidiInputHub::drain(MidiEvent* out, uint32_t cap) noexcept {
    uint32_t n = 0;
    for (auto& sp : slots_) {
        Slot& s = *sp;
        const uint8_t state = s.state.load(std::memory_order_acquire);
        if (state == kFree) continue;
        Msg m;
        while (n < cap && s.queue.pop(m)) {
            const uint8_t kind = m.status & 0xF0;
            MidiEvent e;
            e.offset = 0;
            if (kind == 0x90 && m.data2 > 0) {
                s.held[m.data1] = true;
                e.status = 0x90;
                e.data1 = m.data1;
                e.data2 = m.data2;
            } else if (kind == 0x80 || kind == 0x90) {
                if (!s.held[m.data1]) continue; // nothing to release
                s.held[m.data1] = false;
                e.status = 0x80;
                e.data1 = m.data1;
                e.data2 = kind == 0x80 ? m.data2 : 64;
            } else if (kind == 0xC0) {
                continue; // program changes would switch plugin programs behind the project's back
            } else {
                e.status = kind; // controllers, pitch bend, aftertouch: channel 1
                e.data1 = m.data1;
                e.data2 = m.data2;
            }
            out[n++] = e;
        }
        if (state == kClosing && s.queue.empty()) {
            // Release what the port still held, then free the slot.
            bool done = true;
            for (uint32_t p = 0; p < 128; ++p) {
                if (!s.held[p]) continue;
                if (n == cap) {
                    done = false;
                    break;
                }
                s.held[p] = false;
                MidiEvent e;
                e.status = 0x80;
                e.data1 = static_cast<uint8_t>(p);
                e.data2 = 64;
                out[n++] = e;
            }
            if (done) s.state.store(kFree, std::memory_order_release);
        }
    }
    delivered_.fetch_add(n, std::memory_order_relaxed);
    return n;
}

// ------------------------------------------------------------- LiveSynth

void LiveSynth::remove(uint8_t pitch) noexcept {
    uint32_t w = 0;
    for (uint32_t r = 0; r < count_; ++r)
        if (notes_[r].pitch != pitch) notes_[w++] = notes_[r];
    if (w != count_) ++seq_;
    count_ = w;
}

void LiveSynth::note_on(uint8_t pitch, uint8_t velocity) noexcept {
    remove(pitch); // a repeated key retriggers
    if (count_ == notes_.size()) return;
    NoteEvent& n = notes_[count_++];
    n.start = clock_;
    n.end = std::numeric_limits<int64_t>::max();
    n.pitch = pitch;
    n.velocity = velocity;
    ++seq_;
}

void LiveSynth::note_off(uint8_t pitch) noexcept { remove(pitch); }

bool LiveSynth::render(const se_synth_params* params, uint32_t frames, float* out) noexcept {
    const bool sound = synth_.render(notes_.data(), count_, params, seq_, 0, clock_, frames, true, out);
    clock_ += frames;
    return sound;
}

// ------------------------------------------------------------- TrackMidi

uint32_t TrackMidi::events_for(const MidiEvent* in, uint32_t n, bool routed, MidiEvent* out, uint32_t cap) noexcept {
    uint32_t k = 0;
    if (routed) {
        for (uint32_t i = 0; i < n && k < cap; ++i) {
            const MidiEvent& e = in[i];
            const uint8_t kind = e.status & 0xF0;
            if (kind == 0x90) {
                if (!held[e.data1]) ++held_count;
                held[e.data1] = true;
            } else if (kind == 0x80) {
                if (!held[e.data1]) continue; // the key went down while another track had the input
                held[e.data1] = false;
                --held_count;
            }
            out[k++] = e;
        }
        return k;
    }
    if (held_count == 0) return 0;
    for (uint32_t p = 0; p < 128 && k < cap; ++p) {
        if (!held[p]) continue;
        held[p] = false;
        --held_count;
        MidiEvent e;
        e.status = 0x80;
        e.data1 = static_cast<uint8_t>(p);
        e.data2 = 64;
        out[k++] = e;
    }
    return k;
}

} // namespace dstudio
