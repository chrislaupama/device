// Delay line for plugin delay compensation (docs/adr/0004-delay-compensation.md).
//
// A power-of-two ring per channel, allocated (zeroed) on a non-RT thread with
// capacity >= delay + max block, so one block is written and then read back
// `d` frames earlier with at most two memcpy per channel and direction.
// Everything else is O(1):
//  - flush: frames written before `valid_from_` read as silence, so a seek
//    or stop never has to clear the ring. Flushes are requested lazily
//    through a generation counter (`gen`, bumped by the RT owner on
//    seek/stop) checked at every use.
//  - idle tracking: `signal_end_` is the write index after the last block
//    that carried (possibly) non-silent input. Once the read position has
//    passed it, the output is silent and an unused line just advances
//    (`discard`) without touching memory.
// RT-safe (no allocation, locks or I/O); used by one RT owner at a time.
#pragma once

#include <algorithm>
#include <cstdint>
#include <cstring>
#include <vector>

namespace saine {

class DelayLine {
public:
    // `capacity` must be a power of two; `channels` 1 or 2.
    DelayLine(uint32_t capacity, uint32_t channels) : mask_(capacity - 1), channels_(channels) {
        for (uint32_t c = 0; c < channels; ++c) buf_[c].assign(capacity, 0.0f);
    }

    uint32_t capacity() const noexcept { return mask_ + 1; }
    uint32_t channels() const noexcept { return channels_; }

    static uint32_t capacity_for(uint32_t delay, uint32_t max_block) noexcept {
        uint32_t c = 1;
        while (c < delay + max_block) c <<= 1;
        return c;
    }

    // In place: x[0..n) becomes the input delayed by `d` frames (d + n <=
    // capacity). Frames from before the last flush read as zeros.
    void process(float* l, float* r, uint32_t n, uint32_t d, uint64_t gen) noexcept {
        sync(gen);
        write(0, l, n);
        if (channels_ > 1) write(1, r, n);
        read(0, l, n, d);
        if (channels_ > 1) read(1, r, n, d);
        w_ += n;
        signal_end_ = w_;
    }
    void process_mono(float* x, uint32_t n, uint32_t d, uint64_t gen) noexcept { process(x, nullptr, n, d, gen); }

    // True when the next output of a `d`-frame delay fed with silence is
    // silent (nothing written since the last flush is still pending).
    bool idle(uint32_t d, uint64_t gen) noexcept {
        sync(gen);
        return signal_end_ <= valid_from_ || w_ >= signal_end_ + d;
    }

    // Advances by `n` frames of silence without producing output; anything
    // still pending is dropped (used for idle lines and muted tracks).
    void discard(uint32_t n, uint64_t gen) noexcept {
        sync(gen);
        w_ += n;
        valid_from_ = w_;
        signal_end_ = w_;
    }

private:
    void sync(uint64_t gen) noexcept {
        if (gen == gen_) return;
        gen_ = gen;
        valid_from_ = w_;
        signal_end_ = w_;
    }
    void write(uint32_t c, const float* x, uint32_t n) noexcept {
        float* b = buf_[c].data();
        const uint32_t at = static_cast<uint32_t>(w_) & mask_;
        const uint32_t first = std::min(n, mask_ + 1 - at);
        std::memcpy(b + at, x, sizeof(float) * first);
        std::memcpy(b, x + first, sizeof(float) * (n - first));
    }
    void read(uint32_t c, float* x, uint32_t n, uint32_t d) const noexcept {
        const float* b = buf_[c].data();
        // d <= capacity - n, so [from, from + n) has not been overwritten.
        const int64_t from = static_cast<int64_t>(w_) - static_cast<int64_t>(d);
        const int64_t invalid = static_cast<int64_t>(valid_from_) - from; // leading frames before the flush
        const auto zeros = static_cast<uint32_t>(std::clamp<int64_t>(invalid, 0, n));
        std::memset(x, 0, sizeof(float) * zeros);
        const uint32_t at = static_cast<uint32_t>(from + zeros) & mask_;
        const uint32_t len = n - zeros;
        const uint32_t first = std::min(len, mask_ + 1 - at);
        std::memcpy(x + zeros, b + at, sizeof(float) * first);
        std::memcpy(x + zeros + first, b, sizeof(float) * (len - first));
    }

    std::vector<float> buf_[2];
    uint32_t mask_;
    uint32_t channels_;
    uint64_t w_ = 0;          // frames written so far
    uint64_t valid_from_ = 0; // frames before this index read as silence
    uint64_t signal_end_ = 0; // write index after the last non-silent block
    uint64_t gen_ = 0;        // flush generation last seen
};

} // namespace saine
