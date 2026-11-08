// saine_plugin_scanner: out-of-process plugin scanner.
//
//   saine_plugin_scanner --scan --cache <file.json> [--rescan]
//       Coordinator. Enumerates Audio Units (macOS: types aufx/aumf/aumu)
//       and the VST3/CLAP bundles in the platform's standard folders
//         macOS:   /Library/Audio/Plug-Ins/{VST3,CLAP},
//                  ~/Library/Audio/Plug-Ins/{VST3,CLAP}
//         Windows: %COMMONPROGRAMFILES%\{VST3,CLAP},
//                  %LOCALAPPDATA%\Programs\Common\{VST3,CLAP}, %CLAP_PATH%
//         Linux:   ~/.vst3, /usr/lib/vst3, /usr/local/lib/vst3,
//                  ~/.clap, /usr/lib/clap, /usr/local/lib/clap, $CLAP_PATH
//       and every entry of $SAINE_PLUGIN_PATH (':'-separated, ';' on
//       Windows; a directory is searched, a .vst3/.clap path is taken as a
//       bundle). Each AU / bundle not in the
//       cache (or whose stamp changed: AU version, bundle modification time)
//       is inspected by a child process "--scan-one <format> <target>" with
//       a timeout ($SAINE_PLUGIN_SCAN_TIMEOUT_MS, default 10000; up to 4
//       children at a time). A child that crashes, times out or fails is
//       blocklisted in the cache until --rescan or its stamp changes.
//       Writes the cache, prints progress lines ("progress: ...") to stderr
//       and the report to stdout:
//         {"plugins":[{format,id,path,name,vendor,version,kind,has_editor,
//                      audio_inputs,audio_outputs}...],
//          "failed":[{"target":...,"reason":...}...], "from_cache":bool}
//   saine_plugin_scanner --scan-one audio_unit|vst3|clap <id-or-bundle>
//       Child. Prints a JSON array of plugins, exit 0; on failure prints the
//       reason to stderr and exits 2.
#include "platform/os.hpp"
#include "plugins/plugin_scan.hpp"

#include <algorithm>
#include <cctype>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <map>
#include <sstream>
#include <string>
#include <thread>
#include <vector>

#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <fcntl.h>
#include <io.h>
#include <windows.h>
#endif

namespace fs = std::filesystem;
namespace os = saine::os;
using saine::ScannedPlugin;

namespace {

// ---------------------------------------------------------------- JSON

std::string esc(const std::string& s) {
    std::string o;
    o.reserve(s.size() + 2);
    o += '"';
    for (const char ch : s) {
        const auto c = static_cast<unsigned char>(ch);
        switch (c) {
        case '"': o += "\\\""; break;
        case '\\': o += "\\\\"; break;
        case '\n': o += "\\n"; break;
        case '\r': o += "\\r"; break;
        case '\t': o += "\\t"; break;
        default:
            if (c < 0x20) {
                char buf[8];
                std::snprintf(buf, sizeof buf, "\\u%04x", c);
                o += buf;
            } else {
                o += ch;
            }
        }
    }
    o += '"';
    return o;
}

std::string plugin_json(const ScannedPlugin& p) {
    std::ostringstream o;
    o << "{\"format\":" << esc(p.format) << ",\"id\":" << esc(p.id) << ",\"path\":" << esc(p.path)
      << ",\"name\":" << esc(p.name) << ",\"vendor\":" << esc(p.vendor) << ",\"version\":" << esc(p.version)
      << ",\"kind\":" << (p.instrument ? "\"instrument\"" : "\"effect\"")
      << ",\"has_editor\":" << (p.has_editor ? "true" : "false") << ",\"audio_inputs\":" << p.audio_inputs
      << ",\"audio_outputs\":" << p.audio_outputs << "}";
    return o.str();
}

struct Json {
    enum Type { Null, Bool, Num, Str, Arr, Obj } type = Null;
    bool b = false;
    double n = 0;
    std::string s;
    std::vector<Json> a;
    std::vector<std::pair<std::string, Json>> o;
    const Json* get(const std::string& k) const {
        for (const auto& [key, v] : o)
            if (key == k) return &v;
        return nullptr;
    }
    std::string str(const std::string& k) const {
        const Json* v = get(k);
        return v && v->type == Str ? v->s : std::string();
    }
    double num(const std::string& k) const {
        const Json* v = get(k);
        return v && v->type == Num ? v->n : 0.0;
    }
    bool boolean(const std::string& k) const {
        const Json* v = get(k);
        return v && v->type == Bool && v->b;
    }
};

class Parser {
public:
    explicit Parser(const std::string& t) : t_(t) {}
    bool parse(Json& out) {
        if (!value(out)) return false;
        ws();
        return i_ == t_.size();
    }

private:
    void ws() {
        while (i_ < t_.size() && std::isspace(static_cast<unsigned char>(t_[i_]))) ++i_;
    }
    bool lit(const char* w) {
        const std::size_t n = std::strlen(w);
        if (t_.compare(i_, n, w) != 0) return false;
        i_ += n;
        return true;
    }
    bool string(std::string& out) {
        if (i_ >= t_.size() || t_[i_] != '"') return false;
        ++i_;
        while (i_ < t_.size() && t_[i_] != '"') {
            char c = t_[i_++];
            if (c == '\\') {
                if (i_ >= t_.size()) return false;
                const char e = t_[i_++];
                switch (e) {
                case 'n': out += '\n'; break;
                case 'r': out += '\r'; break;
                case 't': out += '\t'; break;
                case 'b': out += '\b'; break;
                case 'f': out += '\f'; break;
                case 'u': {
                    if (i_ + 4 > t_.size()) return false;
                    const unsigned cp = static_cast<unsigned>(std::stoul(t_.substr(i_, 4), nullptr, 16));
                    i_ += 4;
                    if (cp < 0x80) {
                        out += static_cast<char>(cp);
                    } else if (cp < 0x800) {
                        out += static_cast<char>(0xC0 | (cp >> 6));
                        out += static_cast<char>(0x80 | (cp & 0x3F));
                    } else {
                        out += static_cast<char>(0xE0 | (cp >> 12));
                        out += static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
                        out += static_cast<char>(0x80 | (cp & 0x3F));
                    }
                    break;
                }
                default: out += e;
                }
            } else {
                out += c;
            }
        }
        if (i_ >= t_.size()) return false;
        ++i_;
        return true;
    }
    bool value(Json& v) {
        ws();
        if (i_ >= t_.size()) return false;
        const char c = t_[i_];
        if (c == '{') {
            ++i_;
            v.type = Json::Obj;
            ws();
            if (i_ < t_.size() && t_[i_] == '}') return ++i_, true;
            for (;;) {
                ws();
                std::string k;
                if (!string(k)) return false;
                ws();
                if (i_ >= t_.size() || t_[i_++] != ':') return false;
                Json e;
                if (!value(e)) return false;
                v.o.emplace_back(std::move(k), std::move(e));
                ws();
                if (i_ < t_.size() && t_[i_] == ',') {
                    ++i_;
                    continue;
                }
                return i_ < t_.size() && t_[i_++] == '}';
            }
        }
        if (c == '[') {
            ++i_;
            v.type = Json::Arr;
            ws();
            if (i_ < t_.size() && t_[i_] == ']') return ++i_, true;
            for (;;) {
                Json e;
                if (!value(e)) return false;
                v.a.push_back(std::move(e));
                ws();
                if (i_ < t_.size() && t_[i_] == ',') {
                    ++i_;
                    continue;
                }
                return i_ < t_.size() && t_[i_++] == ']';
            }
        }
        if (c == '"') {
            v.type = Json::Str;
            return string(v.s);
        }
        if (lit("true")) return v.type = Json::Bool, v.b = true, true;
        if (lit("false")) return v.type = Json::Bool, v.b = false, true;
        if (lit("null")) return v.type = Json::Null, true;
        char* end = nullptr;
        v.n = std::strtod(t_.c_str() + i_, &end);
        if (end == t_.c_str() + i_) return false;
        v.type = Json::Num;
        i_ = static_cast<std::size_t>(end - t_.c_str());
        return true;
    }
    const std::string& t_;
    std::size_t i_ = 0;
};

ScannedPlugin plugin_from_json(const Json& j) {
    ScannedPlugin p;
    p.format = j.str("format");
    p.id = j.str("id");
    p.path = j.str("path");
    p.name = j.str("name");
    p.vendor = j.str("vendor");
    p.version = j.str("version");
    p.instrument = j.str("kind") == "instrument";
    p.has_editor = j.boolean("has_editor");
    p.audio_inputs = static_cast<uint32_t>(j.num("audio_inputs"));
    p.audio_outputs = static_cast<uint32_t>(j.num("audio_outputs"));
    return p;
}

std::string read_file(const std::string& path) {
    std::ifstream f(os::path_from_utf8(path), std::ios::binary);
    std::ostringstream ss;
    ss << f.rdbuf();
    return ss.str();
}

// ---------------------------------------------------------------- items

struct Item {
    std::string format; // audio_unit | vst3 | clap
    std::string target; // AU id or bundle path
    std::string name;   // for progress
    std::string stamp;  // AU version / bundle mtime
    std::string key() const { return format + ":" + target; }
};

struct CacheEntry {
    std::string format, target, stamp;
    bool ok = false;
    std::string reason;
    std::vector<ScannedPlugin> plugins;
};

// Binary folders inside a bundle's Contents/ (macOS bundles and the VST3
// bundle layout on Windows/Linux).
constexpr const char* kBundleBinaryDirs[] = {"MacOS",        "x86_64-win",    "arm64-win",   "arm64ec-win",
                                             "x86_64-linux", "aarch64-linux", "i386-linux"};

// Newest modification time of the bundle, its Info.plist and its binaries
// (a single-file bundle, e.g. a CLAP on Windows/Linux: the file's own).
std::string bundle_stamp(const fs::path& path) {
    const std::string p = os::path_to_utf8(path);
    int64_t m = std::max(os::file_mtime_ns(p), os::file_mtime_ns(p + "/Contents/Info.plist"));
    std::error_code ec;
    for (const char* sub : kBundleBinaryDirs)
        for (const auto& e : fs::directory_iterator(path / "Contents" / sub, ec))
            m = std::max(m, os::file_mtime_ns(os::path_to_utf8(e.path())));
    return std::to_string(m);
}

// Adds `dir` itself if it is a .vst3/.clap bundle, else searches it (sorted,
// up to 4 levels deep) for bundles, appending them to `out`.
void find_bundles(const fs::path& dir, int depth, std::vector<Item>& out) {
    std::error_code ec;
    const auto add = [&](const fs::path& p) {
        std::string ext = os::path_to_utf8(p.extension());
        for (char& c : ext) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
        if (ext == ".vst3" || ext == ".clap") {
            out.push_back(Item{ext == ".vst3" ? "vst3" : "clap", os::path_to_utf8(p), os::path_to_utf8(p.filename()),
                               bundle_stamp(p)});
            return true;
        }
        return false;
    };
    if (add(dir)) return;
    if (depth > 3 || !fs::is_directory(dir, ec)) return;
    std::vector<fs::path> entries;
    for (const auto& e : fs::directory_iterator(dir, ec)) entries.push_back(e.path());
    std::sort(entries.begin(), entries.end());
    for (const auto& p : entries)
        if (!add(p) && fs::is_directory(p, ec)) find_bundles(p, depth + 1, out);
}

// Everything to scan: the Audio Units, then the VST3/CLAP bundles found in
// the platform's standard folders, $CLAP_PATH and $SAINE_PLUGIN_PATH
// (de-duplicated).
std::vector<Item> enumerate_items() {
    std::vector<Item> items;
    for (const auto& au : saine::enumerate_audio_units())
        items.push_back(Item{"audio_unit", au.id, au.vendor.empty() ? au.name : au.vendor + ": " + au.name,
                             std::to_string(au.version)});
    std::vector<std::string> dirs;
    std::string home, v;
    // Path lists split like Rust's std::env::split_paths (what Settings shows):
    // on Windows a double-quoted section may contain ';' and the quotes are
    // dropped; elsewhere plain ':' splitting.
    const auto split = [&](const std::string& list) {
        std::string part;
        bool quoted = false;
        for (const char c : list) {
#if defined(_WIN32)
            if (c == '"') {
                quoted = !quoted;
                continue;
            }
#endif
            if (c == os::path_list_separator() && !quoted) {
                if (!part.empty()) dirs.push_back(part);
                part.clear();
            } else {
                part += c;
            }
        }
        if (!part.empty()) dirs.push_back(part);
    };
#if defined(__APPLE__)
    os::get_env("HOME", home);
    for (const char* sub : {"VST3", "CLAP"}) {
        dirs.push_back(std::string("/Library/Audio/Plug-Ins/") + sub);
        if (!home.empty()) dirs.push_back(home + "/Library/Audio/Plug-Ins/" + sub);
    }
#elif defined(_WIN32)
    for (const char* sub : {"VST3", "CLAP"}) {
        if (os::get_env("COMMONPROGRAMFILES", v) && !v.empty()) dirs.push_back(v + "\\" + sub);
        if (os::get_env("LOCALAPPDATA", v) && !v.empty()) dirs.push_back(v + "\\Programs\\Common\\" + sub);
    }
    if (os::get_env("CLAP_PATH", v)) split(v);
#else
    os::get_env("HOME", home);
    if (!home.empty()) dirs.push_back(home + "/.vst3");
    dirs.push_back("/usr/lib/vst3");
    dirs.push_back("/usr/local/lib/vst3");
    if (!home.empty()) dirs.push_back(home + "/.clap");
    dirs.push_back("/usr/lib/clap");
    dirs.push_back("/usr/local/lib/clap");
    if (os::get_env("CLAP_PATH", v)) split(v);
#endif
    if (os::get_env("SAINE_PLUGIN_PATH", v)) split(v);
    std::vector<Item> bundles;
    for (const auto& d : dirs) find_bundles(os::path_from_utf8(d), 0, bundles);
    // De-duplicate (a bundle reachable from several roots).
    std::map<std::string, Item> unique;
    for (auto& b : bundles) unique.emplace(b.key(), std::move(b));
    for (auto& [k, b] : unique) items.push_back(std::move(b));
    return items;
}

// ---------------------------------------------------------------- cache

// Reads the scan cache; an empty map if it is missing, malformed or of
// another version.
std::map<std::string, CacheEntry> load_cache(const std::string& path) {
    std::map<std::string, CacheEntry> out;
    const std::string text = read_file(path);
    if (text.empty()) return out;
    Json root;
    if (!Parser(text).parse(root) || root.type != Json::Obj || root.num("version") != 1.0) return out;
    const Json* entries = root.get("entries");
    if (entries == nullptr || entries->type != Json::Arr) return out;
    for (const Json& e : entries->a) {
        CacheEntry c;
        c.format = e.str("format");
        c.target = e.str("target");
        c.stamp = e.str("stamp");
        c.ok = e.str("status") == "ok";
        c.reason = e.str("reason");
        if (const Json* ps = e.get("plugins"); ps && ps->type == Json::Arr)
            for (const Json& p : ps->a) c.plugins.push_back(plugin_from_json(p));
        out[c.format + ":" + c.target] = std::move(c);
    }
    return out;
}

// Writes the scan cache through a temporary file renamed over the old one;
// false on failure.
bool write_cache(const std::string& path, const std::map<std::string, CacheEntry>& cache) {
    std::ostringstream o;
    o << "{\"version\":1,\"entries\":[";
    bool first = true;
    for (const auto& [k, c] : cache) {
        o << (first ? "" : ",") << "\n {\"format\":" << esc(c.format) << ",\"target\":" << esc(c.target)
          << ",\"stamp\":" << esc(c.stamp) << ",\"status\":" << (c.ok ? "\"ok\"" : "\"failed\"")
          << ",\"reason\":" << esc(c.reason) << ",\"plugins\":[";
        for (std::size_t i = 0; i < c.plugins.size(); ++i) o << (i ? "," : "") << plugin_json(c.plugins[i]);
        o << "]}";
        first = false;
    }
    o << "\n]}\n";
    std::error_code ec;
    const fs::path p = os::path_from_utf8(path);
    if (p.has_parent_path()) fs::create_directories(p.parent_path(), ec);
    const fs::path tmp = os::path_from_utf8(path + ".tmp" + std::to_string(os::process_id()));
    {
        std::ofstream f(tmp, std::ios::binary | std::ios::trunc);
        if (!f) return false;
        f << o.str();
        if (!f) return false;
    }
    fs::rename(tmp, p, ec); // replaces an existing cache atomically (also on Windows)
    return !ec;
}

// ---------------------------------------------------------------- children

// Path of this executable (used to spawn children), falling back to argv[0].
std::string self_path(const std::string& argv0) {
    std::string p = os::executable_path();
    return p.empty() ? argv0 : p;
}

// One running "--scan-one" child and its output files.
struct Child {
    std::size_t item = 0;
    os::ChildProcess process;
    std::string out_file, err_file;
    std::chrono::steady_clock::time_point start;
};

// Starts "--scan-one <format> <target>" for items[index] with its output
// redirected to temporary files; false if it could not be started.
bool spawn_child(const std::string& exe, const Item& item, const std::string& tmpdir, std::size_t index, Child& c) {
    c.item = index;
    const std::string stem = tmpdir + "/scan-" + std::to_string(os::process_id()) + "-" + std::to_string(index);
    c.out_file = stem + ".out";
    c.err_file = stem + ".err";
    std::string err;
    const bool ok = c.process.spawn(exe, {"--scan-one", item.format, item.target}, c.out_file, c.err_file, err);
    c.start = std::chrono::steady_clock::now();
    return ok;
}

// Deletes a file, ignoring errors.
void remove_file(const std::string& path) {
    std::error_code ec;
    fs::remove(os::path_from_utf8(path), ec);
}

// Removes trailing whitespace.
std::string trim(std::string s) {
    while (!s.empty() && std::isspace(static_cast<unsigned char>(s.back()))) s.pop_back();
    return s;
}

// Coordinator (--scan): reuses cached results whose stamp is unchanged,
// scans the rest in up to 4 child processes with a timeout, writes the
// cache and prints the JSON report. Returns the process exit code.
int run_coordinator(const std::string& exe, const std::string& cache_path, bool rescan) {
    const auto t_start = std::chrono::steady_clock::now();
    long timeout_ms = 10000;
    if (std::string t; os::get_env("SAINE_PLUGIN_SCAN_TIMEOUT_MS", t)) timeout_ms = std::max(100L, std::atol(t.c_str()));
    std::map<std::string, CacheEntry> cache = rescan ? std::map<std::string, CacheEntry>{} : load_cache(cache_path);
    const std::vector<Item> items = enumerate_items();

    std::map<std::string, CacheEntry> result;
    std::vector<std::size_t> todo;
    for (std::size_t i = 0; i < items.size(); ++i) {
        const Item& it = items[i];
        auto c = cache.find(it.key());
        if (c != cache.end() && c->second.stamp == it.stamp) {
            std::fprintf(stderr, "progress: cached %s\n", it.name.c_str());
            result[it.key()] = c->second;
        } else {
            todo.push_back(i);
        }
    }
    std::fflush(stderr);

    std::error_code tec;
    std::string tmpdir = os::path_to_utf8(fs::temp_directory_path(tec));
    if (tec || tmpdir.empty()) tmpdir = ".";
    while (tmpdir.size() > 1 && (tmpdir.back() == '/' || tmpdir.back() == '\\')) tmpdir.pop_back();
    const std::size_t parallel = std::clamp<std::size_t>(std::thread::hardware_concurrency(), 1, 4);
    std::vector<Child> running;
    std::size_t next = 0;
    while (next < todo.size() || !running.empty()) {
        while (next < todo.size() && running.size() < parallel) {
            const Item& it = items[todo[next]];
            std::fprintf(stderr, "progress: scanning %s\n", it.name.c_str());
            std::fflush(stderr);
            Child c;
            if (!spawn_child(exe, it, tmpdir, todo[next], c)) {
                CacheEntry e{it.format, it.target, it.stamp, false, "could not start the scanner child", {}};
                result[it.key()] = std::move(e);
            } else {
                running.push_back(std::move(c));
            }
            ++next;
        }
        for (auto ch = running.begin(); ch != running.end();) {
            const Item& it = items[ch->item];
            const auto elapsed = std::chrono::duration_cast<std::chrono::milliseconds>(
                                     std::chrono::steady_clock::now() - ch->start)
                                     .count();
            CacheEntry e{it.format, it.target, it.stamp, false, "", {}};
            bool done = false;
            os::ExitStatus status;
            if (ch->process.poll(status)) {
                done = true;
                if (status.crashed) {
                    e.reason = status.describe();
                } else if (status.code == 0) {
                    Json arr;
                    const std::string out = read_file(ch->out_file);
                    if (Parser(out).parse(arr) && arr.type == Json::Arr) {
                        e.ok = true;
                        for (const Json& p : arr.a) e.plugins.push_back(plugin_from_json(p));
                    } else {
                        e.reason = "scanner child returned malformed output";
                    }
                } else {
                    e.reason = trim(read_file(ch->err_file));
                    if (e.reason.empty()) e.reason = "scan failed (" + status.describe() + ")";
                }
            } else if (elapsed > timeout_ms) {
                ch->process.kill();
                done = true;
                e.reason = "timed out after " + std::to_string(timeout_ms) + " ms";
            }
            if (done) {
                remove_file(ch->out_file);
                remove_file(ch->err_file);
                if (!e.ok) std::fprintf(stderr, "progress: failed %s: %s\n", it.name.c_str(), e.reason.c_str());
                result[it.key()] = std::move(e);
                ch = running.erase(ch);
            } else {
                ++ch;
            }
        }
        if (!running.empty()) std::this_thread::sleep_for(std::chrono::milliseconds(5));
    }

    const bool cache_ok = write_cache(cache_path, result);
    if (!cache_ok) std::fprintf(stderr, "progress: could not write the cache %s\n", cache_path.c_str());

    std::ostringstream o;
    o << "{\"plugins\":[";
    bool first = true;
    for (const auto& [k, e] : result)
        for (const auto& p : e.plugins) {
            o << (first ? "" : ",") << "\n" << plugin_json(p);
            first = false;
        }
    o << "\n],\"failed\":[";
    first = true;
    for (const auto& [k, e] : result)
        if (!e.ok) {
            o << (first ? "" : ",") << "\n{\"target\":" << esc(e.target) << ",\"format\":" << esc(e.format)
              << ",\"reason\":" << esc(e.reason) << "}";
            first = false;
        }
    const auto ms =
        std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - t_start).count();
    o << "\n],\"from_cache\":" << (todo.empty() ? "true" : "false") << ",\"scanned\":" << todo.size()
      << ",\"elapsed_ms\":" << ms << "}\n";
    std::fputs(o.str().c_str(), stdout);
    return 0;
}

// Child (--scan-one): scans one plugin and prints its JSON array; exits 2 with
// the reason on stderr on failure. Exits without static destructors.
int run_child(const std::string& format, const std::string& target) {
#if defined(_WIN32)
    // A crashing plugin must not open a Windows Error Reporting dialog: the
    // coordinator reports the crash itself.
    SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);
#endif
    std::vector<ScannedPlugin> plugins;
    std::string err;
    bool ok = false;
    if (format == "audio_unit") ok = saine::scan_audio_unit(target, plugins, err);
    else if (format == "vst3") ok = saine::scan_vst3_bundle(target, plugins, err);
    else if (format == "clap") ok = saine::scan_clap_bundle(target, plugins, err);
    else err = "unknown format " + format;
    if (!ok) {
        std::fprintf(stderr, "%s\n", err.c_str());
        return 2;
    }
    std::string out = "[";
    for (std::size_t i = 0; i < plugins.size(); ++i) out += (i ? "," : "") + plugin_json(plugins[i]);
    out += "]\n";
    std::fputs(out.c_str(), stdout);
    std::fflush(stdout);
    // Skip static destructors / module exit: some plugins misbehave there,
    // and the result is already written.
    std::_Exit(0);
}

// Prints the command-line usage to stderr.
void usage() {
    std::fprintf(stderr, "usage: saine_plugin_scanner --scan --cache <file.json> [--rescan]\n"
                         "       saine_plugin_scanner --scan-one audio_unit|vst3|clap <target>\n");
}

} // namespace

// Dispatches to the child (--scan-one) or the coordinator (--scan); exit 64
// on bad arguments.
int main(int argc, char** argv) {
#if defined(_WIN32)
    // Byte-exact JSON on stdout (no "\n" -> "\r\n" translation).
    _setmode(_fileno(stdout), _O_BINARY);
#endif
    const std::vector<std::string> args = os::utf8_args(argc, argv);
    if (args.size() == 3 && args[0] == "--scan-one") return run_child(args[1], args[2]);
    if (!args.empty() && args[0] == "--scan") {
        std::string cache;
        bool rescan = false;
        for (std::size_t i = 1; i < args.size(); ++i) {
            if (args[i] == "--cache" && i + 1 < args.size()) cache = args[++i];
            else if (args[i] == "--rescan") rescan = true;
            else {
                usage();
                return 64;
            }
        }
        if (cache.empty()) {
            usage();
            return 64;
        }
        return run_coordinator(self_path(argc > 0 ? argv[0] : ""), cache, rescan);
    }
    usage();
    return 64;
}
