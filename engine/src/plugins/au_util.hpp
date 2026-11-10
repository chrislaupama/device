// Small Audio Unit / CoreFoundation helpers shared by hosting and scanning.
#pragma once

#if defined(__APPLE__)
#include <AudioToolbox/AudioToolbox.h>

#include <string>

namespace dstudio {
// "aufx lpas appl" <-> AudioComponentDescription.
bool parse_au_id(const std::string& id, AudioComponentDescription& d);
std::string au_id_string(const AudioComponentDescription& d);
std::string four_cc(uint32_t c);
std::string cf_to_string(CFStringRef s);
} // namespace dstudio
#endif
