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
// Re-sync (optional, `resync_tolerance` > 0): for backends whose input and
// output run on one clock (PipeWire, both streams in one driver group), the
// fill never drifts, so a lasting change of it means a glitch: a cycle that
// one stream missed and the other did not (an xrun). Left alone, that shifts
// every later recording by a whole cycle against the reported latency. The
// reader therefore watches the fill at the end of each cycle over windows of
// kResyncWindow cycles. The first window after priming sets the reference
// range [lo, hi] (the fill can alternate when the two streams' order within
// a cycle varies). A later window whose *minimum* is above lo + tolerance
// has gained input for good: the excess is skipped (dropped()). A window
// whose *maximum* is below hi - tolerance has lost input: that many frames
// of silence are inserted (inserted()). A single odd cycle never triggers a
// correction, because it moves only one end of a window's range.
//
// The fill read at the end of a render cycle depends on the streams' order:
// a cycle behind if the capture side has not written this cycle yet. A
// lasting change of order would then look like a lost or gained cycle, so a
// backend that knows the driver cycle passes it to write() and read() (a
// non-zero id that both sides see the same in one cycle, PipeWire's clock
// position). The reader then measures the fill as if the capture side had
// already run: an order change leaves it unchanged, a missed cycle does not.
//
// Single producer, single consumer, lock-free; read()/write() never
// allocate (RT safe). reset() allocates and must not race with them.
#pragma once

#include <algorithm>
#include <atomic>
#include <cstdint>
#include <cstring>
#include <vector>

namespace dstudio {

// Single-producer, single-consumer frame FIFO with drift handling (see the
// file comment).
class FrameRing {
public:
    // Sizes the ring and clears it and its counters; the reader re-primes.
    // Non-RT. capacity is rounded up to a power of two; target is capped at
    // half of it.
    void reset(uint32_t channels, uint32_t capacity_frames, uint32_t target_frames, uint32_t slack_frames,
               uint32_t resync_tolerance = 0) {
        channels_ = std::max<uint32_t>(channels, 1);
        uint32_t cap = 1;
        while (cap < capacity_frames) cap <<= 1;
        capacity_ = cap;
        target_ = std::min(target_frames, cap / 2);
        slack_ = slack_frames;
        tolerance_ = resync_tolerance;
        data_.assign(static_cast<std::size_t>(cap) * channels_, 0.0f);
        write_pos_.store(0, std::memory_order_relaxed);
        read_pos_.store(0, std::memory_order_relaxed);
        prime();
        underruns_.store(0, std::memory_order_relaxed);
        dropped_.store(0, std::memory_order_relaxed);
        overflows_.store(0, std::memory_order_relaxed);
        inserted_.store(0, std::memory_order_relaxed);
    }

    // End-of-cycle reads per re-sync window.
    static constexpr uint32_t kResyncWindow = 32;

    // Channels per stored frame.
    uint32_t channels() const noexcept { return channels_; }
    // Fill level the reader aims to keep, in frames.
    uint32_t target() const noexcept { return target_; }
    // Frames currently buffered.
    uint32_t fill() const noexcept {
        return static_cast<uint32_t>(write_pos_.load(std::memory_order_acquire) -
                                     read_pos_.load(std::memory_order_acquire));
    }

    // Producer. `interleaved` == nullptr writes silence (e.g. WASAPI's
    // AUDCLNT_BUFFERFLAGS_SILENT). Frames that do not fit are dropped
    // (counted in overflows()). Returns the frames written.
    // `cycle` (optional, non-zero): the driver cycle this write belongs to
    // (see the file comment).
    uint32_t write(const float* interleaved, uint32_t frames, uint64_t cycle = 0) noexcept {
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
        if (cycle != 0) write_cycle_.store(cycle, std::memory_order_release);
        return n;
    }

    // Consumer: fills dst[0..channels) (planar, `frames` each; channels may
    // be fewer than the ring's) and applies the drift policy. Returns the
    // frames that came from the input (the rest are zeros). A device cycle
    // read in several chunks passes `cycle_remaining`, the frames it will
    // still read this cycle after this chunk: those count towards priming
    // and are not treated as excess by the drift policy. `cycle` (optional,
    // non-zero): the driver cycle being rendered (see the file comment).
    uint32_t read(float* const* dst, uint32_t channels, uint32_t frames, uint32_t cycle_remaining = 0,
                  uint64_t cycle = 0) noexcept {
        const uint32_t cycle_frames = cycle_frames_ + frames; // read this cycle, this chunk included
        cycle_frames_ = cycle_remaining == 0 ? 0 : cycle_frames;
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
        // Silence owed by a re-sync comes first, without consuming input.
        const uint32_t silent = std::min(frames, insert_pending_);
        insert_pending_ -= silent;
        zero(dst, channels, 0, silent);
        const uint32_t want = frames - silent;
        const uint32_t n = std::min(want, avail);
        for (uint32_t i = 0; i < n; ++i) {
            const float* src = data_.data() + static_cast<std::size_t>((r + i) & (capacity_ - 1)) * channels_;
            for (uint32_t c = 0; c < ch; ++c) dst[c][silent + i] = src[c];
        }
        for (uint32_t c = ch; c < channels; ++c) std::memset(dst[c] + silent, 0, sizeof(float) * n);
        r += n;
        avail -= n;
        if (n < want) {
            zero(dst, channels, silent + n, want - n);
            underruns_.fetch_add(1, std::memory_order_relaxed);
            prime();
        } else if (avail > cycle_remaining && avail - cycle_remaining > target_ + slack_) {
            const uint32_t excess = avail - cycle_remaining - target_;
            dropped_.fetch_add(excess, std::memory_order_relaxed);
            r += excess;
            avail -= excess;
        }
        if (primed_ && cycle_remaining == 0 && tolerance_ > 0) {
            // Measured as if the capture side had already run this cycle.
            const bool written = cycle == 0 || write_cycle_.load(std::memory_order_acquire) >= cycle;
            r += resync(avail + (written ? 0 : cycle_frames), avail);
        }
        read_pos_.store(r, std::memory_order_release);
        return n;
    }

    // Short reads after the reader was primed: each one zero-fills the rest of
    // the read and makes the reader prime again before it returns input.
    // Silence returned while priming is not counted.
    uint64_t underruns() const noexcept { return underruns_.load(std::memory_order_relaxed); }
    // Input frames skipped because the fill exceeded target + slack.
    uint64_t dropped() const noexcept { return dropped_.load(std::memory_order_relaxed); }
    // Silent frames inserted by re-syncs after the input fell behind for good.
    uint64_t inserted() const noexcept { return inserted_.load(std::memory_order_relaxed); }
    // Input frames dropped because the ring was full.
    uint64_t overflows() const noexcept { return overflows_.load(std::memory_order_relaxed); }

private:
    // (Re)starts priming and forgets the re-sync reference.
    void prime() noexcept {
        primed_ = false;
        insert_pending_ = 0;
        has_ref_ = false;
        win_count_ = 0;
    }

    // One end-of-cycle fill level for the re-sync windows (see the file
    // comment); `avail` is what can be skipped now. Returns the frames to skip
    // now (an excess); a deficit is queued as silence for the next reads.
    uint32_t resync(uint32_t level, uint32_t avail) noexcept {
        if (win_count_ == 0) win_lo_ = win_hi_ = level;
        win_lo_ = std::min(win_lo_, level);
        win_hi_ = std::max(win_hi_, level);
        if (++win_count_ < kResyncWindow) return 0;
        win_count_ = 0;
        if (!has_ref_) {
            ref_lo_ = win_lo_;
            ref_hi_ = win_hi_;
            has_ref_ = true;
            return 0;
        }
        if (win_lo_ > ref_lo_ + tolerance_) {
            const uint32_t excess = std::min(win_lo_ - ref_lo_, avail);
            dropped_.fetch_add(excess, std::memory_order_relaxed);
            return excess;
        }
        if (win_hi_ + tolerance_ < ref_hi_) {
            const uint32_t deficit = ref_hi_ - win_hi_;
            insert_pending_ += deficit;
            inserted_.fetch_add(deficit, std::memory_order_relaxed);
        }
        return 0;
    }

    // Zeroes `frames` samples from `offset` in each of `channels` planar buffers.
    static void zero(float* const* dst, uint32_t channels, uint32_t offset, uint32_t frames) noexcept {
        for (uint32_t c = 0; c < channels; ++c) std::memset(dst[c] + offset, 0, sizeof(float) * frames);
    }

    uint32_t channels_ = 1;
    uint32_t capacity_ = 1;
    uint32_t target_ = 0;
    uint32_t slack_ = 0;
    uint32_t tolerance_ = 0; // re-sync threshold, 0 = off
    std::vector<float> data_;
    std::atomic<uint64_t> write_pos_{0};
    std::atomic<uint64_t> read_pos_{0};
    // Consumer-owned.
    bool primed_ = false;
    bool has_ref_ = false;
    uint32_t insert_pending_ = 0;
    uint32_t win_count_ = 0, win_lo_ = 0, win_hi_ = 0, ref_lo_ = 0, ref_hi_ = 0;
    uint32_t cycle_frames_ = 0;                // frames read so far this cycle
    std::atomic<uint64_t> write_cycle_{0};     // last cycle the writer tagged
    std::atomic<uint64_t> underruns_{0}, dropped_{0}, overflows_{0}, inserted_{0};
};

} // namespace dstudio
