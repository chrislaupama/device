// Small OS portability layer (files, paths, environment, child processes).
//
// Every string crossing this interface is UTF-8. On Windows the
// implementation converts to UTF-16 and uses the wide APIs (_wfopen,
// CreateProcessW, GetModuleFileNameW, _wgetenv), so non-ASCII paths work
// regardless of the process code page. On macOS/Linux the narrow APIs are
// already UTF-8.
//
// Not real-time safe: none of this may be called on the audio thread.
#pragma once

#include <chrono>
#include <cstdint>
#include <cstdio>
#include <filesystem>
#include <string>
#include <string_view>
#include <vector>

namespace dstudio::os {

// ---------------------------------------------------------------- files, paths

// fopen() for a UTF-8 path (Windows: _wfopen).
std::FILE* fopen_utf8(const char* utf8_path, const char* mode) noexcept;

// UTF-8 <-> std::filesystem::path without going through the ANSI code page.
std::filesystem::path path_from_utf8(std::string_view utf8);
std::string path_to_utf8(const std::filesystem::path& p);

// Last modification time in nanoseconds (0 if the path does not exist).
// Only meaningful for equality comparisons (the epoch is unspecified).
int64_t file_mtime_ns(const std::string& utf8_path) noexcept;

// Absolute UTF-8 path of the running executable ("" if unknown).
std::string executable_path();

// ---------------------------------------------------------------- environment

// Value of an environment variable as UTF-8; false if it is not set.
bool get_env(const char* name, std::string& out);
// Sets (or, with value == nullptr, removes) an environment variable of
// this process; inherited by children spawned afterwards.
bool set_env(const char* name, const char* utf8_value);

// Separator of PATH-like lists ($DSTUDIO_PLUGIN_PATH): ';' on Windows (drive
// letters contain ':'), ':' elsewhere.
char path_list_separator() noexcept;

// Operating-system id of the current process.
int process_id() noexcept;

// The command-line arguments (without argv[0]) as UTF-8. Windows: from
// GetCommandLineW (argv is in the ANSI code page there).
std::vector<std::string> utf8_args(int argc, char** argv);

#if defined(_WIN32)
// UTF-8 <-> UTF-16 for the wide Win32 APIs. Invalid sequences are replaced
// with U+FFFD, not rejected: these are conversions, not input checks.
std::wstring widen(std::string_view utf8);
std::string narrow(std::wstring_view wide);
#endif

// ---------------------------------------------------------------- child processes

// How a child process ended.
struct ExitStatus {
    bool crashed = false;   // POSIX: killed by a signal; Windows: NTSTATUS exception code
    int code = 0;           // exit code, or the signal number when crashed (POSIX)
    uint32_t exception = 0; // Windows: the exception code when crashed
    // Human-readable: "exit 2", "crashed (signal 11)", "crashed (exception 0xC0000005)".
    std::string describe() const;
};

// One child process with stdin from the null device and stdout/stderr
// redirected to files (created/truncated). Destroying a running child kills
// it. Used by the out-of-process plugin scanner and its tests.
class ChildProcess {
public:
    ChildProcess() = default;
    ~ChildProcess();
    ChildProcess(ChildProcess&& o) noexcept;
    ChildProcess& operator=(ChildProcess&& o) noexcept;
    ChildProcess(const ChildProcess&) = delete;
    ChildProcess& operator=(const ChildProcess&) = delete;

    // `args` excludes argv[0] (the executable path is passed as argv[0]).
    bool spawn(const std::string& exe, const std::vector<std::string>& args, const std::string& stdout_path,
               const std::string& stderr_path, std::string& err);
    // Non-blocking. True once the child has finished (`out` filled).
    bool poll(ExitStatus& out);
    // Blocks up to `timeout`; true once the child has finished.
    bool wait_for(std::chrono::milliseconds timeout, ExitStatus& out);
    // Forcibly terminates the child and reaps it.
    void kill();
    // True while a child has been spawned and not yet reaped.
    bool running() const noexcept;

private:
    // Forgets the child (Windows: closes its handle) without signalling it.
    void reset() noexcept;
#if defined(_WIN32)
    void* process_ = nullptr; // HANDLE
#else
    int pid_ = -1;
#endif
};

} // namespace dstudio::os
