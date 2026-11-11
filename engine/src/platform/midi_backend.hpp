// Platform MIDI input. All platform-specific MIDI code lives in
// engine/src/platform/; the engine sees this interface only (see
// midi_input.hpp for how messages reach the audio thread).
//
// Exactly one translation unit provides create_platform_midi_backend():
//   coremidi_input.cpp      macOS   (Core MIDI, MIDI 1.0 protocol input port)
//   winmm_midi_input.cpp    Windows (multimedia midiIn* API)
//   alsa_midi_input.cpp     Linux   (ALSA sequencer, one reader thread)
//   midi_backend_stub.cpp   anything else, or Linux without ALSA (no ports)
#pragma once

#include <cstdint>
#include <memory>
#include <string>
#include <vector>

namespace dstudio {

// One MIDI input port. `id` is stable across rescans and app restarts as far
// as the platform allows (Core MIDI unique id, port name elsewhere); it is
// what the app stores in its preferences.
struct MidiPortInfo {
    std::string id;
    std::string name;
};

class MidiBackend {
public:
    // Called on a backend thread with one complete channel message (status
    // byte 0x80..0xEF and its data bytes; `len` 2 or 3). System messages
    // (sysex, clock, active sensing) are never passed on. Calls for one
    // opened port never overlap, so the receiver can be a single producer
    // per port. Must not block.
    using Receive = void (*)(void* ctx, const uint8_t* msg, uint32_t len) noexcept;

    virtual ~MidiBackend() = default;
    // Lists the input ports present now. Non-RT; may take a few ms.
    virtual void list(std::vector<MidiPortInfo>& out) = 0;
    // Opens port `id` and starts delivering its messages to `rx(ctx, ...)`.
    // False + `err` when the port is gone or cannot be opened.
    virtual bool open(const std::string& id, Receive rx, void* ctx, std::string& err) = 0;
    // Stops delivery for `id`. After it returns, `rx` is not called again
    // for that port. Closing a port that is not open does nothing.
    virtual void close(const std::string& id) = 0;
};

// The platform backend, or a backend with no ports when the platform has no
// MIDI system (no ALSA sequencer, no Core MIDI server). Never null.
std::unique_ptr<MidiBackend> create_platform_midi_backend();

// Splits a running-status MIDI 1.0 byte stream into channel messages
// (shared by the backends that receive raw bytes). Feed bytes in order;
// `emit` is called per complete channel message. System common and
// real-time bytes are skipped; sysex is skipped up to its end.
class MidiByteParser {
public:
    template <class Emit>
    void feed(uint8_t b, Emit&& emit) noexcept {
        if (b >= 0xF8) return; // real-time: may appear anywhere, ignored
        if (b >= 0x80) {
            if (b >= 0xF0) { // system common / sysex: cancels running status
                status_ = 0;
                in_sysex_ = b == 0xF0;
                return;
            }
            in_sysex_ = false;
            status_ = b;
            count_ = 0;
            return;
        }
        if (in_sysex_ || status_ == 0) return;
        data_[count_++] = b;
        if (count_ == needed(status_)) {
            const uint8_t msg[3] = {status_, data_[0], data_[1]};
            emit(msg, static_cast<uint32_t>(1 + count_));
            count_ = 0; // running status: the next data bytes reuse status_
        }
    }

    static uint32_t needed(uint8_t status) noexcept {
        const uint8_t kind = status & 0xF0;
        return kind == 0xC0 || kind == 0xD0 ? 1u : 2u;
    }

private:
    uint8_t status_ = 0;
    uint8_t data_[2] = {0, 0};
    uint32_t count_ = 0;
    bool in_sysex_ = false;
};

} // namespace dstudio
