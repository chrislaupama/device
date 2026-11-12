// Linux MIDI input through the ALSA sequencer.
//
// Why the sequencer (and not raw MIDI devices): it lists every MIDI source
// the system knows, hardware ports and software clients alike (PipeWire and
// JACK bridge into it), it parses the byte stream into events, and several
// applications can read the same port at once.
//
// One sequencer client ("Device") with one input port. Opening a
// source subscribes that port to the source; closing unsubscribes. One
// reader thread waits on the sequencer's poll descriptors plus a wake pipe
// and turns events into channel messages for the source's receiver, so the
// receiver of each source sees calls from this single thread only. Listing
// and looking up ports use a second sequencer handle, so a slow scan never
// holds up the reader.
//
// Ids are "client name:port name", which stay the same when a device is
// unplugged and plugged back in (client numbers do not). Identical devices
// share that name, so the second one listed gets "#2" appended, the third
// "#3", and so on (as on Windows). Without a sequencer (/dev/snd/seq
// missing, e.g. in containers) there are no ports.
//
// libasound is loaded with dlopen on first use rather than linked: programs
// that never touch MIDI don't pay for loading it, and the engine still runs
// (with no MIDI inputs) on a system without it. The ALSA headers are only
// needed to build.
#if defined(__linux__) && defined(DSTUDIO_HAVE_ALSA)

#include "midi_backend.hpp"

#include <alsa/asoundlib.h>

#include <atomic>
#include <cerrno>
#include <dlfcn.h>
#include <cstring>
#include <map>
#include <mutex>
#include <poll.h>
#include <string>
#include <thread>
#include <unistd.h>
#include <vector>

// The ALSA headers' accessor macros expand to C casts.
#pragma GCC diagnostic ignored "-Wold-style-cast"

namespace dstudio {
namespace {

// The libasound functions this backend uses, resolved from libasound.so.2.
struct Alsa {
#define DSTUDIO_ALSA_FNS(X)                                                                                         \
    X(snd_seq_open) X(snd_seq_close) X(snd_seq_set_client_name) X(snd_seq_create_simple_port) X(snd_seq_client_id) \
    X(snd_seq_client_info_sizeof) X(snd_seq_client_info_set_client) X(snd_seq_client_info_get_client)            \
    X(snd_seq_client_info_get_name) X(snd_seq_query_next_client) X(snd_seq_port_info_sizeof)                       \
    X(snd_seq_port_info_set_client) X(snd_seq_port_info_set_port) X(snd_seq_port_info_get_port)                   \
    X(snd_seq_port_info_get_name) X(snd_seq_port_info_get_capability) X(snd_seq_port_info_get_type)               \
    X(snd_seq_query_next_port) X(snd_seq_connect_from) X(snd_seq_disconnect_from) X(snd_seq_event_input)         \
    X(snd_seq_poll_descriptors_count) X(snd_seq_poll_descriptors) X(snd_strerror)
#define DSTUDIO_ALSA_FIELD(name) decltype(&::name) name = nullptr;
    DSTUDIO_ALSA_FNS(DSTUDIO_ALSA_FIELD)
#undef DSTUDIO_ALSA_FIELD
};

// Null when libasound (or one of its functions) is missing.
const Alsa* alsa() {
    static const Alsa* const loaded = []() -> const Alsa* {
        void* lib = dlopen("libasound.so.2", RTLD_NOW | RTLD_LOCAL);
        if (lib == nullptr) return nullptr;
        static Alsa a;
        bool ok = true;
#define DSTUDIO_ALSA_LOAD(name)                                                  \
    a.name = reinterpret_cast<decltype(a.name)>(dlsym(lib, #name));               \
    ok = ok && a.name != nullptr;
        DSTUDIO_ALSA_FNS(DSTUDIO_ALSA_LOAD)
#undef DSTUDIO_ALSA_LOAD
        return ok ? &a : nullptr; // the library stays loaded for the process
    }();
    return loaded;
}

// Zeroed, correctly sized info records (what the snd_*_alloca macros make).
struct SeqInfos {
    explicit SeqInfos(const Alsa& a)
        : cbuf(a.snd_seq_client_info_sizeof(), 0), pbuf(a.snd_seq_port_info_sizeof(), 0),
          ci(reinterpret_cast<snd_seq_client_info_t*>(cbuf.data())), pi(reinterpret_cast<snd_seq_port_info_t*>(pbuf.data())) {}
    std::vector<unsigned char> cbuf, pbuf;
    snd_seq_client_info_t* ci;
    snd_seq_port_info_t* pi;
};

struct Source {
    int client = 0;
    int port = 0;
    MidiBackend::Receive rx = nullptr;
    void* ctx = nullptr;
};

class AlsaMidi final : public MidiBackend {
public:
    AlsaMidi() : a_(alsa()) {
        if (a_ == nullptr) return; // no libasound: no MIDI inputs
        if (a_->snd_seq_open(&seq_, "default", SND_SEQ_OPEN_INPUT, SND_SEQ_NONBLOCK) < 0) {
            seq_ = nullptr;
            return;
        }
        if (a_->snd_seq_open(&query_, "default", SND_SEQ_OPEN_OUTPUT, 0) < 0) query_ = nullptr;
        a_->snd_seq_set_client_name(seq_, "Device");
        port_ = a_->snd_seq_create_simple_port(seq_, "MIDI in", SND_SEQ_PORT_CAP_WRITE | SND_SEQ_PORT_CAP_SUBS_WRITE,
                                               SND_SEQ_PORT_TYPE_MIDI_GENERIC | SND_SEQ_PORT_TYPE_APPLICATION);
        if (query_ == nullptr || port_ < 0 || pipe(wake_) != 0) {
            if (query_ != nullptr) a_->snd_seq_close(query_);
            a_->snd_seq_close(seq_);
            seq_ = query_ = nullptr;
            return;
        }
        a_->snd_seq_set_client_name(query_, "Device (ports)");
        self_ = a_->snd_seq_client_id(seq_);
        query_self_ = a_->snd_seq_client_id(query_);
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
        a_->snd_seq_close(query_);
        a_->snd_seq_close(seq_);
    }

    void list(std::vector<MidiPortInfo>& out) override {
        out.clear();
        if (seq_ == nullptr) return;
        enumerate([&](std::string id, std::string name, int, int) {
            out.push_back(MidiPortInfo{std::move(id), std::move(name)});
            return false;
        });
    }

    bool open(const std::string& id, Receive rx, void* ctx, std::string& err) override {
        if (seq_ == nullptr) {
            err = "no ALSA sequencer";
            return false;
        }
        int client = -1, port = -1;
        enumerate([&](const std::string& pid, const std::string&, int c, int p) {
            if (pid != id) return false;
            client = c;
            port = p;
            return true;
        });
        if (client < 0) {
            err = "not present";
            return false;
        }
        // Registered before connecting, so the first event already finds it.
        {
            std::lock_guard lock(sources_mutex_);
            sources_[id] = Source{client, port, rx, ctx};
        }
        int rc;
        {
            std::lock_guard lock(seq_mutex_);
            rc = a_->snd_seq_connect_from(seq_, port_, client, port);
        }
        if (rc < 0) {
            std::lock_guard lock(sources_mutex_);
            sources_.erase(id);
            err = a_->snd_strerror(rc);
            return false;
        }
        return true;
    }

    void close(const std::string& id) override {
        if (seq_ == nullptr) return;
        Source src;
        {
            std::lock_guard lock(sources_mutex_); // the reader holds it while delivering
            auto it = sources_.find(id);
            if (it == sources_.end()) return;
            src = it->second;
            sources_.erase(it);
        }
        std::lock_guard lock(seq_mutex_);
        a_->snd_seq_disconnect_from(seq_, port_, src.client, src.port);
    }

private:
    // Calls f(id, name, client, port) for every listable input port, in
    // sequencer order, until it returns true. Ids are made unique here
    // ("#2", "#3" for identical names), so list() and open() agree.
    template <class F>
    void enumerate(F&& f) {
        std::lock_guard lock(query_mutex_);
        SeqInfos infos(*a_);
        snd_seq_client_info_t* const ci = infos.ci;
        snd_seq_port_info_t* const pi = infos.pi;
        std::map<std::string, int> seen;
        a_->snd_seq_client_info_set_client(ci, -1);
        while (a_->snd_seq_query_next_client(query_, ci) >= 0) {
            const int client = a_->snd_seq_client_info_get_client(ci);
            if (client == self_ || client == query_self_ || client == SND_SEQ_CLIENT_SYSTEM) continue;
            const std::string cname = a_->snd_seq_client_info_get_name(ci);
            a_->snd_seq_port_info_set_client(pi, client);
            a_->snd_seq_port_info_set_port(pi, -1);
            while (a_->snd_seq_query_next_port(query_, pi) >= 0) {
                const unsigned caps = a_->snd_seq_port_info_get_capability(pi);
                const unsigned want = SND_SEQ_PORT_CAP_READ | SND_SEQ_PORT_CAP_SUBS_READ;
                if ((caps & want) != want || (caps & SND_SEQ_PORT_CAP_NO_EXPORT) != 0) continue;
                if ((a_->snd_seq_port_info_get_type(pi) & SND_SEQ_PORT_TYPE_MIDI_GENERIC) == 0) continue;
                const std::string pname = a_->snd_seq_port_info_get_name(pi);
                std::string id = cname + ":" + pname;
                std::string name = pname == cname || pname.rfind(cname, 0) == 0 ? pname : cname + " " + pname;
                if (const int k = ++seen[id]; k > 1) {
                    id += "#" + std::to_string(k);
                    name += " " + std::to_string(k);
                }
                if (f(std::move(id), std::move(name), client, a_->snd_seq_port_info_get_port(pi))) return;
            }
        }
    }

    // Requires sources_mutex_.
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
        const int n = a_->snd_seq_poll_descriptors_count(seq_, POLLIN);
        std::vector<pollfd> fds(static_cast<std::size_t>(n) + 1);
        a_->snd_seq_poll_descriptors(seq_, fds.data(), static_cast<unsigned>(n), POLLIN);
        fds[static_cast<std::size_t>(n)] = pollfd{wake_[0], POLLIN, 0};
        while (!stop_.load(std::memory_order_acquire)) {
            if (poll(fds.data(), fds.size(), -1) < 0 && errno != EINTR) break;
            if (stop_.load(std::memory_order_acquire)) break;
            for (;;) {
                snd_seq_event_t ev;
                {
                    std::lock_guard lock(seq_mutex_); // only for the read: connects/disconnects use seq_ too
                    snd_seq_event_t* p = nullptr;
                    if (a_->snd_seq_event_input(seq_, &p) < 0 || p == nullptr) break;
                    ev = *p; // channel events carry their data inline
                }
                std::lock_guard lock(sources_mutex_);
                deliver(ev);
            }
        }
    }

    const Alsa* const a_; // null without libasound (then seq_ stays null)
    snd_seq_t* seq_ = nullptr;   // input port and subscriptions; read by the reader thread
    snd_seq_t* query_ = nullptr; // port enumeration (list, open)
    int port_ = -1;
    int self_ = -1;
    int query_self_ = -1;
    int wake_[2] = {-1, -1};
    std::mutex seq_mutex_;     // seq_ calls
    std::mutex query_mutex_;   // query_ calls
    std::mutex sources_mutex_; // sources_; held while a receiver runs
    std::map<std::string, Source> sources_;
    std::thread thread_;
    std::atomic<bool> stop_{false};
};

} // namespace

std::unique_ptr<MidiBackend> create_platform_midi_backend() { return std::make_unique<AlsaMidi>(); }

} // namespace dstudio

#endif
