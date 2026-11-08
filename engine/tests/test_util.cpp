#include "test_util.hpp"

#include "wav.hpp"

#include <atomic>
#include <cmath>
#include <cstdio>
#include <filesystem>
#include <numbers>
#include <stdexcept>
#include <unistd.h>

namespace testutil {

TempFile::TempFile(const std::string& stem) {
    static std::atomic<int> counter{0};
    path = (std::filesystem::temp_directory_path() /
            ("saine_test_" + std::to_string(getpid()) + "_" + std::to_string(counter++) + "_" + stem))
               .string();
}

TempFile::~TempFile() {
    std::error_code ec;
    std::filesystem::remove(path, ec);
}

void write_float_wav(const std::string& path, const std::vector<std::vector<float>>& planar, uint32_t rate) {
    const auto ch = static_cast<uint32_t>(planar.size());
    const std::size_t frames = planar.empty() ? 0 : planar[0].size();
    std::vector<float> inter(frames * ch);
    for (std::size_t f = 0; f < frames; ++f)
        for (uint32_t c = 0; c < ch; ++c) inter[f * ch + c] = planar[c][f];
    std::string err;
    if (saine::write_wav_float32(path.c_str(), inter.data(), frames, ch, rate, err) != SE_OK)
        throw std::runtime_error("write_float_wav: " + err);
}

void write_bytes(const std::string& path, const std::vector<uint8_t>& bytes) {
    std::FILE* f = std::fopen(path.c_str(), "wb");
    if (f == nullptr) throw std::runtime_error("cannot write " + path);
    std::fwrite(bytes.data(), 1, bytes.size(), f);
    std::fclose(f);
}

EngineHandle::EngineHandle(double rate, uint32_t max_block) {
    se_config cfg{SE_ABI_VERSION, rate, max_block, 2};
    e = se_engine_create(&cfg);
    if (e == nullptr) throw std::runtime_error(std::string("se_engine_create: ") + se_last_error());
}

EngineHandle::~EngineHandle() { se_engine_destroy(e); }

uint64_t load_asset(se_engine* e, const std::vector<std::vector<float>>& planar, uint32_t rate) {
    TempFile f("asset.wav");
    write_float_wav(f.path, planar, rate);
    se_asset_info info{};
    if (se_asset_load_file(e, f.path.c_str(), &info) != SE_OK)
        throw std::runtime_error(std::string("se_asset_load_file: ") + se_last_error());
    return info.asset_id;
}

std::vector<float> render(se_engine* e, uint32_t frames, uint32_t block) {
    std::vector<float> out(static_cast<std::size_t>(frames) * 2);
    for (uint32_t done = 0; done < frames;) {
        const uint32_t n = std::min(block, frames - done);
        if (se_render_offline(e, out.data() + static_cast<std::size_t>(done) * 2, n) != SE_OK)
            throw std::runtime_error(std::string("se_render_offline: ") + se_last_error());
        done += n;
    }
    return out;
}

std::vector<float> ramp(std::size_t n, float scale) {
    std::vector<float> v(n);
    for (std::size_t i = 0; i < n; ++i) v[i] = static_cast<float>(i + 1) * scale;
    return v;
}

std::vector<float> constant(std::size_t n, float value) { return std::vector<float>(n, value); }

std::vector<float> sine(std::size_t n, double freq, double rate, double amp) {
    std::vector<float> v(n);
    for (std::size_t i = 0; i < n; ++i)
        v[i] = static_cast<float>(amp * std::sin(2.0 * std::numbers::pi * freq * static_cast<double>(i) / rate));
    return v;
}

se_clip_desc clip(uint64_t id, uint64_t asset, int64_t start, int64_t offset, int64_t length, float gain) {
    se_clip_desc c{};
    c.clip_id = id;
    c.asset_id = asset;
    c.timeline_start = start;
    c.source_offset = offset;
    c.length = length;
    c.gain = gain;
    return c;
}

void Session::add_track(uint64_t id, std::vector<se_clip_desc> c, float gain, float pan, bool mute, bool solo) {
    clips.push_back(std::move(c));
    se_track_desc t{};
    t.track_id = id;
    t.gain = gain;
    t.pan = pan;
    t.mute = mute ? 1 : 0;
    t.solo = solo ? 1 : 0;
    tracks.push_back(t);
}

se_session_desc Session::desc() {
    for (std::size_t i = 0; i < tracks.size(); ++i) {
        tracks[i].clips = clips[i].data();
        tracks[i].clip_count = static_cast<uint32_t>(clips[i].size());
    }
    se_session_desc d{};
    d.tracks = tracks.data();
    d.track_count = static_cast<uint32_t>(tracks.size());
    d.master_gain = master_gain;
    return d;
}

int32_t Session::publish(se_engine* e) {
    const se_session_desc d = desc();
    return se_session_publish(e, &d);
}

} // namespace testutil
