// macOS MIDI input through Core MIDI.
//
// One MIDI client and one input port created with the MIDI 1.0 protocol
// (MIDIInputPortCreateWithProtocol): Core MIDI delivers Universal MIDI
// Packets, and the MIDI 1.0 channel voice messages (message type 2) among
// them carry exactly the status and data bytes we pass on. Opening a source
// connects it to the port with its slot as the connection's refCon;
// closing disconnects it. Core MIDI runs the receive block on its own
// high-priority thread and serialises calls for a port, so each source's
// receiver has a single producer. MIDIPortDisconnectSource is synchronous:
// after it returns, no more packets arrive for that source.
//
// Ids are the source's kMIDIPropertyUniqueID (persistent across launches and
// re-plugging); names are kMIDIPropertyDisplayName ("Device Port").
// The client is created lazily, the first time the engine lists or opens
// MIDI, so engines that never use MIDI never start the MIDI server.
#if defined(__APPLE__)

#include "midi_backend.hpp"

#include <CoreFoundation/CoreFoundation.h>
#include <CoreMIDI/CoreMIDI.h>

#include <map>
#include <mutex>

namespace dstudio {
namespace {

std::string cf_string(CFStringRef s) {
    if (s == nullptr) return {};
    char buf[512];
    if (CFStringGetCString(s, buf, sizeof buf, kCFStringEncodingUTF8)) return buf;
    return {};
}

struct Connection {
    MIDIEndpointRef source = 0;
    MidiBackend::Receive rx = nullptr;
    void* ctx = nullptr;
};

class CoreMidi final : public MidiBackend {
public:
    CoreMidi() {
        if (MIDIClientCreate(CFSTR("Device"), nullptr, nullptr, &client_) != noErr) {
            client_ = 0;
            return;
        }
        // The block receives one MIDIEventList per call; refCon is the
        // Connection of the source the packets came from.
        const OSStatus st = MIDIInputPortCreateWithProtocol(
            client_, CFSTR("MIDI in"), kMIDIProtocol_1_0, &port_,
            ^(const MIDIEventList* list, void* ref) {
                const auto* c = static_cast<const Connection*>(ref);
                if (c == nullptr || list == nullptr) return;
                const MIDIEventPacket* p = &list->packet[0];
                for (UInt32 i = 0; i < list->numPackets; ++i) {
                    for (UInt32 w = 0; w < p->wordCount; ++w) {
                        const UInt32 word = p->words[w];
                        const UInt32 type = word >> 28;
                        if (type != 0x2) { // not a MIDI 1.0 channel voice message: skip its words
                            const UInt32 size = type <= 0x2 || type == 0x6 || type == 0x7 ? 1u
                                                : type == 0x3 || type == 0x4 || type == 0x8 || type == 0x9 || type == 0xA ? 2u
                                                : type == 0xB || type == 0xC ? 3u
                                                : 4u;
                            w += size - 1;
                            continue;
                        }
                        const uint8_t msg[3] = {static_cast<uint8_t>((word >> 16) & 0xFF),
                                                static_cast<uint8_t>((word >> 8) & 0x7F),
                                                static_cast<uint8_t>(word & 0x7F)};
                        const uint8_t kind = msg[0] & 0xF0;
                        c->rx(c->ctx, msg, kind == 0xC0 || kind == 0xD0 ? 2u : 3u);
                    }
                    p = MIDIEventPacketNext(p);
                }
            });
        if (st != noErr) port_ = 0;
    }

    ~CoreMidi() override {
        std::lock_guard lock(mutex_);
        for (auto& [id, c] : connections_) MIDIPortDisconnectSource(port_, c->source);
        connections_.clear();
        if (port_ != 0) MIDIPortDispose(port_);
        if (client_ != 0) MIDIClientDispose(client_);
    }

    void list(std::vector<MidiPortInfo>& out) override {
        out.clear();
        if (port_ == 0) return;
        const ItemCount n = MIDIGetNumberOfSources();
        for (ItemCount i = 0; i < n; ++i) {
            const MIDIEndpointRef src = MIDIGetSource(i);
            if (src == 0) continue;
            MidiPortInfo p;
            if (!describe(src, p)) continue;
            out.push_back(std::move(p));
        }
    }

    bool open(const std::string& id, Receive rx, void* ctx, std::string& err) override {
        if (port_ == 0) {
            err = "Core MIDI is not available";
            return false;
        }
        const MIDIEndpointRef src = find(id);
        if (src == 0) {
            err = "not present";
            return false;
        }
        std::lock_guard lock(mutex_);
        auto conn = std::make_unique<Connection>(Connection{src, rx, ctx});
        const OSStatus st = MIDIPortConnectSource(port_, src, conn.get());
        if (st != noErr) {
            err = "MIDIPortConnectSource failed (" + std::to_string(st) + ")";
            return false;
        }
        connections_[id] = std::move(conn);
        return true;
    }

    void close(const std::string& id) override {
        std::lock_guard lock(mutex_);
        auto it = connections_.find(id);
        if (it == connections_.end()) return;
        MIDIPortDisconnectSource(port_, it->second->source);
        connections_.erase(it);
    }

private:
    static bool describe(MIDIEndpointRef src, MidiPortInfo& p) {
        SInt32 uid = 0;
        if (MIDIObjectGetIntegerProperty(src, kMIDIPropertyUniqueID, &uid) != noErr) return false;
        p.id = "coremidi:" + std::to_string(uid);
        CFStringRef name = nullptr;
        if (MIDIObjectGetStringProperty(src, kMIDIPropertyDisplayName, &name) == noErr && name != nullptr) {
            p.name = cf_string(name);
            CFRelease(name);
        }
        if (p.name.empty()) p.name = "MIDI input " + std::to_string(uid);
        return true;
    }

    static MIDIEndpointRef find(const std::string& id) {
        const ItemCount n = MIDIGetNumberOfSources();
        for (ItemCount i = 0; i < n; ++i) {
            const MIDIEndpointRef src = MIDIGetSource(i);
            MidiPortInfo p;
            if (src != 0 && describe(src, p) && p.id == id) return src;
        }
        return 0;
    }

    MIDIClientRef client_ = 0;
    MIDIPortRef port_ = 0;
    std::mutex mutex_;
    std::map<std::string, std::unique_ptr<Connection>> connections_; // guarded by mutex_
};

} // namespace

std::unique_ptr<MidiBackend> create_platform_midi_backend() { return std::make_unique<CoreMidi>(); }

} // namespace dstudio

#endif
