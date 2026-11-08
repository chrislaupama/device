#pragma once

#include "saine_engine.h"

#include <cstdint>
#include <string>
#include <vector>

namespace testutil {

// --- RT allocation tracking (implemented in test_main.cpp)
void set_rt_scope(bool on) noexcept;
long rt_alloc_count() noexcept;
long rt_free_count() noexcept;

struct RtAllocGuard {
    RtAllocGuard() noexcept { set_rt_scope(true); }
    ~RtAllocGuard() { set_rt_scope(false); }
};

// --- files
// Unique path in the system temp dir; removed by TempFile's destructor.
struct TempFile {
    explicit TempFile(const std::string& stem);
    ~TempFile();
    TempFile(const TempFile&) = delete;
    TempFile& operator=(const TempFile&) = delete;
    std::string path;
};

// Writes a 32-bit float WAV from planar channel data.
void write_float_wav(const std::string& path, const std::vector<std::vector<float>>& planar, uint32_t rate);
// Writes raw bytes to a file.
void write_bytes(const std::string& path, const std::vector<uint8_t>& bytes);

// --- engine helpers
struct EngineHandle {
    explicit EngineHandle(double rate = 48000.0, uint32_t max_block = 256);
    ~EngineHandle();
    EngineHandle(const EngineHandle&) = delete;
    EngineHandle& operator=(const EngineHandle&) = delete;
    se_engine* get() const { return e; }
    operator se_engine*() const { return e; }
    se_engine* e = nullptr;
};

// Loads planar float data as an asset via a temp float WAV at `rate`.
uint64_t load_asset(se_engine* e, const std::vector<std::vector<float>>& planar, uint32_t rate = 48000);

// Renders `frames` frames in blocks of `block` (interleaved stereo).
std::vector<float> render(se_engine* e, uint32_t frames, uint32_t block);

std::vector<float> ramp(std::size_t n, float scale = 1e-5f);
std::vector<float> constant(std::size_t n, float v);
std::vector<float> sine(std::size_t n, double freq, double rate, double amp = 0.5);

se_track_desc track(uint64_t id, const std::vector<se_clip_desc>& clips, float gain = 1.0f, float pan = 0.0f,
                    bool mute = false, bool solo = false);
se_clip_desc clip(uint64_t id, uint64_t asset, int64_t start, int64_t offset, int64_t length, float gain = 1.0f);

// Owns session storage (tracks point at the clip vectors held here).
struct Session {
    std::vector<std::vector<se_clip_desc>> clips;
    std::vector<se_track_desc> tracks;
    float master_gain = 1.0f;
    void add_track(uint64_t id, std::vector<se_clip_desc> c, float gain = 1.0f, float pan = 0.0f, bool mute = false,
                   bool solo = false);
    se_session_desc desc();
    int32_t publish(se_engine* e);
};

} // namespace testutil
