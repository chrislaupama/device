// Minimal JSON reader for the plugin scanner's cache file.
// Shared by saine_plugin_scanner and the cache-parse benchmark
// (bench/bench_scan_cache.cpp). Not a general-purpose parser: numbers go
// through strtod, \uXXXX escapes decode to UTF-8 without surrogate pairing,
// and any malformed input makes parse() return false (it never throws on
// malformed text).
#pragma once

#include <cctype>
#include <cstdlib>
#include <cstring>
#include <string>
#include <utility>
#include <vector>

namespace saine::scanner {

// A parsed JSON value; only the member matching `type` is meaningful.
struct Json {
    enum Type { Null, Bool, Num, Str, Arr, Obj } type = Null;
    bool b = false;
    double n = 0;
    std::string s;
    std::vector<Json> a;
    std::vector<std::pair<std::string, Json>> o;
    // Member `k` of an object, or nullptr.
    const Json* get(const std::string& k) const {
        for (const auto& [key, v] : o)
            if (key == k) return &v;
        return nullptr;
    }
    // String member `k`, or "" when missing or not a string.
    std::string str(const std::string& k) const {
        const Json* v = get(k);
        return v && v->type == Str ? v->s : std::string();
    }
    // Number member `k`, or 0 when missing or not a number.
    double num(const std::string& k) const {
        const Json* v = get(k);
        return v && v->type == Num ? v->n : 0.0;
    }
    // Bool member `k`; false when missing or not a bool.
    bool boolean(const std::string& k) const {
        const Json* v = get(k);
        return v && v->type == Bool && v->b;
    }
};

// Recursive-descent parser over `t` (which must outlive the parser).
class Parser {
public:
    explicit Parser(const std::string& t) : t_(t) {}
    // Parses the whole text into `out`; false on malformed JSON or trailing text.
    bool parse(Json& out) {
        if (!value(out)) return false;
        ws();
        return i_ == t_.size();
    }

private:
    // Skips whitespace.
    void ws() {
        while (i_ < t_.size() && std::isspace(static_cast<unsigned char>(t_[i_]))) ++i_;
    }
    // Consumes the literal `w` if the text continues with it.
    bool lit(const char* w) {
        const std::size_t n = std::strlen(w);
        if (t_.compare(i_, n, w) != 0) return false;
        i_ += n;
        return true;
    }
    // Parses a JSON string at the cursor into `out`, decoding escapes (\uXXXX
    // to UTF-8). Returns false on a missing quote or a malformed escape.
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
                    // Exactly four hex digits; anything else is malformed
                    // (std::stoul would throw, or accept a sign/spaces).
                    if (i_ + 4 > t_.size()) return false;
                    unsigned cp = 0;
                    for (int k = 0; k < 4; ++k) {
                        const char h = t_[i_++];
                        unsigned d = 0;
                        if (h >= '0' && h <= '9') d = static_cast<unsigned>(h - '0');
                        else if (h >= 'a' && h <= 'f') d = static_cast<unsigned>(h - 'a' + 10);
                        else if (h >= 'A' && h <= 'F') d = static_cast<unsigned>(h - 'A' + 10);
                        else return false;
                        cp = (cp << 4) | d;
                    }
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
    // Parses one value (object, array, string, literal or number) into `v`.
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

} // namespace saine::scanner
