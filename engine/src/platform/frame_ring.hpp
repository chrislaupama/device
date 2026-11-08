// Duplex input FIFO for backends whose input and output are separate
// clients/streams (WASAPI: a capture and a render IAudioClient; PipeWire: a
// capture and a playback pw_stream), possibly on independent clocks.
//
// The capture side writes interleaved float frames; the render side reads
// exactly the frames of its cycle as planar channels, so the engine sees
// input and output of equal length (AudioDevice::Callback contract).
//
// Drift tolerance (no resampling): the reader keeps the fill level near
// `target` frames.
//   - Priming: after reset() or an underrun the reader outputs silence until
//     `target` frames are buffered, so a slow input never starves every
//     cycle.
//   - Input clock faster than output: the fill grows; once it exceeds
//     target + slack the reader skips the excess (a short discontinuity,
//     counted in dropped()).
//   - Input clock slower: the fill shrinks to zero, the missing frames are
//     zeros (counted in underruns()) and the reader re-primes.
// Two clocks differing by 100 ppm drift by ~5 frames per second at 48 kHz,
// so with a slack of a few periods a correction happens every few minutes.
// The added input latency is about `target` frames (backends report it).
//
// Single producer, single consumer, lock-free; read()/write() never
// allocate (RT safe). reset() allocates and must not race with them.
#pragma once

#include <algorithm>
#include <atomic>
#include <cstdint>
#include <cstring>
#include <vector>

namespace saine {

class FrameRing {
public:
    // Non-RT. capacity is rounded up to a power of two.
    void reset(uint32_t channels, uint32_t capacity_frames, uint32_t target_frames, uint32_t slack_frames) {
        channels_ = std::max<uint32_t>(channels, 1);
        uint32_t cap = 1;
        while (cap < capacity_frames) cap <<= 1;
        capacity_ = cap;
        target_ = std::min(target_frames, cap / 2);
        slack_ = slack_frames;
        data_.assign(static_cast<std::size_t>(cap) * channels_, 0.0f);
        write_pos_.store(0, std::memory_order_relaxed);
        read_pos_.store(0, std::memory_order_relaxed);
        primed_ = false;
        underruns_.store(0, std::memory_order_relaxed);
        dropped_.store(0, std::memory_order_relaxed);
        overflows_.store(0, std::memory_order_relaxed);
    }

    uint32_t channels() const noexcept { return channels_; }
    uint32_t target() const noexcept { return target_; }
    uint32_t fill() const noexcept {
        return static_cast<uint32_t>(write_pos_.load(std::memory_order_acquire) -
                                     read_pos_.load(std::memory_order_acquire));
    }

    // Producer. `interleaved` == nullptr writes silence (e.g. WASAPI's
    // AUDCLNT_BUFFERFLAGS_SILENT). Frames that do not fit are dropped
    // (counted in overflows()). Returns the frames written.
    uint32_t write(const float* interleaved, uint32_t frames) noexcept {
        const uint64_t w = write_pos_.load(std::memory_order_relaxed);
        const uint64_t r = read_pos_.load(std::memory_order_acquire);
        const uint32_t space = capacity_ - static_cast<uint32_t>(w - r);
        const uint32_t n = std::min(frames, space);
        if (n < frames) overflows_.fetch_add(frames - n, std::memory_order_relaxed);
        for (uint32_t done = 0; done < n;) {
            const uint32_t at = static_cast<uint32_t>((w + done) & (capacity_ - 1));
            const uint32_t run = std::min(n - done, capacity_ - at);
            float* dst = data_.data() + static_cast<std::size_t>(at) * channels_;
            const std::size_t count = static_cast<std::size_t>(run) * channels_;
            if (interleaved != nullptr)
                std::memcpy(dst, interleaved + static_cast<std::size_t>(done) * channels_, count * sizeof(float));
            else
                std::memset(dst, 0, count * sizeof(float));
            done += run;
        }
        write_pos_.store(w + n, std::memory_order_release);
        return n;
    }

    // Consumer: fills dst[0..channels) (planar, `frames` each; channels may
    // be fewer than the ring's) and applies the drift policy. Returns the
    // frames that came from the input (the rest are zeros). A device cycle
    // read in several chunks passes `cycle_remaining`, the frames it will
    // still read this cycle after this chunk: those count towards priming
    // and are not treated as excess by the drift policy.
    uint32_t read(float* const* dst, uint32_t channels, uint32_t frames, uint32_t cycle_remaining = 0) noexcept {
        const uint32_t ch = std::min(channels, channels_);
        uint64_t r = read_pos_.load(std::memory_order_relaxed);
        const uint64_t w = write_pos_.load(std::memory_order_acquire);
        uint32_t avail = static_cast<uint32_t>(w - r);
        if (!primed_) {
            if (avail < target_ + frames + cycle_remaining) {
                zero(dst, channels, 0, frames);
                return 0;
            }
            // Start `target` frames behind the writer.
            const uint32_t skip = avail - (target_ + frames + cycle_remaining);
            r += skip;
            avail -= skip;
            primed_ = true;
        }
        const uint32_t n = std::min(frames, avail);
        for (uint32_t i = 0; i < n; ++i) {
            const float* src = data_.data() + static_cast<std::size_t>((r + i) & (capacity_ - 1)) * channels_;
            for (uint32_t c = 0; c < ch; ++c) dst[c][i] = src[c];
        }
        for (uint32_t c = ch; c < channels; ++c) std::memset(dst[c], 0, sizeof(float) * n);
        r += n;
        avail -= n;
        if (n < frames) {
            zero(dst, channels, n, frames - n);
            underruns_.fetch_add(1, std::memory_order_relaxed);
            primed_ = false;
        } else if (avail > cycle_remaining && avail - cycle_remaining > target_ + slack_) {
            const uint32_t excess = avail - cycle_remaining - target_;
            dropped_.fetch_add(excess, std::memory_order_relaxed);
            r += excess;
        }
        read_pos_.store(r, std::memory_order_release);
        return n;
    }

    uint64_t underruns() const noexcept { return underruns_.load(std::memory_order_relaxed); }
    uint64_t dropped() const noexcept { return dropped_.load(std::memory_order_relaxed); }
    uint64_t overflows() const noexcept { return overflows_.load(std::memory_order_relaxed); }

private:
    static void zero(float* const* dst, uint32_t channels, uint32_t offset, uint32_t frames) noexcept {
        for (uint32_t c = 0; c < channels; ++c) std::memset(dst[c] + offset, 0, sizeof(float) * frames);
    }

    uint32_t channels_ = 1;
    uint32_t capacity_ = 1;
    uint32_t target_ = 0;
    uint32_t slack_ = 0;
    std::vector<float> data_;
    std::atomic<uint64_t> write_pos_{0};
    std::atomic<uint64_t> read_pos_{0};
    bool primed_ = false; // consumer-owned
    std::atomic<uint64_t> underruns_{0}, dropped_{0}, overflows_{0};
};

} // namespace saine
