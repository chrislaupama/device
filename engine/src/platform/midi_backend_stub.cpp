// MIDI backend for platforms without one (and Linux builds without ALSA):
// no ports. See midi_backend.hpp.
#include "midi_backend.hpp"

#if !defined(__APPLE__) && !defined(_WIN32) && !defined(DSTUDIO_HAVE_ALSA)

namespace dstudio {
namespace {

class NoMidi final : public MidiBackend {
public:
    void list(std::vector<MidiPortInfo>& out) override { out.clear(); }
    bool open(const std::string& id, Receive, void*, std::string& err) override {
        err = "no MIDI input on this platform (port " + id + ")";
        return false;
    }
    void close(const std::string&) override {}
};

} // namespace

std::unique_ptr<MidiBackend> create_platform_midi_backend() { return std::make_unique<NoMidi>(); }

} // namespace dstudio

#endif
