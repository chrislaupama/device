// Main-thread run loop for plugins on Linux, where (unlike macOS and
// Windows) there is no system event loop a plugin can rely on. Plugins hand
// the host file descriptors to watch (typically their own X11 connection) and
// timers to run (repaints, meters), through VST3's Linux::IRunLoop and CLAP's
// clap.posix-fd-support / clap.timer-support. The host runs them from
// dispatch(), which PluginHost::idle() calls on every se_plugin_idle (the app
// calls that every 16 ms while a plugin window is open). A timer therefore
// fires at most once per idle call, and its resolution is the idle rate.
//
// MAIN THREAD ONLY. Callbacks may register or remove entries (their own or
// others') while dispatch() runs; an entry removed during a dispatch is not
// called afterwards.
#pragma once

#include <cstddef>
#include <cstdint>
#include <functional>

namespace dstudio::run_loop {

// Who registered an entry (a plugin instance or editor), so everything it
// registered can be removed at once.
using Owner = const void*;

// poll(2) events to watch for, and what dispatch() reports as ready.
enum : uint32_t { kRead = 1u << 0, kWrite = 1u << 1, kError = 1u << 2 };

// Watches `fd` for `events` (kRead | kWrite; errors are always reported) and
// calls `on_ready(fd, ready)` from dispatch() while it is ready. Replaces an
// earlier registration of the same owner and fd. False for a negative fd.
bool add_fd(Owner owner, int fd, uint32_t events, std::function<void(int fd, uint32_t ready)> on_ready);
// Changes the events watched on a registered fd; false if not registered.
bool modify_fd(Owner owner, int fd, uint32_t events);
// Stops watching; false if not registered.
bool remove_fd(Owner owner, int fd);

// Calls `on_timer(id)` every `period_ms` (at least 1) from dispatch().
// Returns the timer's id: non-zero, unique within the process.
uint32_t add_timer(Owner owner, uint32_t period_ms, std::function<void(uint32_t id)> on_timer);
// Stops a timer; false if `id` is not one of `owner`'s.
bool remove_timer(Owner owner, uint32_t id);

// Removes every fd and timer of `owner` (when the plugin or editor goes away).
void remove_all(Owner owner);

// Polls the fds without blocking and runs their callbacks, then runs the
// timers that are due. Returns the number of callbacks run.
std::size_t dispatch();

// Registered fds and timers (tests).
std::size_t fd_count();
std::size_t timer_count();

} // namespace dstudio::run_loop
