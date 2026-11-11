// FrameRing (platform/frame_ring.hpp) cost per device cycle: one capture
// write plus one render read of a stereo cycle, as the WASAPI and PipeWire
// duplex paths do on their audio threads.
//
//   dstudio_frame_ring_bench [cycles=2000000]
#include "platform/frame_ring.hpp"

#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <vector>

namespace {

// Average ns per write+read cycle of `frames` stereo frames.
double run(uint32_t frames, uint32_t cycles, uint32_t tolerance) {
    dstudio::FrameRing ring;
    ring.reset(2, 8 * 8192, 2 * frames, 4 * frames, tolerance);
    std::vector<float> in(static_cast<std::size_t>(frames) * 2, 0.25f);
    std::vector<float> l(frames), r(frames);
    float* out[2] = {l.data(), r.data()};
    double sink = 0.0;
    const auto t0 = std::chrono::steady_clock::now();
    for (uint32_t k = 0; k < cycles; ++k) {
        ring.write(in.data(), frames);
        ring.read(out, 2, frames);
        sink += static_cast<double>(l[k % frames]);
    }
    const auto t1 = std::chrono::steady_clock::now();
    if (sink < 0.0) std::printf("%f\n", sink); // keep the loop
    return std::chrono::duration<double, std::nano>(t1 - t0).count() / cycles;
}

} // namespace

int main(int argc, char** argv) {
    const uint32_t cycles = argc > 1 ? static_cast<uint32_t>(std::atoi(argv[1])) : 2000000;
    std::printf("dstudio_frame_ring_bench: %u cycles, stereo, ns per write + read (best of 5)\n", cycles);
    std::printf("%8s %16s %16s\n", "frames", "re-sync off", "re-sync on");
    for (uint32_t frames : {64u, 256u, 1024u}) {
        double off = 1e30, on = 1e30;
        for (int rep = 0; rep < 5; ++rep) {
            off = std::min(off, run(frames, cycles / (frames / 64), 0));
            on = std::min(on, run(frames, cycles / (frames / 64), frames / 2));
        }
        std::printf("%8u %16.1f %16.1f\n", frames, off, on);
    }
    return 0;
}
