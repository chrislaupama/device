// Windows MIDI input through the multimedia API (midiIn*).
//
// Why winmm: it is on every Windows version, needs no extra runtime, and
// every class-compliant USB keyboard shows up in it. (Windows MIDI Services
// can replace it later for MIDI 2.0 and multi-client access.)
//
// Each opened device gets a CALLBACK_FUNCTION; Windows calls it on a
// system thread, one call at a time per device, so each device's receiver
// has a single producer. MIM_DATA carries one short message (status, data1,
// data2 packed in dwParam1); system messages are dropped. Closing stops,
// resets and closes the handle; after midiInClose returns, no callback runs
// for it.
//
// winmm has no stable device id, so ids are the product name, with "#2",
// "#3", ... appended to repeated names in enumeration order.
#if defined(_WIN32)

#include "midi_backend.hpp"

#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#include <mmsystem.h>

#include <map>
#include <mutex>

namespace dstudio {
namespace {

std::string utf8(const wchar_t* w) {
    const int n = WideCharToMultiByte(CP_UTF8, 0, w, -1, nullptr, 0, nullptr, nullptr);
    if (n <= 1) return {};
    std::string s(static_cast<std::size_t>(n - 1), '\0');
    WideCharToMultiByte(CP_UTF8, 0, w, -1, s.data(), n, nullptr, nullptr);
    return s;
}

struct Device {
    HMIDIIN handle = nullptr;
    MidiBackend::Receive rx = nullptr;
    void* ctx = nullptr;
};

void CALLBACK on_midi(HMIDIIN, UINT msg, DWORD_PTR instance, DWORD_PTR param1, DWORD_PTR) {
    if (msg != MIM_DATA) return;
    const auto* d = reinterpret_cast<const Device*>(instance);
    const auto packed = static_cast<uint32_t>(param1);
    const uint8_t bytes[3] = {static_cast<uint8_t>(packed & 0xFF), static_cast<uint8_t>((packed >> 8) & 0x7F),
                              static_cast<uint8_t>((packed >> 16) & 0x7F)};
    if (bytes[0] < 0x80 || bytes[0] >= 0xF0) return;
    d->rx(d->ctx, bytes, MidiByteParser::needed(bytes[0]) + 1u);
}

class WinMidi final : public MidiBackend {
public:
    ~WinMidi() override {
        std::lock_guard lock(mutex_);
        for (auto& [id, d] : devices_) shut(*d);
        devices_.clear();
    }

    void list(std::vector<MidiPortInfo>& out) override {
        out.clear();
        std::vector<UINT> index;
        enumerate(out, index);
    }

    bool open(const std::string& id, Receive rx, void* ctx, std::string& err) override {
        std::vector<MidiPortInfo> ports;
        std::vector<UINT> index;
        enumerate(ports, index);
        for (std::size_t i = 0; i < ports.size(); ++i) {
            if (ports[i].id != id) continue;
            std::lock_guard lock(mutex_);
            auto d = std::make_unique<Device>();
            d->rx = rx;
            d->ctx = ctx;
            MMRESULT r = midiInOpen(&d->handle, index[i], reinterpret_cast<DWORD_PTR>(&on_midi),
                                    reinterpret_cast<DWORD_PTR>(d.get()), CALLBACK_FUNCTION);
            if (r != MMSYSERR_NOERROR) {
                err = "midiInOpen failed (" + std::to_string(r) + ")";
                return false;
            }
            r = midiInStart(d->handle);
            if (r != MMSYSERR_NOERROR) {
                midiInClose(d->handle);
                err = "midiInStart failed (" + std::to_string(r) + ")";
                return false;
            }
            devices_[id] = std::move(d);
            return true;
        }
        err = "not present";
        return false;
    }

    void close(const std::string& id) override {
        std::lock_guard lock(mutex_);
        auto it = devices_.find(id);
        if (it == devices_.end()) return;
        shut(*it->second);
        devices_.erase(it);
    }

private:
    static void shut(Device& d) {
        midiInStop(d.handle);
        midiInReset(d.handle);
        midiInClose(d.handle);
    }

    static void enumerate(std::vector<MidiPortInfo>& out, std::vector<UINT>& index) {
        std::map<std::string, int> seen;
        const UINT n = midiInGetNumDevs();
        for (UINT i = 0; i < n; ++i) {
            MIDIINCAPSW caps{};
            if (midiInGetDevCapsW(i, &caps, sizeof caps) != MMSYSERR_NOERROR) continue;
            MidiPortInfo p;
            p.name = utf8(caps.szPname);
            if (p.name.empty()) p.name = "MIDI input " + std::to_string(i + 1);
            const int k = ++seen[p.name];
            p.id = k == 1 ? p.name : p.name + "#" + std::to_string(k);
            if (k > 1) p.name += " " + std::to_string(k);
            out.push_back(std::move(p));
            index.push_back(i);
        }
    }

    std::mutex mutex_;
    std::map<std::string, std::unique_ptr<Device>> devices_; // guarded by mutex_
};

} // namespace

std::unique_ptr<MidiBackend> create_platform_midi_backend() { return std::make_unique<WinMidi>(); }

} // namespace dstudio

#endif
