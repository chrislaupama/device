// Linux MIDI input through the ALSA sequencer.
//
// Why the sequencer (and not raw MIDI devices): it lists every MIDI source
// the system knows, hardware ports and software clients alike (PipeWire and
// JACK bridge into it), it parses the byte stream into events, and several
// applications can read the same port at once.
//
// One sequencer client ("Device Studio") with one input port. Opening a
// source subscribes that port to the source; closing unsubscribes. One
// reader thread waits on the sequencer's poll descriptors plus a wake pipe
// and turns events into channel messages for the source's receiver, so the
// receiver of each source sees calls from this single thread only.
//
// Ids are "client name:port name", which stay the same when a device is
// unplugged and plugged back in (client numbers do not). Without a
// sequencer (/dev/snd/seq missing, e.g. in containers) there are no ports.
#if defined(__linux__) && defined(DSTUDIO_HAVE_ALSA)

#include "midi_backend.hpp"

#include <alsa/asoundlib.h>

#include <atomic>
#include <cerrno>
#include <cstring>
#include <map>
#include <mutex>
#include <poll.h>
#include <thread>
#include <unistd.h>
#include <vector>

// The ALSA headers' accessor macros expand to C casts.
#pragma GCC diagnostic ignored "-Wold-style-cast"

namespace dstudio {
namespace {

struct Source {
    int client = 0;
    int port = 0;
    MidiBackend::Receive rx = nullptr;
    void* ctx = nullptr;
};

class AlsaMidi final : public MidiBackend {
public:
    AlsaMidi() {
        if (snd_seq_open(&seq_, "default", SND_SEQ_OPEN_INPUT, SND_SEQ_NONBLOCK) < 0) {
            seq_ = nullptr;
            return;
        }
        snd_seq_set_client_name(seq_, "Device Studio");
        port_ = snd_seq_create_simple_port(seq_, "MIDI in", SND_SEQ_PORT_CAP_WRITE | SND_SEQ_PORT_CAP_SUBS_WRITE,
                                           SND_SEQ_PORT_TYPE_MIDI_GENERIC | SND_SEQ_PORT_TYPE_APPLICATION);
        if (port_ < 0 || pipe(wake_) != 0) {
            snd_seq_close(seq_);
            seq_ = nullptr;
            return;
        }
        self_ = snd_seq_client_id(seq_);
        thread_ = std::thread([this] { run(); });
    }

    ~AlsaMidi() override {
        if (seq_ == nullptr) return;
        stop_.store(true, std::memory_order_release);
        const char b = 1;
        [[maybe_unused]] const ssize_t w = write(wake_[1], &b, 1);
        thread_.join();
        ::close(wake_[0]);
        ::close(wake_[1]);
        snd_seq_close(seq_);
    }

    void list(std::vector<MidiPortInfo>& out) override {
        out.clear();
        if (seq_ == nullptr) return;
        std::lock_guard lock(mutex_);
        snd_seq_client_info_t* ci = nullptr;
        snd_seq_port_info_t* pi = nullptr;
        snd_seq_client_info_alloca(&ci);
        snd_seq_port_info_alloca(&pi);
        snd_seq_client_info_set_client(ci, -1);
        while (snd_seq_query_next_client(seq_, ci) >= 0) {
            const int client = snd_seq_client_info_get_client(ci);
            if (client == self_ || client == SND_SEQ_CLIENT_SYSTEM) continue;
            const std::string cname = snd_seq_client_info_get_name(ci);
            snd_seq_port_info_set_client(pi, client);
            snd_seq_port_info_set_port(pi, -1);
            while (snd_seq_query_next_port(seq_, pi) >= 0) {
                const unsigned caps = snd_seq_port_info_get_capability(pi);
                const unsigned want = SND_SEQ_PORT_CAP_READ | SND_SEQ_PORT_CAP_SUBS_READ;
                if ((caps & want) != want || (caps & SND_SEQ_PORT_CAP_NO_EXPORT) != 0) continue;
                if ((snd_seq_port_info_get_type(pi) & SND_SEQ_PORT_TYPE_MIDI_GENERIC) == 0) continue;
                const std::string pname = snd_seq_port_info_get_name(pi);
                MidiPortInfo p;
                p.id = cname + ":" + pname;
                p.name = pname == cname || pname.rfind(cname, 0) == 0 ? pname : cname + " " + pname;
                out.push_back(std::move(p));
            }
        }
    }

    bool open(const std::string& id, Receive rx, void* ctx, std::string& err) override {
        if (seq_ == nullptr) {
            err = "no ALSA sequencer";
            return false;
        }
        int client = -1, port = -1;
        if (!find(id, client, port)) {
            err = "not present";
            return false;
        }
        std::lock_guard lock(mutex_);
        const int rc = snd_seq_connect_from(seq_, port_, client, port);
        if (rc < 0) {
            err = snd_strerror(rc);
            return false;
        }
        sources_[id] = Source{client, port, rx, ctx};
        return true;
    }

    void close(const std::string& id) override {
        if (seq_ == nullptr) return;
        std::lock_guard lock(mutex_); // the reader holds it while delivering
        auto it = sources_.find(id);
        if (it == sources_.end()) return;
        snd_seq_disconnect_from(seq_, port_, it->second.client, it->second.port);
        sources_.erase(it);
    }

private:
    bool find(const std::string& id, int& client, int& port) {
        std::lock_guard lock(mutex_);
        snd_seq_client_info_t* ci = nullptr;
        snd_seq_port_info_t* pi = nullptr;
        snd_seq_client_info_alloca(&ci);
        snd_seq_port_info_alloca(&pi);
        snd_seq_client_info_set_client(ci, -1);
        while (snd_seq_query_next_client(seq_, ci) >= 0) {
            const int c = snd_seq_client_info_get_client(ci);
            const std::string cname = snd_seq_client_info_get_name(ci);
            if (id.rfind(cname + ":", 0) != 0) continue;
            snd_seq_port_info_set_client(pi, c);
            snd_seq_port_info_set_port(pi, -1);
            while (snd_seq_query_next_port(seq_, pi) >= 0)
                if (cname + ":" + snd_seq_port_info_get_name(pi) == id) {
                    client = c;
                    port = snd_seq_port_info_get_port(pi);
                    return true;
                }
        }
        return false;
    }

    void deliver(const snd_seq_event_t& ev) {
        uint8_t msg[3] = {0, 0, 0};
        uint32_t len = 3;
        switch (ev.type) {
        case SND_SEQ_EVENT_NOTEON:
            msg[0] = static_cast<uint8_t>(0x90 | (ev.data.note.channel & 0x0F));
            msg[1] = ev.data.note.note;
            msg[2] = ev.data.note.velocity;
            break;
        case SND_SEQ_EVENT_NOTEOFF:
            msg[0] = static_cast<uint8_t>(0x80 | (ev.data.note.channel & 0x0F));
            msg[1] = ev.data.note.note;
            msg[2] = ev.data.note.velocity;
            break;
        case SND_SEQ_EVENT_KEYPRESS:
            msg[0] = static_cast<uint8_t>(0xA0 | (ev.data.note.channel & 0x0F));
            msg[1] = ev.data.note.note;
            msg[2] = ev.data.note.velocity;
            break;
        case SND_SEQ_EVENT_CONTROLLER:
            msg[0] = static_cast<uint8_t>(0xB0 | (ev.data.control.channel & 0x0F));
            msg[1] = static_cast<uint8_t>(ev.data.control.param & 0x7F);
            msg[2] = static_cast<uint8_t>(ev.data.control.value & 0x7F);
            break;
        case SND_SEQ_EVENT_CHANPRESS:
            msg[0] = static_cast<uint8_t>(0xD0 | (ev.data.control.channel & 0x0F));
            msg[1] = static_cast<uint8_t>(ev.data.control.value & 0x7F);
            len = 2;
            break;
        case SND_SEQ_EVENT_PITCHBEND: {
            const int v = ev.data.control.value + 8192; // -8192..8191 -> 0..16383
            msg[0] = static_cast<uint8_t>(0xE0 | (ev.data.control.channel & 0x0F));
            msg[1] = static_cast<uint8_t>(v & 0x7F);
            msg[2] = static_cast<uint8_t>((v >> 7) & 0x7F);
            break;
        }
        default:
            return;
        }
        for (const auto& [id, s] : sources_)
            if (s.client == ev.source.client && s.port == ev.source.port) s.rx(s.ctx, msg, len);
    }

    void run() {
        const int n = snd_seq_poll_descriptors_count(seq_, POLLIN);
        std::vector<pollfd> fds(static_cast<std::size_t>(n) + 1);
        snd_seq_poll_descriptors(seq_, fds.data(), static_cast<unsigned>(n), POLLIN);
        fds[static_cast<std::size_t>(n)] = pollfd{wake_[0], POLLIN, 0};
        while (!stop_.load(std::memory_order_acquire)) {
            if (poll(fds.data(), fds.size(), -1) < 0 && errno != EINTR) break;
            if (stop_.load(std::memory_order_acquire)) break;
            std::lock_guard lock(mutex_);
            snd_seq_event_t* ev = nullptr;
            while (snd_seq_event_input(seq_, &ev) >= 0 && ev != nullptr) deliver(*ev);
        }
    }

    snd_seq_t* seq_ = nullptr;
    int port_ = -1;
    int self_ = -1;
    int wake_[2] = {-1, -1};
    std::mutex mutex_;
    std::map<std::string, Source> sources_; // guarded by mutex_
    std::thread thread_;
    std::atomic<bool> stop_{false};
};

} // namespace

std::unique_ptr<MidiBackend> create_platform_midi_backend() { return std::make_unique<AlsaMidi>(); }

} // namespace dstudio

#endif
