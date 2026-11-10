// Bounded lock-free single-producer / single-consumer ring buffer.
//
// The consumer is the audio thread (wait-free pop). Multiple non-RT producers
// must serialise on their side (the engine uses a mutex around push), which
// keeps the RT side trivially lock-free.
#pragma once

#include <array>
#include <atomic>
#include <cstddef>
#include <type_traits>

namespace dstudio {

template <class T, std::size_t Capacity>
class SpscQueue {
    static_assert(Capacity >= 2 && (Capacity & (Capacity - 1)) == 0, "Capacity must be a power of two");
    static_assert(std::is_trivially_copyable_v<T>, "RT queue elements must be trivially copyable");

public:
    bool push(const T& value) noexcept {
        const std::size_t head = head_.load(std::memory_order_relaxed);
        const std::size_t tail = tail_.load(std::memory_order_acquire);
        if (head - tail >= Capacity) return false; // full
        slots_[head & (Capacity - 1)] = value;
        head_.store(head + 1, std::memory_order_release);
        return true;
    }

    bool pop(T& out) noexcept {
        const std::size_t tail = tail_.load(std::memory_order_relaxed);
        const std::size_t head = head_.load(std::memory_order_acquire);
        if (tail == head) return false; // empty
        out = slots_[tail & (Capacity - 1)];
        tail_.store(tail + 1, std::memory_order_release);
        return true;
    }

    // True if nothing is queued. Safe from any thread (a snapshot).
    bool empty() const noexcept {
        return head_.load(std::memory_order_acquire) == tail_.load(std::memory_order_acquire);
    }

private:
    std::array<T, Capacity> slots_{};
    alignas(64) std::atomic<std::size_t> head_{0}; // written by producer
    alignas(64) std::atomic<std::size_t> tail_{0}; // written by consumer
};

} // namespace dstudio
