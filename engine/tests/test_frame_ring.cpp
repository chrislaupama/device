// FrameRing: the duplex input FIFO of the WASAPI and PipeWire backends
// (platform/frame_ring.hpp). Runs on every OS, without audio hardware.
#include "test_harness.hpp"
#include "test_util.hpp"

#include "platform/frame_ring.hpp"

#include <algorithm>
#include <atomic>
#include <thread>
#include <vector>

using saine::FrameRing;

namespace {

// Interleaved stereo frames whose left value is the running frame index
// and right value its negation.
std::vector<float> frames_from(uint64_t first, uint32_t n) {
    std::vector<float> v(static_cast<std::size_t>(n) * 2);
    for (uint32_t i = 0; i < n; ++i) {
        v[2 * i] = static_cast<float>(first + i);
        v[2 * i + 1] = -static_cast<float>(first + i);
    }
    return v;
}

struct Planar {
    explicit Planar(uint32_t ch, uint32_t n) : data(ch, std::vector<float>(n, 99.0f)) {
        for (auto& c : data) ptrs.push_back(c.data());
    }
    std::vector<std::vector<float>> data;
    std::vector<float*> ptrs;
};

} // namespace

TEST_CASE(frame_ring_primes_then_passes_frames_in_order) {
    FrameRing ring;
    ring.reset(2, 1024, 64, 128);
    Planar out(2, 32);
    // Below target + one cycle: silence, nothing consumed.
    ring.write(frames_from(0, 64).data(), 64);
    CHECK_EQ(ring.read(out.ptrs.data(), 2, 32), 0u);
    CHECK(out.data[0][0] == 0.0f && out.data[1][31] == 0.0f);
    CHECK_EQ(ring.fill(), 64u);
    // Primed: reads continue exactly from where `target` frames remain.
    ring.write(frames_from(64, 32).data(), 32);
    CHECK_EQ(ring.read(out.ptrs.data(), 2, 32), 32u);
    CHECK_EQ(ring.fill(), 64u);
    for (uint32_t i = 0; i < 32; ++i) {
        CHECK_EQ(out.data[0][i], static_cast<float>(i));
        CHECK_EQ(out.data[1][i], -static_cast<float>(i));
    }
    // Steady state: one cycle in, one cycle out, contiguous.
    for (uint64_t k = 0; k < 20; ++k) {
        ring.write(frames_from(96 + 32 * k, 32).data(), 32);
        REQUIRE(ring.read(out.ptrs.data(), 2, 32) == 32u);
        CHECK_EQ(out.data[0][0], static_cast<float>(32 + 32 * k));
        CHECK_EQ(out.data[0][31], static_cast<float>(63 + 32 * k));
    }
    CHECK_EQ(ring.underruns(), uint64_t{0});
    CHECK_EQ(ring.dropped(), uint64_t{0});
}

TEST_CASE(frame_ring_underrun_zero_fills_and_reprimes) {
    FrameRing ring;
    ring.reset(2, 1024, 16, 64);
    Planar out(2, 32);
    ring.write(frames_from(0, 48).data(), 48);
    REQUIRE(ring.read(out.ptrs.data(), 2, 32) == 32u); // primed, 16 left
    // Input stalls: the 16 buffered frames come out, then zeros.
    CHECK_EQ(ring.read(out.ptrs.data(), 2, 32), 16u);
    CHECK_EQ(out.data[0][15], 47.0f);
    CHECK(out.data[0][16] == 0.0f && out.data[0][31] == 0.0f);
    CHECK_EQ(ring.underruns(), uint64_t{1});
    // Re-priming: silence until target + cycle frames are buffered again.
    ring.write(frames_from(48, 32).data(), 32);
    CHECK_EQ(ring.read(out.ptrs.data(), 2, 32), 0u);
    ring.write(frames_from(80, 16).data(), 16);
    CHECK_EQ(ring.read(out.ptrs.data(), 2, 32), 32u);
    CHECK_EQ(out.data[0][0], 48.0f);
}

TEST_CASE(frame_ring_fast_input_clock_is_trimmed_back_to_target) {
    FrameRing ring;
    ring.reset(2, 4096, 64, 96);
    Planar out(2, 64);
    uint64_t written = 0;
    const auto put = [&](uint32_t n) {
        ring.write(frames_from(written, n).data(), n);
        written += n;
    };
    put(128);
    REQUIRE(ring.read(out.ptrs.data(), 2, 64) == 64u);
    // The input delivers 65 frames per 64-frame output cycle (a grossly fast
    // clock): the fill grows by one per cycle until it passes target+slack,
    // then the excess is skipped.
    uint32_t max_fill = 0;
    for (int k = 0; k < 300; ++k) {
        put(65);
        REQUIRE(ring.read(out.ptrs.data(), 2, 64) == 64u);
        max_fill = std::max(max_fill, ring.fill());
        // Within one cycle the frames stay contiguous.
        CHECK_EQ(out.data[0][63] - out.data[0][0], 63.0f);
    }
    CHECK(max_fill <= 64u + 96u);
    CHECK(ring.dropped() > 0);
    CHECK_EQ(ring.underruns(), uint64_t{0});
}

TEST_CASE(frame_ring_overflow_and_channel_mismatch) {
    FrameRing ring;
    ring.reset(2, 64, 8, 8);
    CHECK_EQ(ring.write(frames_from(0, 100).data(), 100), 64u);
    CHECK_EQ(ring.overflows(), uint64_t{36});
    // Reader wants 3 channels: the extra one is silent. Writes of silence.
    FrameRing mono;
    mono.reset(1, 64, 0, 64);
    mono.write(nullptr, 8);
    Planar out(3, 8);
    CHECK_EQ(mono.read(out.ptrs.data(), 3, 8), 8u);
    for (int c = 0; c < 3; ++c) CHECK(out.data[static_cast<std::size_t>(c)][7] == 0.0f);
}

TEST_CASE(frame_ring_threads_and_rt_safety) {
    // Producer and consumer on different threads, as with two device
    // clients; read/write never allocate.
    FrameRing ring;
    ring.reset(2, 8192, 256, 512);
    std::atomic<bool> done{false};
    const long allocs_before = testutil::rt_alloc_count();
    const std::vector<float> block = frames_from(0, 128);
    std::thread producer([&] {
        testutil::RtAllocGuard guard;
        for (int k = 0; k < 2000; ++k) {
            ring.write(block.data(), 128);
            if ((k & 7) == 0) std::this_thread::yield();
        }
        done = true;
    });
    Planar out(2, 128);
    uint64_t got = 0;
    {
        testutil::RtAllocGuard guard;
        while (!done.load() || ring.fill() >= 128 + ring.target()) got += ring.read(out.ptrs.data(), 2, 128);
    }
    producer.join();
    CHECK(got > 0);
    CHECK_EQ(testutil::rt_alloc_count(), allocs_before);
}

TEST_CASE(frame_ring_chunked_cycle_is_contiguous_and_not_trimmed) {
    // A 96-frame device cycle read as 3 chunks of 32 (cycle_remaining 64,
    // 32, 0): primes on the whole cycle, keeps the chunks contiguous and the
    // frames still to be read this cycle are not dropped as drift excess.
    FrameRing ring;
    ring.reset(2, 1024, 32, 16);
    Planar out(2, 32);
    uint64_t written = 0, expect = 0;
    bool primed = false;
    for (int cycle = 0; cycle < 10; ++cycle) {
        ring.write(frames_from(written, 96).data(), 96);
        written += 96;
        for (uint32_t k = 0; k < 3; ++k) {
            const uint32_t got = ring.read(out.ptrs.data(), 2, 32, 64 - 32 * k);
            if (got == 0 && !primed) continue;
            primed = true;
            REQUIRE(got == 32u);
            if (expect == 0) expect = static_cast<uint64_t>(out.data[0][0]);
            for (uint32_t i = 0; i < 32; ++i) CHECK_EQ(out.data[0][i], static_cast<float>(expect + i));
            expect += 32;
        }
    }
    CHECK(primed);
    CHECK_EQ(ring.dropped(), 0u);
    CHECK_EQ(ring.underruns(), 0u);
}
