#include "os.hpp"

#include <algorithm>
#include <cstdlib>
#include <cstring>
#include <thread>

#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
// After windows.h:
#include <shellapi.h>
#include <share.h>
#else
#include <cerrno>
#include <climits>
#include <csignal>
#include <fcntl.h>
#include <spawn.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>
extern char** environ;
#endif

#if defined(__APPLE__)
#include <mach-o/dyld.h>
#endif

namespace saine::os {

// ---------------------------------------------------------------- strings

#if defined(_WIN32)
std::wstring widen(std::string_view utf8) {
    if (utf8.empty()) return {};
    const int n = MultiByteToWideChar(CP_UTF8, 0, utf8.data(), static_cast<int>(utf8.size()), nullptr, 0);
    if (n <= 0) return {};
    std::wstring w(static_cast<std::size_t>(n), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, utf8.data(), static_cast<int>(utf8.size()), w.data(), n);
    return w;
}

std::string narrow(std::wstring_view wide) {
    if (wide.empty()) return {};
    const int n =
        WideCharToMultiByte(CP_UTF8, 0, wide.data(), static_cast<int>(wide.size()), nullptr, 0, nullptr, nullptr);
    if (n <= 0) return {};
    std::string s(static_cast<std::size_t>(n), '\0');
    WideCharToMultiByte(CP_UTF8, 0, wide.data(), static_cast<int>(wide.size()), s.data(), n, nullptr, nullptr);
    return s;
}
#endif

// ---------------------------------------------------------------- files, paths

std::FILE* fopen_utf8(const char* utf8_path, const char* mode) noexcept {
    if (utf8_path == nullptr || mode == nullptr) return nullptr;
#if defined(_WIN32)
    try {
        // _wfsopen + _SH_DENYNO: share like fopen does on POSIX (_wfopen_s
        // opens without sharing, so readers would fail on open takes/assets).
        return _wfsopen(widen(utf8_path).c_str(), widen(mode).c_str(), _SH_DENYNO);
    } catch (...) {
        return nullptr;
    }
#else
    return std::fopen(utf8_path, mode);
#endif
}

std::filesystem::path path_from_utf8(std::string_view utf8) {
#if defined(_WIN32)
    return std::filesystem::path(widen(utf8));
#else
    return std::filesystem::path(std::string(utf8));
#endif
}

std::string path_to_utf8(const std::filesystem::path& p) {
#if defined(_WIN32)
    return narrow(p.native());
#else
    return p.native();
#endif
}

int64_t file_mtime_ns(const std::string& utf8_path) noexcept {
#if defined(_WIN32)
    WIN32_FILE_ATTRIBUTE_DATA data{};
    try {
        if (!GetFileAttributesExW(widen(utf8_path).c_str(), GetFileExInfoStandard, &data)) return 0;
    } catch (...) {
        return 0;
    }
    const uint64_t t100ns = (static_cast<uint64_t>(data.ftLastWriteTime.dwHighDateTime) << 32) |
                            data.ftLastWriteTime.dwLowDateTime;
    // FILETIME counts 100 ns from 1601; rebase to 1970 so ns fit in int64.
    constexpr uint64_t kUnixEpoch100ns = 116444736000000000ull;
    return t100ns >= kUnixEpoch100ns ? static_cast<int64_t>(t100ns - kUnixEpoch100ns) * 100 : 0;
#else
    struct stat st {};
    if (::stat(utf8_path.c_str(), &st) != 0) return 0;
#if defined(__APPLE__)
    return static_cast<int64_t>(st.st_mtimespec.tv_sec) * 1000000000 + st.st_mtimespec.tv_nsec;
#else
    return static_cast<int64_t>(st.st_mtim.tv_sec) * 1000000000 + st.st_mtim.tv_nsec;
#endif
#endif
}

std::string executable_path() {
#if defined(_WIN32)
    std::wstring buf(1024, L'\0');
    for (;;) {
        const DWORD n = GetModuleFileNameW(nullptr, buf.data(), static_cast<DWORD>(buf.size()));
        if (n == 0) return {};
        if (n < buf.size()) {
            buf.resize(n);
            return narrow(buf);
        }
        buf.resize(buf.size() * 2);
    }
#elif defined(__APPLE__)
    char buf[4096];
    uint32_t size = sizeof buf;
    if (_NSGetExecutablePath(buf, &size) != 0) return {};
    char real[PATH_MAX];
    if (::realpath(buf, real) != nullptr) return real;
    return buf;
#else
    char buf[PATH_MAX];
    const ssize_t n = ::readlink("/proc/self/exe", buf, sizeof buf - 1);
    if (n <= 0) return {};
    return std::string(buf, static_cast<std::size_t>(n));
#endif
}

// ---------------------------------------------------------------- environment

bool get_env(const char* name, std::string& out) {
#if defined(_WIN32)
    const std::wstring wname = widen(name);
    const DWORD n = GetEnvironmentVariableW(wname.c_str(), nullptr, 0);
    if (n == 0) return false;
    std::wstring v(n, L'\0');
    const DWORD got = GetEnvironmentVariableW(wname.c_str(), v.data(), n);
    v.resize(got);
    out = narrow(v);
    return true;
#else
    const char* v = std::getenv(name);
    if (v == nullptr) return false;
    out = v;
    return true;
#endif
}

bool set_env(const char* name, const char* utf8_value) {
#if defined(_WIN32)
    // Both the Win32 environment (inherited by CreateProcessW) and the CRT
    // copy (getenv) are updated.
    const std::wstring wname = widen(name);
    const std::wstring wval = utf8_value != nullptr ? widen(utf8_value) : std::wstring();
    const BOOL ok = SetEnvironmentVariableW(wname.c_str(), utf8_value != nullptr ? wval.c_str() : nullptr);
    _wputenv_s(wname.c_str(), wval.c_str());
    return ok != 0;
#else
    return utf8_value != nullptr ? ::setenv(name, utf8_value, 1) == 0 : ::unsetenv(name) == 0;
#endif
}

char path_list_separator() noexcept {
#if defined(_WIN32)
    return ';';
#else
    return ':';
#endif
}

int process_id() noexcept {
#if defined(_WIN32)
    return static_cast<int>(GetCurrentProcessId());
#else
    return static_cast<int>(::getpid());
#endif
}

std::vector<std::string> utf8_args(int argc, char** argv) {
#if defined(_WIN32)
    (void)argc;
    (void)argv;
    std::vector<std::string> out;
    int n = 0;
    LPWSTR* w = CommandLineToArgvW(GetCommandLineW(), &n);
    if (w == nullptr) return out;
    for (int i = 1; i < n; ++i) out.push_back(narrow(w[i]));
    LocalFree(w);
    return out;
#else
    return std::vector<std::string>(argv + (argc > 0 ? 1 : 0), argv + argc);
#endif
}

// ---------------------------------------------------------------- child processes

std::string ExitStatus::describe() const {
    if (!crashed) return "exit " + std::to_string(code);
#if defined(_WIN32)
    char buf[16];
    std::snprintf(buf, sizeof buf, "0x%08X", static_cast<unsigned>(exception));
    return std::string("crashed (exception ") + buf + ")";
#else
    return "crashed (signal " + std::to_string(code) + ")";
#endif
}

ChildProcess::~ChildProcess() { kill(); }

ChildProcess::ChildProcess(ChildProcess&& o) noexcept {
#if defined(_WIN32)
    process_ = o.process_;
    o.process_ = nullptr;
#else
    pid_ = o.pid_;
    o.pid_ = -1;
#endif
}

ChildProcess& ChildProcess::operator=(ChildProcess&& o) noexcept {
    if (this != &o) {
        kill();
#if defined(_WIN32)
        process_ = o.process_;
        o.process_ = nullptr;
#else
        pid_ = o.pid_;
        o.pid_ = -1;
#endif
    }
    return *this;
}

bool ChildProcess::running() const noexcept {
#if defined(_WIN32)
    return process_ != nullptr;
#else
    return pid_ > 0;
#endif
}

#if defined(_WIN32)

namespace {

// Quotes one argument so CommandLineToArgvW / the MSVC CRT parse it back
// verbatim (backslashes are literal except before a quote).
void append_quoted(std::wstring& cmd, const std::wstring& arg) {
    if (!arg.empty() && arg.find_first_of(L" \t\n\v\"") == std::wstring::npos) {
        cmd += arg;
        return;
    }
    cmd += L'"';
    for (auto it = arg.begin();; ++it) {
        std::size_t backslashes = 0;
        while (it != arg.end() && *it == L'\\') {
            ++it;
            ++backslashes;
        }
        if (it == arg.end()) {
            cmd.append(backslashes * 2, L'\\');
            break;
        }
        if (*it == L'"') {
            cmd.append(backslashes * 2 + 1, L'\\');
        } else {
            cmd.append(backslashes, L'\\');
        }
        cmd += *it;
    }
    cmd += L'"';
}

HANDLE open_inheritable(const std::wstring& path, bool write) {
    SECURITY_ATTRIBUTES sa{};
    sa.nLength = sizeof sa;
    sa.bInheritHandle = TRUE;
    return CreateFileW(path.c_str(), write ? GENERIC_WRITE : GENERIC_READ,
                       FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, &sa,
                       write ? CREATE_ALWAYS : OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
}

ExitStatus status_from_code(DWORD code) {
    ExitStatus s;
    // NTSTATUS error codes (0xC0000005 access violation, 0xC0000409 stack
    // buffer overrun / fail-fast, ...) mean the process died of an
    // unhandled exception.
    if ((code & 0xF0000000u) == 0xC0000000u || code == 0x80000003u) {
        s.crashed = true;
        s.exception = code;
    } else {
        s.code = static_cast<int>(code);
    }
    return s;
}

} // namespace

bool ChildProcess::spawn(const std::string& exe, const std::vector<std::string>& args, const std::string& stdout_path,
                         const std::string& stderr_path, std::string& err) {
    kill();
    const std::wstring wexe = widen(exe);
    std::wstring cmd;
    append_quoted(cmd, wexe);
    for (const auto& a : args) {
        cmd += L' ';
        append_quoted(cmd, widen(a));
    }
    HANDLE in = open_inheritable(L"NUL", false);
    HANDLE out = open_inheritable(widen(stdout_path), true);
    HANDLE errh = open_inheritable(widen(stderr_path), true);
    const auto close_all = [&] {
        for (HANDLE h : {in, out, errh})
            if (h != INVALID_HANDLE_VALUE) CloseHandle(h);
    };
    if (in == INVALID_HANDLE_VALUE || out == INVALID_HANDLE_VALUE || errh == INVALID_HANDLE_VALUE) {
        err = "cannot open the child's redirection files";
        close_all();
        return false;
    }
    // Only these three handles are inherited (not every inheritable handle
    // the host process happens to own).
    HANDLE inherit[3] = {in, out, errh};
    SIZE_T attr_size = 0;
    InitializeProcThreadAttributeList(nullptr, 1, 0, &attr_size);
    std::vector<unsigned char> attr_buf(attr_size);
    auto* attrs = reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(attr_buf.data());
    if (!InitializeProcThreadAttributeList(attrs, 1, 0, &attr_size) ||
        !UpdateProcThreadAttribute(attrs, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, inherit, sizeof inherit, nullptr,
                                   nullptr)) {
        err = "cannot set up the child's handle list";
        close_all();
        return false;
    }
    STARTUPINFOEXW si{};
    si.StartupInfo.cb = sizeof si;
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    si.StartupInfo.hStdInput = in;
    si.StartupInfo.hStdOutput = out;
    si.StartupInfo.hStdError = errh;
    si.lpAttributeList = attrs;
    PROCESS_INFORMATION pi{};
    const BOOL ok = CreateProcessW(wexe.c_str(), cmd.data(), nullptr, nullptr, TRUE,
                                   EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW, nullptr, nullptr,
                                   &si.StartupInfo, &pi);
    const DWORD last = GetLastError();
    DeleteProcThreadAttributeList(attrs);
    close_all();
    if (!ok) {
        err = "CreateProcessW failed (error " + std::to_string(last) + ")";
        return false;
    }
    CloseHandle(pi.hThread);
    process_ = pi.hProcess;
    return true;
}

bool ChildProcess::wait_for(std::chrono::milliseconds timeout, ExitStatus& out) {
    if (process_ == nullptr) return false;
    const auto ms = static_cast<DWORD>(std::max<long long>(0, timeout.count()));
    if (WaitForSingleObject(static_cast<HANDLE>(process_), ms) != WAIT_OBJECT_0) return false;
    DWORD code = 0;
    GetExitCodeProcess(static_cast<HANDLE>(process_), &code);
    out = status_from_code(code);
    reset();
    return true;
}

bool ChildProcess::poll(ExitStatus& out) { return wait_for(std::chrono::milliseconds(0), out); }

void ChildProcess::kill() {
    if (process_ == nullptr) return;
    TerminateProcess(static_cast<HANDLE>(process_), 1);
    WaitForSingleObject(static_cast<HANDLE>(process_), INFINITE);
    reset();
}

void ChildProcess::reset() noexcept {
    if (process_ != nullptr) CloseHandle(static_cast<HANDLE>(process_));
    process_ = nullptr;
}

#else // POSIX

namespace {
ExitStatus status_from_wait(int status) {
    ExitStatus s;
    if (WIFSIGNALED(status)) {
        s.crashed = true;
        s.code = WTERMSIG(status);
    } else if (WIFEXITED(status)) {
        s.code = WEXITSTATUS(status);
    }
    return s;
}
} // namespace

bool ChildProcess::spawn(const std::string& exe, const std::vector<std::string>& args, const std::string& stdout_path,
                         const std::string& stderr_path, std::string& err) {
    kill();
    posix_spawn_file_actions_t fa;
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_addopen(&fa, 0, "/dev/null", O_RDONLY, 0);
    posix_spawn_file_actions_addopen(&fa, 1, stdout_path.c_str(), O_WRONLY | O_CREAT | O_TRUNC, 0600);
    posix_spawn_file_actions_addopen(&fa, 2, stderr_path.c_str(), O_WRONLY | O_CREAT | O_TRUNC, 0600);
    std::vector<std::string> all;
    all.reserve(args.size() + 1);
    all.push_back(exe);
    all.insert(all.end(), args.begin(), args.end());
    std::vector<char*> argv;
    for (auto& a : all) argv.push_back(a.data());
    argv.push_back(nullptr);
    pid_t pid = -1;
    const int rc = posix_spawn(&pid, exe.c_str(), &fa, nullptr, argv.data(), environ);
    posix_spawn_file_actions_destroy(&fa);
    if (rc != 0) {
        err = std::string("posix_spawn failed: ") + std::strerror(rc);
        return false;
    }
    pid_ = static_cast<int>(pid);
    return true;
}

bool ChildProcess::poll(ExitStatus& out) {
    if (pid_ <= 0) return false;
    int status = 0;
    const pid_t r = ::waitpid(static_cast<pid_t>(pid_), &status, WNOHANG);
    if (r != static_cast<pid_t>(pid_)) return false;
    out = status_from_wait(status);
    pid_ = -1;
    return true;
}

bool ChildProcess::wait_for(std::chrono::milliseconds timeout, ExitStatus& out) {
    const auto deadline = std::chrono::steady_clock::now() + timeout;
    for (;;) {
        if (poll(out)) return true;
        if (pid_ <= 0 || std::chrono::steady_clock::now() >= deadline) return false;
        std::this_thread::sleep_for(std::chrono::milliseconds(2));
    }
}

void ChildProcess::kill() {
    if (pid_ <= 0) return;
    ::kill(static_cast<pid_t>(pid_), SIGKILL);
    int status = 0;
    ::waitpid(static_cast<pid_t>(pid_), &status, 0);
    pid_ = -1;
}

void ChildProcess::reset() noexcept { pid_ = -1; }

#endif

} // namespace saine::os
