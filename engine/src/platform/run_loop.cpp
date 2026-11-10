// Plugin run loop for Linux (see run_loop.hpp). Portable POSIX code; built on
// Linux only, where plugins need it.
#include "run_loop.hpp"

#include <poll.h>

#include <algorithm>
#include <chrono>
#include <cstddef>
#include <vector>

namespace saine::run_loop {
namespace {

using Clock = std::chrono::steady_clock;

struct FdEntry {
    Owner owner = nullptr;
    int fd = -1;
    uint32_t events = 0;
    uint64_t serial = 0; // identifies this registration across re-registrations
    std::function<void(int, uint32_t)> cb;
};

struct TimerEntry {
    Owner owner = nullptr;
    uint32_t id = 0;
    Clock::duration period{};
    Clock::time_point due{};
    std::function<void(uint32_t)> cb;
};

// Main thread only, so no locking.
std::vector<FdEntry> g_fds;
std::vector<TimerEntry> g_timers;
uint64_t g_serial = 0;
uint32_t g_next_timer = 0;

short to_poll(uint32_t events) {
    short e = 0;
    if ((events & kRead) != 0) e |= POLLIN;
    if ((events & kWrite) != 0) e |= POLLOUT;
    return e;
}

uint32_t from_poll(short revents) {
    uint32_t r = 0;
    if ((revents & (POLLIN | POLLHUP)) != 0) r |= kRead;
    if ((revents & POLLOUT) != 0) r |= kWrite;
    if ((revents & (POLLERR | POLLNVAL)) != 0) r |= kError;
    return r;
}

std::vector<FdEntry>::iterator find_fd(Owner owner, int fd) {
    return std::find_if(g_fds.begin(), g_fds.end(), [&](const FdEntry& e) { return e.owner == owner && e.fd == fd; });
}

} // namespace

bool add_fd(Owner owner, int fd, uint32_t events, std::function<void(int, uint32_t)> on_ready) {
    if (fd < 0 || !on_ready) return false;
    if (auto it = find_fd(owner, fd); it != g_fds.end()) g_fds.erase(it);
    g_fds.push_back(FdEntry{owner, fd, events, ++g_serial, std::move(on_ready)});
    return true;
}

bool modify_fd(Owner owner, int fd, uint32_t events) {
    auto it = find_fd(owner, fd);
    if (it == g_fds.end()) return false;
    it->events = events;
    return true;
}

bool remove_fd(Owner owner, int fd) {
    auto it = find_fd(owner, fd);
    if (it == g_fds.end()) return false;
    g_fds.erase(it);
    return true;
}

uint32_t add_timer(Owner owner, uint32_t period_ms, std::function<void(uint32_t)> on_timer) {
    if (!on_timer) return 0;
    if (++g_next_timer == 0) ++g_next_timer;
    const auto period = std::chrono::milliseconds(std::max<uint32_t>(period_ms, 1));
    g_timers.push_back(TimerEntry{owner, g_next_timer, period, Clock::now() + period, std::move(on_timer)});
    return g_next_timer;
}

bool remove_timer(Owner owner, uint32_t id) {
    auto it = std::find_if(g_timers.begin(), g_timers.end(),
                           [&](const TimerEntry& t) { return t.owner == owner && t.id == id; });
    if (it == g_timers.end()) return false;
    g_timers.erase(it);
    return true;
}

void remove_all(Owner owner) {
    std::erase_if(g_fds, [&](const FdEntry& e) { return e.owner == owner; });
    std::erase_if(g_timers, [&](const TimerEntry& t) { return t.owner == owner; });
}

std::size_t dispatch() {
    std::size_t ran = 0;
    // Fds: one non-blocking poll over a snapshot; callbacks may change the
    // registry, so each one is looked up again (by serial) before it runs.
    if (!g_fds.empty()) {
        std::vector<pollfd> pfds;
        std::vector<uint64_t> serials;
        pfds.reserve(g_fds.size());
        serials.reserve(g_fds.size());
        for (const FdEntry& e : g_fds) {
            pfds.push_back(pollfd{e.fd, to_poll(e.events), 0});
            serials.push_back(e.serial);
        }
        if (::poll(pfds.data(), static_cast<nfds_t>(pfds.size()), 0) > 0) {
            for (std::size_t i = 0; i < pfds.size(); ++i) {
                const uint32_t ready = from_poll(pfds[i].revents);
                if (ready == 0) continue;
                auto it = std::find_if(g_fds.begin(), g_fds.end(),
                                       [&](const FdEntry& e) { return e.serial == serials[i]; });
                if (it == g_fds.end()) continue; // removed by an earlier callback
                auto cb = it->cb;                // the callback may remove its own entry
                cb(pfds[i].fd, ready);
                ++ran;
            }
        }
    }
    // Timers: each due timer fires once and is rescheduled from now, so a
    // slow idle rate never queues up a burst of calls.
    const auto now = Clock::now();
    std::vector<std::pair<Owner, uint32_t>> due;
    for (const TimerEntry& t : g_timers)
        if (t.due <= now) due.emplace_back(t.owner, t.id);
    for (const auto& [owner, id] : due) {
        auto it = std::find_if(g_timers.begin(), g_timers.end(),
                               [&](const TimerEntry& t) { return t.owner == owner && t.id == id; });
        if (it == g_timers.end()) continue;
        it->due = now + it->period;
        auto cb = it->cb;
        cb(id);
        ++ran;
    }
    return ran;
}

std::size_t fd_count() { return g_fds.size(); }
std::size_t timer_count() { return g_timers.size(); }

} // namespace saine::run_loop
