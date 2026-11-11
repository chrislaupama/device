// MIDI input: from the platform's MIDI thread(s) to instruments on the audio
// thread, plus the capture ring the app records from.
//
//   MIDI thread(s) (backend)          audio thread (RT owner)            main thread
//   ------------------------          -----------------------            -----------
//   rx(): message -> slot queue  -->  drain(): all slots, normalise  -->
//         (SPSC per slot, counted)    route per track (render_block)
//                                     capture(): timeline frame      --> capture_read()
//
// One slot per open port (kDeviceSlots) plus one slot for injected events
// (se_midi_inject, tests). A slot's queue has exactly one producer: the
// backend calls for one port never overlap, and injection is serialised by
// a mutex on the main side. Closing a port marks its slot Closing; the audio
// thread then sends note-offs for the notes that slot still holds and frees
// the slot, so closing an input never leaves a note hanging. While nothing
// renders, open_only() frees Closing slots itself under the RT role
// (reclaim_closing()), and the note-offs wait for the next block.
//
// Several ports can hold the same key: the hub counts holders per key and
// passes a note-off on only when the last one lets go. A slot whose queue
// overflowed may have lost a note-off, so the next drain releases every key
// that slot holds once everything queued before the loss has been delivered.
//
// Nothing here allocates, locks or blocks on the RT side. See
// specs/003-midi-input-and-recording/plan.md.
#pragma once

#include "platform/midi_backend.hpp"
#include "plugins/plugin_instance.hpp"
#include "spsc_queue.hpp"
#include "synth.hpp"
#include "dstudio_engine.h"

#include <array>
#include <atomic>
#include <cstdint>
#include <functional>
#include <memory>
#include <mutex>
#include <string>
#include <vector>

namespace dstudio {

// One port as listed for the app.
struct MidiPortState {
    MidiPortInfo info;
    bool open = false;
    uint64_t events = 0; // messages received since it was opened (activity light)
};

class MidiInputHub {
public:
    static constexpr uint32_t kDeviceSlots = 16;
    static constexpr uint32_t kInjectSlot = kDeviceSlots;
    static constexpr uint32_t kSlots = kDeviceSlots + 1;
    static constexpr std::size_t kQueueSize = 1024;
    static constexpr uint32_t kMaxBlockEvents = 256; // delivered per callback; the rest waits
    static constexpr std::size_t kCaptureSize = 4096;

    // `backend` null: created on first use with create_platform_midi_backend()
    // (so engines that never list or open MIDI never touch the MIDI system).
    explicit MidiInputHub(std::unique_ptr<MidiBackend> backend = nullptr);
    ~MidiInputHub(); // closes every port (no backend call runs afterwards)
    MidiInputHub(const MidiInputHub&) = delete;
    MidiInputHub& operator=(const MidiInputHub&) = delete;

    // ---- main thread
    void list(std::vector<MidiPortState>& out);
    // Opens exactly the listed ports that exist (others are closed). Returns
    // SE_OK, or SE_ERR_DEVICE with `err` naming the ports that failed (the
    // rest are still opened). When every slot is taken and some are Closing,
    // `reclaim` is called: it runs reclaim_closing() if it can take the RT
    // role (nothing renders) and returns true; false means a renderer will
    // free them, and open_only waits briefly for it.
    using Reclaim = std::function<bool()>;
    int32_t open_only(const std::vector<std::string>& ids, std::string& err, const Reclaim& reclaim = {});
    // Queues one channel message as if a port had sent it. False when the
    // message is not a channel message or the queue is full.
    bool inject(uint8_t status, uint8_t data1, uint8_t data2) noexcept;
    void set_focus(uint64_t track_id) noexcept { focus_.store(track_id, std::memory_order_relaxed); }
    // Capture ring: enabling starts a new pass (events captured for an
    // earlier pass are never read); reading pops. Both are serialised, so
    // any thread may call them.
    void capture_enable(bool on) noexcept;
    std::size_t capture_read(se_midi_captured* out, std::size_t capacity) noexcept;
    void stats(se_midi_stats* out) const noexcept;

    // ---- RT (the RT owner only)
    // Moves up to `cap` normalised events to `out` (offset 0): channel 1,
    // note-on velocity 0 becomes note-off, note-offs without a held note-on
    // and program changes are dropped. Frees Closing slots after sending
    // note-offs for what they held.
    uint32_t drain(MidiEvent* out, uint32_t cap) noexcept;
    // RT owner with nothing rendering (open_only's `reclaim`): frees Closing
    // slots now. Their note-offs go out with the next drain.
    void reclaim_closing() noexcept;
    uint64_t focus() const noexcept { return focus_.load(std::memory_order_relaxed); }
    // The capture pass in progress: 0 = not capturing.
    uint32_t capture_pass() const noexcept {
        const uint32_t st = capture_state_.load(std::memory_order_acquire);
        return (st & 1u) != 0 ? st >> 1 : 0;
    }
    // Records `e` heard at timeline `frame` (notes only) for `pass`
    // (capture_pass() read before the events were captured). Dropped and
    // counted when the ring is full.
    void capture(const MidiEvent& e, int64_t frame, uint32_t pass) noexcept;
    // The transport wrapped at the loop end during `pass` (RT, at the wrap).
    void capture_note_wrap(uint32_t pass) noexcept { capture_wrapped_.store(pass, std::memory_order_release); }
    // Whether the latest pass (running or stopped) saw a loop wrap: a real
    // wrap, never a seek. Any thread.
    bool capture_wrapped() const noexcept {
        const uint32_t pass = capture_state_.load(std::memory_order_acquire) >> 1;
        return pass != 0 && capture_wrapped_.load(std::memory_order_acquire) == pass;
    }

private:
    enum SlotState : uint8_t { kFree = 0, kOpen = 1, kClosing = 2 };
    struct Msg {
        uint8_t status = 0, data1 = 0, data2 = 0, _pad = 0;
    };
    struct Slot {
        SpscQueue<Msg, kQueueSize> queue;
        std::atomic<uint8_t> state{kFree};
        std::atomic<bool> overflowed{false}; // a message was dropped since the last drain
        bool recovering = false;             // RT-owned: overflowed; release its keys once the queue is empty
        std::atomic<uint64_t> received{0};
        std::atomic<uint64_t> dropped{0};
        std::array<bool, 128> held{}; // RT-owned
        std::string id;               // main-owned (ports_mutex_)
        MidiInputHub* hub = nullptr;
    };
    struct Captured {
        se_midi_captured event;
        uint32_t pass = 0;
    };
    static void receive(void* ctx, const uint8_t* msg, uint32_t len) noexcept;
    // RT: one key of slot `s` goes up. A note-off is due when no other port
    // still holds the key: written to out[n] while there is room, else kept
    // for the next drain.
    void release(Slot& s, uint8_t pitch, uint8_t velocity, MidiEvent* out, uint32_t& n, uint32_t cap) noexcept;
    void release_all(Slot& s, MidiEvent* out, uint32_t& n, uint32_t cap) noexcept;
    MidiBackend& backend(); // requires ports_mutex_
    void close_slot(Slot& s); // requires ports_mutex_
    // The slot, allocated on first use (main thread, under ports_mutex_ or
    // inject_mutex_). Slots are never freed before the hub, so an engine that
    // never uses MIDI allocates nothing and its audio thread skips the hub.
    Slot& ensure_slot(uint32_t i);
    Slot* slot(uint32_t i) const noexcept { return slots_[i].load(std::memory_order_acquire); }

    std::mutex ports_mutex_;
    std::unique_ptr<MidiBackend> backend_;
    std::array<std::atomic<Slot*>, kSlots> slots_{};
    std::array<std::unique_ptr<Slot>, kSlots> owned_;
    std::atomic<uint32_t> allocated_{0}; // bit i: slots_[i] exists
    std::mutex alloc_mutex_;
    std::mutex inject_mutex_;
    std::atomic<uint64_t> focus_{0};
    // RT-owned: ports holding each key, and note-offs owed (no room last drain,
    // or slots reclaimed while nothing rendered).
    std::array<uint8_t, 128> holders_{};
    std::array<bool, 128> owed_off_{};
    uint32_t owed_count_ = 0;

    // bit 0: capturing; bits 1..: the pass number (bumped by capture_enable(true)).
    std::atomic<uint32_t> capture_state_{0};
    std::mutex capture_mutex_; // capture_enable / capture_read (any thread; the ring has one consumer)
    std::atomic<uint64_t> capture_dropped_{0};
    std::atomic<uint32_t> capture_wrapped_{0}; // the pass number that last saw a loop wrap
    std::atomic<uint64_t> delivered_{0};
    // Allocated by the first capture_enable(true), published by capture_state_.
    std::unique_ptr<SpscQueue<Captured, kCaptureSize>> capture_;
};

// The built-in Synth played live. The Synth renders notes from a timeline
// list and has no live entry point, so this keeps its own free-running clock
// and turns each held key into a note that started "now" and has not ended:
// a key release removes the note, which releases the voice. It only uses
// Synth::render(), and live notes never take voices from sequenced ones.
class LiveSynth {
public:
    explicit LiveSynth(double sample_rate) : synth_(sample_rate) {}
    void note_on(uint8_t pitch, uint8_t velocity) noexcept;
    void note_off(uint8_t pitch) noexcept;
    // Renders `frames` frames (mono) into `out`; false (out untouched) when silent.
    bool render(const se_synth_params* params, uint32_t frames, float* out) noexcept;
    uint32_t held() const noexcept { return count_; }

private:
    void remove(uint8_t pitch) noexcept;
    Synth synth_;
    bool sounding_ = false; // the last render made sound (release tails still ring)
    std::array<NoteEvent, 128> notes_{}; // held keys, sorted by start
    uint32_t count_ = 0;
    int64_t clock_ = 0;
    uint64_t seq_ = 1;
};

// Live MIDI state of one instrument track, kept across republishes by
// track id (like the track's Synth). RT-owned once installed.
struct TrackMidi {
    std::array<bool, 128> held{}; // keys this track got a note-on for and no note-off yet
    uint32_t held_count = 0;
    std::unique_ptr<LiveSynth> live; // tracks that play the built-in Synth

    // RT: the events this track gets this block. Routed: `in` (copied to
    // `out`); not routed: note-offs for the keys it still holds. Updates
    // `held`. Returns the count written (at most `cap`).
    uint32_t events_for(const MidiEvent* in, uint32_t n, bool routed, MidiEvent* out, uint32_t cap) noexcept;
};

// Timeline frame a captured event is placed at: `position - latency`, folded
// back into the loop [loop_start, loop_end) when playback has wrapped since
// the capture started and is inside the loop (the performer heard the end of
// the previous pass).
inline int64_t midi_capture_frame(int64_t position, int64_t latency, bool wrapped, bool loop_on, int64_t loop_start,
                                  int64_t loop_end) noexcept {
    const int64_t frame = position - latency;
    const int64_t len = loop_end - loop_start;
    if (!wrapped || !loop_on || len <= 0 || position < loop_start || position >= loop_end || frame >= loop_start)
        return frame;
    const int64_t back = (loop_start - frame) % len;
    return back == 0 ? loop_start : loop_end - back;
}

// The MIDI input of one render_block() call. `events` go to the routed
// tracks (only in a callback's first segment; later segments pass none but
// still release held notes). Routing: if any instrument track is armed or
// monitoring (RenderGraph::midi_explicit), those tracks; else the track
// whose id is `focus`.
struct MidiBlock {
    const MidiEvent* events = nullptr;
    uint32_t count = 0;
    uint64_t focus = 0;
};

} // namespace dstudio
