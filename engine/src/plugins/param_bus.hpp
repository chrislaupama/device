// Lock-free, bounded multi-producer / multi-consumer queue of parameter
// changes made in plugin editors (se_plugin_take_param_changes). Producers
// are any thread (UI, main, a dispatch queue, the audio thread); push never
// blocks or allocates. Dmitry Vyukov's bounded MPMC ring.
#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <memory>

namespace dstudio {

struct ParamChangeRecord {
    uint64_t device_id = 0;
    int64_t frame = 0;
    uint32_t param_id = 0;
    float normalized = 0.0f;
    bool gesture_begin = false;
    bool gesture_end = false;
};

class ParamBus {
public:
    static constexpr std::size_t kCapacity = 8192; // power of two

    ParamBus() : cells_(new Cell[kCapacity]) {
        for (std::size_t i = 0; i < kCapacity; ++i) cells_[i].seq.store(i, std::memory_order_relaxed);
    }
    ParamBus(const ParamBus&) = delete;
    ParamBus& operator=(const ParamBus&) = delete;

    // Transport position used to stamp changes reported outside process().
    void set_position_source(const std::atomic<int64_t>* pos) noexcept { position_ = pos; }
    int64_t position() const noexcept { return position_ != nullptr ? position_->load(std::memory_order_relaxed) : 0; }

    bool push(const ParamChangeRecord& r) noexcept {
        std::size_t pos = tail_.load(std::memory_order_relaxed);
        for (;;) {
            Cell& c = cells_[pos & (kCapacity - 1)];
            const std::size_t seq = c.seq.load(std::memory_order_acquire);
            const auto diff = static_cast<std::ptrdiff_t>(seq) - static_cast<std::ptrdiff_t>(pos);
            if (diff == 0) {
                if (tail_.compare_exchange_weak(pos, pos + 1, std::memory_order_relaxed)) {
                    c.value = r;
                    c.seq.store(pos + 1, std::memory_order_release);
                    return true;
                }
            } else if (diff < 0) {
                dropped_.fetch_add(1, std::memory_order_relaxed);
                return false; // full
            } else {
                pos = tail_.load(std::memory_order_relaxed);
            }
        }
    }

    bool pop(ParamChangeRecord& r) noexcept {
        std::size_t pos = head_.load(std::memory_order_relaxed);
        for (;;) {
            Cell& c = cells_[pos & (kCapacity - 1)];
            const std::size_t seq = c.seq.load(std::memory_order_acquire);
            const auto diff = static_cast<std::ptrdiff_t>(seq) - static_cast<std::ptrdiff_t>(pos + 1);
            if (diff == 0) {
                if (head_.compare_exchange_weak(pos, pos + 1, std::memory_order_relaxed)) {
                    r = c.value;
                    c.seq.store(pos + kCapacity, std::memory_order_release);
                    return true;
                }
            } else if (diff < 0) {
                return false; // empty
            } else {
                pos = head_.load(std::memory_order_relaxed);
            }
        }
    }

    uint64_t dropped() const noexcept { return dropped_.load(std::memory_order_relaxed); }

private:
    struct Cell {
        std::atomic<std::size_t> seq{0};
        ParamChangeRecord value;
    };
    std::unique_ptr<Cell[]> cells_;
    alignas(64) std::atomic<std::size_t> tail_{0};
    alignas(64) std::atomic<std::size_t> head_{0};
    std::atomic<uint64_t> dropped_{0};
    const std::atomic<int64_t>* position_ = nullptr;
};

} // namespace dstudio
