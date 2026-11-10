// Linux plugin run loop (platform/run_loop.hpp) cost per se_plugin_idle: one
// run_loop::dispatch() with N idle fds (pipes with nothing to read, as a
// plugin's X connection is between events) and N timers not yet due. The app
// calls it every 16 ms while a plugin window is open, on the main thread.
//
//   saine_run_loop_bench [calls=200000]
#include "platform/run_loop.hpp"

#include <unistd.h>

#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <vector>

namespace {

// Average ns per dispatch() with `n` idle fds and `n` timers (best of 5).
double run(int n, int calls) {
    int owner = 0;
    std::vector<int> fds;
    for (int i = 0; i < n; ++i) {
        int p[2];
        if (::pipe(p) != 0) return -1.0;
        fds.push_back(p[0]);
        fds.push_back(p[1]);
        saine::run_loop::add_fd(&owner, p[0], saine::run_loop::kRead, [](int, uint32_t) {});
        saine::run_loop::add_timer(&owner, 60000, [](uint32_t) {});
    }
    double best = 1e30;
    for (int rep = 0; rep < 5; ++rep) {
        std::size_t ran = 0;
        const auto t0 = std::chrono::steady_clock::now();
        for (int k = 0; k < calls; ++k) ran += saine::run_loop::dispatch();
        const auto t1 = std::chrono::steady_clock::now();
        if (ran != 0) std::printf("unexpected callbacks: %zu\n", ran);
        best = std::min(best, std::chrono::duration<double, std::nano>(t1 - t0).count() / calls);
    }
    saine::run_loop::remove_all(&owner);
    for (int fd : fds) ::close(fd);
    return best;
}

} // namespace

int main(int argc, char** argv) {
    const int calls = argc > 1 ? std::atoi(argv[1]) : 200000;
    std::printf("saine_run_loop_bench: %d calls, ns per dispatch() (best of 5)\n", calls);
    std::printf("%12s %12s\n", "fds+timers", "ns");
    for (int n : {0, 1, 4, 16}) std::printf("%12d %12.1f\n", n, run(n, calls));
    return 0;
}
