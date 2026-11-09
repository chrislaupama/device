#include "plugin_instance.hpp"
#include "../devices/builtin.hpp"

#include <algorithm>
#include <thread>

#if defined(__APPLE__)
#include <pthread.h>
#elif defined(__linux__)
#include <sys/syscall.h>
#include <unistd.h>
#endif

namespace saine {

const char* plugin_format_name(PluginFormat f) noexcept {
    switch (f) {
    case PluginFormat::AudioUnit: return "Audio Unit";
    case PluginFormat::Vst3: return "VST3";
    case PluginFormat::Clap: return "CLAP";
    case PluginFormat::Builtin: return "Built-in";
    }
    return "?";
}

#if !defined(__APPLE__) && !defined(__linux__)
namespace {
// Windows has no "main thread" query: remember the thread that ran the
// static initialisers, i.e. the process's initial thread (this library is
// linked statically into the executable).
const std::thread::id g_initial_thread = std::this_thread::get_id();
} // namespace
#endif

// True on the process's main thread (macOS: the Cocoa main thread; Linux:
// the initial thread; Windows: the thread that ran the static initialisers).
bool on_platform_main_thread() noexcept {
#if defined(__APPLE__)
    return pthread_main_np() != 0;
#elif defined(__linux__)
    // The initial thread's tid equals the process id.
    return static_cast<pid_t>(::syscall(SYS_gettid)) == ::getpid();
#else
    return std::this_thread::get_id() == g_initial_thread;
#endif
}

void PluginInstance::suspend() noexcept {
    suspended_.store(true, std::memory_order_seq_cst);
    while (in_process_.load(std::memory_order_seq_cst)) std::this_thread::yield();
}

bool PluginInstance::push_param(const ParamChange& c) noexcept {
    std::lock_guard lock(param_mutex_);
    return params_.push(c);
}

bool PluginInstance::set_parameter(uint32_t id, double value) noexcept {
    const ParamChange c{id, value};
    on_parameter_set(c);
    return push_param(c);
}

std::shared_ptr<PluginInstance> create_plugin_instance(const PluginSpec& spec, double sample_rate, uint32_t max_block,
                                                       std::string& err) {
    switch (spec.format) {
    case PluginFormat::AudioUnit: return create_au_instance(spec, sample_rate, max_block, err);
    case PluginFormat::Vst3: return create_vst3_instance(spec, sample_rate, max_block, err);
    case PluginFormat::Clap: return create_clap_instance(spec, sample_rate, max_block, err);
    case PluginFormat::Builtin: return create_builtin_instance(spec, sample_rate, max_block, err);
    }
    err = "unknown plugin format";
    return nullptr;
}

// ---------------------------------------------------------------- NoteFeeder

uint32_t NoteFeeder::build(const NoteEvent* notes, uint32_t note_count, uint64_t graph_seq, uint64_t transport_gen,
                           int64_t position, uint32_t frames, bool playing, MidiEvent* out, uint32_t cap) noexcept {
    uint32_t n = 0;
    const auto push = [&](int64_t offset, uint8_t status, uint8_t d1, uint8_t d2) {
        if (n < cap) out[n++] = MidiEvent{static_cast<uint32_t>(std::max<int64_t>(offset, 0)), status, d1, d2, 0};
    };
    const auto remove_at = [&](uint32_t i) { sounding_[i] = sounding_[--count_]; };

    // 1. Playback jump: everything that sounds stops.
    const bool discontinuity = playing && have_pos_ && position != expected_pos_;
    if (transport_gen != seen_gen_ || discontinuity) {
        seen_gen_ = transport_gen;
        if (count_ > 0) {
            for (uint32_t i = 0; i < count_; ++i) push(0, 0x80, sounding_[i].pitch, 0);
            push(0, 0xB0, 123, 0); // all notes off (for MIDI-driven formats)
        }
        count_ = 0;
        have_pos_ = false;
    }

    // 2. Republish: re-match sounding notes by (start, pitch).
    if (graph_seq != seen_seq_) {
        seen_seq_ = graph_seq;
        for (uint32_t i = 0; i < count_;) {
            Sounding& s = sounding_[i];
            const NoteEvent* it = std::lower_bound(notes, notes + note_count, s.start,
                                                   [](const NoteEvent& a, int64_t v) { return a.start < v; });
            bool found = false;
            for (; it != notes + note_count && it->start == s.start; ++it)
                if (it->pitch == s.pitch) {
                    s.end = it->end;
                    found = true;
                    break;
                }
            if (found) {
                ++i;
            } else {
                push(0, 0x80, s.pitch, 0);
                remove_at(i);
            }
        }
    }

    if (playing && frames > 0) {
        const int64_t end = position + frames;
        // 3. Note-offs due in this segment (or overdue after a re-match).
        for (uint32_t i = 0; i < count_;) {
            if (sounding_[i].end < end) {
                push(sounding_[i].end - position, 0x80, sounding_[i].pitch, 0);
                remove_at(i);
            } else {
                ++i;
            }
        }
        // 4. Note-ons starting in this segment.
        const NoteEvent* it = std::lower_bound(notes, notes + note_count, position,
                                               [](const NoteEvent& a, int64_t v) { return a.start < v; });
        for (; it != notes + note_count && it->start < end; ++it) {
            const int64_t on = it->start - position;
            // A retriggered pitch ends the previous note first (MIDI has one
            // voice per channel/key).
            for (uint32_t i = 0; i < count_; ++i)
                if (sounding_[i].pitch == it->pitch) {
                    push(on, 0x80, it->pitch, 0);
                    remove_at(i);
                    break;
                }
            push(on, 0x90, it->pitch, it->velocity);
            if (it->end < end) {
                push(it->end - position, 0x80, it->pitch, 0);
            } else if (count_ < kMaxSounding) {
                sounding_[count_++] = Sounding{it->start, it->end, it->pitch};
            } else {
                push(on, 0x80, it->pitch, 0); // no room to track it: cut
            }
        }
        expected_pos_ = end;
        have_pos_ = true;
    }

    // Stable insertion sort by offset; at equal offsets note-offs (and
    // CCs) precede note-ons. n is small.
    const auto key = [](const MidiEvent& e) {
        return (static_cast<uint64_t>(e.offset) << 1) | ((e.status & 0xF0) == 0x90 ? 1u : 0u);
    };
    for (uint32_t i = 1; i < n; ++i) {
        const MidiEvent e = out[i];
        uint32_t j = i;
        while (j > 0 && key(out[j - 1]) > key(e)) {
            out[j] = out[j - 1];
            --j;
        }
        out[j] = e;
    }
    return n;
}

} // namespace saine
