// Test-only entry points (not part of the public ABI). Used by the C++ test
// harness, which links the static library directly.
#pragma once

#include "platform/audio_device.hpp"
#include "saine_engine.h"

#include <cstddef>
#include <functional>
#include <memory>

namespace saine::testing {

// Replace the platform device (fake devices drive Engine's device path
// without hardware). Call before the first start/configure.
void set_device_factory(se_engine* e, std::function<std::unique_ptr<AudioDevice>()> factory);
// While stalled, the recording writer thread does not drain the rings
// (stop still drains and finalizes). Applies to the current recording.
void set_record_writer_stalled(se_engine* e, bool stalled);
// Ring size (frames per track) for the next se_record_start; 0 = default (2 s).
void set_record_ring_frames(se_engine* e, std::size_t frames);

} // namespace saine::testing
