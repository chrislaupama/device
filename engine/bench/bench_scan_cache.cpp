// Plugin-cache parse benchmark: times the scanner's cache JSON reader
// (scanner/cache_json.hpp), which runs on every scan before any plugin is
// looked at. Prints measurements only; no pass/fail thresholds.
//
// usage: dstudio_scan_cache_bench [entries=2000] [runs=50]
// Two synthetic caches with `entries` bundles of 2 plugins each:
//   typical:  what write_cache produces (non-ASCII stays raw UTF-8, so \u
//             escapes are rare: only control characters);
//   escapes:  every name, vendor and path written with \uXXXX escapes, the
//             worst case for the escape decoder.
// Each is parsed `runs` times after one warm-up; the median, min and max
// are reported per parse, with throughput and the cost per entry.
#include "cache_json.hpp"

#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <vector>

namespace {

using dstudio::scanner::Json;
using dstudio::scanner::Parser;

// Appends `s` as a JSON string; with `escape_all`, every character after the
// first is written as \uXXXX (ASCII and 2-byte UTF-8 only in this input).
void put_string(std::string& o, const std::string& s, bool escape_all) {
    o += '"';
    for (std::size_t i = 0; i < s.size(); ++i) {
        const auto c = static_cast<unsigned char>(s[i]);
        if (!escape_all || i == 0) {
            o += static_cast<char>(c);
            continue;
        }
        unsigned cp = c;
        if (c >= 0xC0 && i + 1 < s.size()) cp = ((c & 0x1Fu) << 6) | (static_cast<unsigned char>(s[++i]) & 0x3Fu);
        char buf[8];
        std::snprintf(buf, sizeof buf, "\\u%04X", cp);
        o += buf;
    }
    o += '"';
}

// A cache file in write_cache's layout with `entries` bundles.
std::string make_cache(int entries, bool escape_all) {
    std::string o = "{\"version\":1,\"entries\":[";
    for (int e = 0; e < entries; ++e) {
        const std::string n = std::to_string(e);
        const std::string path = "/Library/Audio/Plug-Ins/VST3/Caf\xC3\xA9 Studio " + n + ".vst3";
        o += e ? ",\n {" : "\n {";
        o += "\"format\":\"vst3\",\"target\":";
        put_string(o, path, escape_all);
        o += ",\"stamp\":\"1730000000" + n + "\",\"status\":\"ok\",\"reason\":\"\",\"plugins\":[";
        for (int p = 0; p < 2; ++p) {
            o += p ? ",{" : "{";
            o += "\"format\":\"vst3\",\"id\":\"studio.dstudio.bench." + n + "." + std::to_string(p) + "\",\"path\":";
            put_string(o, path, escape_all);
            o += ",\"name\":";
            put_string(o, "R\xC3\xA9verb Pro " + n, escape_all);
            o += ",\"vendor\":";
            put_string(o, "S\xC3\xA4ine Audio Labs", escape_all);
            o += ",\"version\":\"1.2.3\",\"kind\":\"effect\",\"has_editor\":true,"
                 "\"audio_inputs\":2,\"audio_outputs\":2}";
        }
        o += "]}";
    }
    o += "\n]}\n";
    return o;
}

// Parses `text` and walks the entries the way load_cache does; returns a
// checksum so the work cannot be optimised away (0 on a parse failure).
std::size_t parse_once(const std::string& text) {
    Json root;
    if (!Parser(text).parse(root)) return 0;
    const Json* entries = root.get("entries");
    if (entries == nullptr || entries->type != Json::Arr) return 0;
    std::size_t sum = 0;
    for (const Json& e : entries->a) {
        sum += e.str("target").size() + e.str("stamp").size() + (e.str("status") == "ok");
        if (const Json* ps = e.get("plugins"))
            for (const Json& p : ps->a) sum += p.str("name").size() + p.str("vendor").size();
    }
    return sum;
}

void run(const char* label, const std::string& text, int entries, int runs) {
    std::size_t check = parse_once(text); // warm-up
    if (check == 0) {
        std::printf("%-8s  PARSE FAILED\n", label);
        return;
    }
    std::vector<double> us(static_cast<std::size_t>(runs));
    for (double& t : us) {
        const auto t0 = std::chrono::steady_clock::now();
        check += parse_once(text);
        t = std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - t0).count();
    }
    std::sort(us.begin(), us.end());
    const double med = us[us.size() / 2];
    const auto bytes = static_cast<double>(text.size());
    std::printf("%-8s %8.1f KiB %10.1f %10.1f %10.1f %9.1f %9.3f   (check %zu)\n", label, bytes / 1024.0, med,
                us.front(), us.back(), bytes / med, med / entries, check);
}

} // namespace

int main(int argc, char** argv) {
    const int entries = argc > 1 ? std::max(1, std::atoi(argv[1])) : 2000;
    const int runs = argc > 2 ? std::max(1, std::atoi(argv[2])) : 50;
    std::printf("plugin cache parse: %d entries (2 plugins each), %d runs\n", entries, runs);
    std::printf("%-8s %12s %10s %10s %10s %9s %9s\n", "cache", "size", "median us", "min us", "max us", "MB/s",
                "us/entry");
    run("typical", make_cache(entries, false), entries, runs);
    run("escapes", make_cache(entries, true), entries, runs);
    return 0;
}
