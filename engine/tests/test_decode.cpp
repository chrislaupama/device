// Compressed / non-WAV import (decode.cpp): AIFF, FLAC, Ogg Vorbis, MP3, chosen by content.
#include "decode_fixtures.hpp"
#include "test_harness.hpp"
#include "test_util.hpp"
#include "wav.hpp"

#include <cmath>
#include <cstdio>

using namespace testutil;

namespace {

saine::DecodedAudio decode(const uint8_t* data, std::size_t size, int32_t* rc = nullptr) {
    saine::DecodedAudio d;
    std::string err;
    const int32_t r = saine::decode_audio_memory(data, size, d, err);
    if (rc != nullptr) *rc = r;
    return d;
}

double rms(const std::vector<float>& v, std::size_t from, std::size_t to) {
    double s = 0.0;
    for (std::size_t i = from; i < to; ++i) s += static_cast<double>(v[i]) * v[i];
    return std::sqrt(s / static_cast<double>(to - from));
}

int crossings(const std::vector<float>& v, std::size_t from, std::size_t to) {
    int n = 0;
    for (std::size_t i = from + 1; i < to; ++i) n += (v[i - 1] < 0.0f) != (v[i] < 0.0f);
    return n;
}

// The fixtures hold 0.1 s at 48 kHz: left a 440 Hz sine at 0.5, right silent.
void check_tone(const saine::DecodedAudio& d) {
    CHECK_EQ(d.channels, 2u);
    CHECK_EQ(d.sample_rate, 48000.0);
    CHECK(d.frames >= 4700 && d.frames <= 6500);
    REQUIRE(d.frames >= 4000);
    const double l = rms(d.planar[0], 1200, 3600); // away from codec delay and fades
    CHECK(l > 0.32 && l < 0.39);
    float r_peak = 0.0f;
    for (float x : d.planar[1]) r_peak = std::max(r_peak, std::fabs(x));
    CHECK(r_peak < 0.05f);
    const int z = crossings(d.planar[0], 1200, 3600); // 2400 frames of 440 Hz: 44 crossings
    CHECK(z >= 42 && z <= 46);
}

void be16(std::vector<uint8_t>& b, uint32_t v) {
    b.push_back(static_cast<uint8_t>(v >> 8));
    b.push_back(static_cast<uint8_t>(v));
}
void be32(std::vector<uint8_t>& b, uint32_t v) {
    be16(b, v >> 16);
    be16(b, v & 0xFFFF);
}

// 16-bit big-endian AIFF, written by hand (sample rate as an 80-bit extended float).
std::vector<uint8_t> aiff16(const std::vector<int16_t>& interleaved, uint16_t channels, uint32_t rate) {
    const uint32_t frames = static_cast<uint32_t>(interleaved.size() / channels);
    const uint32_t data = static_cast<uint32_t>(interleaved.size() * 2);
    std::vector<uint8_t> b;
    b.insert(b.end(), {'F', 'O', 'R', 'M'});
    be32(b, 4 + 26 + 16 + data);
    b.insert(b.end(), {'A', 'I', 'F', 'F', 'C', 'O', 'M', 'M'});
    be32(b, 18);
    be16(b, channels);
    be32(b, frames);
    be16(b, 16);
    int e = 0;
    while ((rate >> (e + 1)) != 0) ++e;
    be16(b, static_cast<uint32_t>(16383 + e));
    const uint64_t mantissa = static_cast<uint64_t>(rate) << (63 - e);
    be32(b, static_cast<uint32_t>(mantissa >> 32));
    be32(b, static_cast<uint32_t>(mantissa & 0xFFFFFFFFu));
    b.insert(b.end(), {'S', 'S', 'N', 'D'});
    be32(b, 8 + data);
    be32(b, 0);
    be32(b, 0);
    for (int16_t s : interleaved) be16(b, static_cast<uint16_t>(s));
    return b;
}

} // namespace

TEST_CASE(decode_flac_tone) { check_tone(decode(fixtures::kToneFlac, sizeof fixtures::kToneFlac)); }
TEST_CASE(decode_ogg_vorbis_tone) { check_tone(decode(fixtures::kToneOgg, sizeof fixtures::kToneOgg)); }
TEST_CASE(decode_mp3_tone) { check_tone(decode(fixtures::kToneMp3, sizeof fixtures::kToneMp3)); }

TEST_CASE(decode_flac_is_lossless) {
    // FLAC of 16-bit samples: every left sample is a multiple of 1/32768 and the sine is exact.
    const auto d = decode(fixtures::kToneFlac, sizeof fixtures::kToneFlac);
    REQUIRE(d.frames == 4800);
    bool grid = true;
    for (float x : d.planar[0]) grid = grid && std::fabs(x * 32768.0f - std::round(x * 32768.0f)) < 1e-3f;
    CHECK(grid);
    CHECK(std::fabs(d.planar[0][109] - 0.5f * static_cast<float>(std::sin(2.0 * 3.141592653589793 * 440.0 * 109 / 48000.0))) < 2e-4f);
}

TEST_CASE(decode_aiff_exact) {
    std::vector<int16_t> pcm;
    for (int i = 0; i < 1000; ++i) {
        pcm.push_back(static_cast<int16_t>(i * 30 - 15000));
        pcm.push_back(static_cast<int16_t>(-i));
    }
    const auto bytes = aiff16(pcm, 2, 44100);
    int32_t rc = 0;
    const auto d = decode(bytes.data(), bytes.size(), &rc);
    REQUIRE(rc == SE_OK);
    CHECK_EQ(d.channels, 2u);
    CHECK_EQ(d.sample_rate, 44100.0);
    REQUIRE(d.frames == 1000);
    CHECK(d.planar[0][0] == -15000.0f / 32768.0f && d.planar[0][999] == (999 * 30 - 15000) / 32768.0f);
    CHECK(d.planar[1][500] == -500.0f / 32768.0f);
    // Mono AIFF.
    const auto mono = aiff16({1, 2, 3, 4}, 1, 22050);
    const auto m = decode(mono.data(), mono.size(), &rc);
    REQUIRE(rc == SE_OK);
    CHECK(m.channels == 1 && m.frames == 4 && m.sample_rate == 22050.0);
}

TEST_CASE(decode_rejects_unknown_and_broken_files) {
    const uint8_t text[] = {'h', 'e', 'l', 'l', 'o', ' ', 'w', 'o', 'r', 'l', 'd'};
    int32_t rc = 0;
    decode(text, sizeof text, &rc);
    CHECK_EQ(rc, SE_ERR_UNSUPPORTED_FORMAT);
    const uint8_t flac[] = {'f', 'L', 'a', 'C', 0, 0, 0, 0};
    decode(flac, sizeof flac, &rc);
    CHECK_EQ(rc, SE_ERR_UNSUPPORTED_FORMAT);
    decode(fixtures::kToneOgg, 40, &rc); // truncated Ogg
    CHECK_EQ(rc, SE_ERR_UNSUPPORTED_FORMAT);
    decode(nullptr, 0, &rc);
    CHECK_EQ(rc, SE_ERR_UNSUPPORTED_FORMAT);
    const uint8_t bw64[] = {'B', 'W', '6', '4', 0xff, 0xff, 0xff, 0xff, 'W', 'A', 'V', 'E'};
    saine::DecodedAudio d;
    std::string err;
    CHECK_EQ(saine::decode_audio_memory(bw64, sizeof bw64, d, err), SE_ERR_UNSUPPORTED_FORMAT);
    CHECK(err.find("BW64") != std::string::npos);
}

TEST_CASE(asset_load_file_reads_compressed_formats) {
    EngineHandle e(48000, 64);
    TempFile f("decode_tone.mp3"); // the content decides, not the extension
    std::FILE* out = std::fopen(f.path.c_str(), "wb");
    REQUIRE(out != nullptr);
    std::fwrite(fixtures::kToneOgg, 1, sizeof fixtures::kToneOgg, out);
    std::fclose(out);
    se_asset_info info{};
    REQUIRE(se_asset_load_file(e, f.path.c_str(), &info) == SE_OK);
    CHECK_EQ(info.channels, 2u);
    CHECK_EQ(info.source_sample_rate, 48000.0);
    CHECK(info.frames >= 4700);
}
