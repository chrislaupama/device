#include "test_harness.hpp"
#include "test_util.hpp"
#include "wav.hpp"

#include <bit>
#include <cstring>
#include <limits>
#include <random>

using namespace testutil;

namespace {

void put16(std::vector<uint8_t>& b, uint16_t v) {
    b.push_back(static_cast<uint8_t>(v));
    b.push_back(static_cast<uint8_t>(v >> 8));
}
void put32(std::vector<uint8_t>& b, uint32_t v) {
    for (int i = 0; i < 4; ++i) b.push_back(static_cast<uint8_t>(v >> (8 * i)));
}
void put_tag(std::vector<uint8_t>& b, const char* t) { b.insert(b.end(), t, t + 4); }

struct WavSpec {
    uint16_t tag = 1;
    uint16_t channels = 1;
    uint32_t rate = 48000;
    uint16_t bits = 16;
    bool extensible = false;
    uint16_t valid_bits = 0;
    bool junk_chunk_first = false; // odd-sized chunk before fmt to test padding
};

std::vector<uint8_t> make_wav(const WavSpec& s, const std::vector<uint8_t>& data) {
    std::vector<uint8_t> body;
    put_tag(body, "WAVE");
    if (s.junk_chunk_first) {
        put_tag(body, "LIST");
        put32(body, 3);
        body.push_back('a');
        body.push_back('b');
        body.push_back('c');
        body.push_back(0); // pad byte
    }
    put_tag(body, "fmt ");
    const uint16_t block = static_cast<uint16_t>(s.channels * s.bits / 8);
    put32(body, s.extensible ? 40u : 16u);
    put16(body, s.extensible ? uint16_t{0xFFFE} : s.tag);
    put16(body, s.channels);
    put32(body, s.rate);
    put32(body, s.rate * block);
    put16(body, block);
    put16(body, s.bits);
    if (s.extensible) {
        put16(body, 22);
        put16(body, s.valid_bits ? s.valid_bits : s.bits);
        put32(body, s.channels == 2 ? 3u : 4u);
        put16(body, s.tag);
        const uint8_t suffix[14] = {0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71};
        body.insert(body.end(), suffix, suffix + 14);
    }
    put_tag(body, "data");
    put32(body, static_cast<uint32_t>(data.size()));
    body.insert(body.end(), data.begin(), data.end());
    if (data.size() & 1u) body.push_back(0);

    std::vector<uint8_t> out;
    put_tag(out, "RIFF");
    put32(out, static_cast<uint32_t>(body.size()));
    out.insert(out.end(), body.begin(), body.end());
    return out;
}

int32_t decode(const std::vector<uint8_t>& bytes, dstudio::DecodedAudio& out) {
    std::string err;
    return dstudio::decode_wav_memory(bytes.data(), bytes.size(), out, err);
}

} // namespace

TEST_CASE(wav_float32_roundtrip_bit_exact) {
    std::mt19937 rng(1234);
    std::uniform_real_distribution<float> dist(-1.5f, 1.5f);
    const std::size_t frames = 10007;
    std::vector<std::vector<float>> planar(2, std::vector<float>(frames));
    for (auto& ch : planar)
        for (auto& v : ch) v = dist(rng);
    // Specials: signed zero, denormal, extremes.
    planar[0][0] = -0.0f;
    planar[0][1] = std::numeric_limits<float>::denorm_min();
    planar[0][2] = std::numeric_limits<float>::max();
    planar[1][0] = std::numeric_limits<float>::lowest();
    planar[1][1] = 1.0f;
    planar[1][2] = -1.0f;

    TempFile f("roundtrip.wav");
    write_float_wav(f.path, planar, 44100);
    dstudio::DecodedAudio d;
    std::string err;
    REQUIRE(dstudio::decode_wav_file(f.path.c_str(), d, err) == SE_OK);
    CHECK_EQ(d.channels, 2u);
    CHECK_EQ(d.frames, frames);
    CHECK(d.sample_rate == 44100.0);
    bool exact = true;
    for (uint32_t c = 0; c < 2; ++c)
        for (std::size_t i = 0; i < frames; ++i)
            exact &= std::bit_cast<uint32_t>(d.planar[c][i]) == std::bit_cast<uint32_t>(planar[c][i]);
    CHECK(exact);
}

TEST_CASE(wav_decode_pcm16_stereo) {
    std::vector<uint8_t> data;
    const int16_t samples[] = {0, -32768, 32767, 16384, -1, 1};
    for (int16_t s : samples) put16(data, static_cast<uint16_t>(s));
    WavSpec spec;
    spec.channels = 2;
    spec.bits = 16;
    dstudio::DecodedAudio d;
    REQUIRE(decode(make_wav(spec, data), d) == SE_OK);
    CHECK_EQ(d.frames, 3u);
    CHECK(d.planar[0][0] == 0.0f);
    CHECK(d.planar[1][0] == -1.0f);
    CHECK(d.planar[0][1] == 32767.0f / 32768.0f);
    CHECK(d.planar[1][1] == 0.5f);
    CHECK(d.planar[0][2] == -1.0f / 32768.0f);
}

TEST_CASE(wav_decode_pcm24_mono_with_padding_chunk) {
    std::vector<uint8_t> data;
    const int32_t samples[] = {0, -8388608, 8388607, 4194304, -1};
    for (int32_t s : samples) {
        const auto u = static_cast<uint32_t>(s);
        data.push_back(static_cast<uint8_t>(u));
        data.push_back(static_cast<uint8_t>(u >> 8));
        data.push_back(static_cast<uint8_t>(u >> 16));
    }
    WavSpec spec;
    spec.bits = 24;
    spec.junk_chunk_first = true;
    dstudio::DecodedAudio d;
    REQUIRE(decode(make_wav(spec, data), d) == SE_OK); // 15 data bytes: odd, padded
    CHECK_EQ(d.frames, 5u);
    CHECK(d.planar[0][1] == -1.0f);
    CHECK(d.planar[0][2] == 8388607.0f / 8388608.0f);
    CHECK(d.planar[0][3] == 0.5f);
    CHECK(d.planar[0][4] == -1.0f / 8388608.0f);
}

TEST_CASE(wav_decode_pcm32_and_8bit) {
    {
        std::vector<uint8_t> data;
        put32(data, static_cast<uint32_t>(INT32_MIN));
        put32(data, static_cast<uint32_t>(1 << 30));
        WavSpec spec;
        spec.bits = 32;
        dstudio::DecodedAudio d;
        REQUIRE(decode(make_wav(spec, data), d) == SE_OK);
        CHECK(d.planar[0][0] == -1.0f);
        CHECK(d.planar[0][1] == 0.5f);
    }
    {
        std::vector<uint8_t> data = {128, 0, 192};
        WavSpec spec;
        spec.bits = 8;
        dstudio::DecodedAudio d;
        REQUIRE(decode(make_wav(spec, data), d) == SE_OK);
        CHECK(d.planar[0][0] == 0.0f);
        CHECK(d.planar[0][1] == -1.0f);
        CHECK(d.planar[0][2] == 0.5f);
    }
}

TEST_CASE(wav_decode_extensible_and_float64) {
    {
        // 24-bit valid bits in 32-bit containers, WAVE_FORMAT_EXTENSIBLE/PCM.
        std::vector<uint8_t> data;
        put32(data, static_cast<uint32_t>(4194304) << 8); // 0.5
        put32(data, static_cast<uint32_t>(-8388608) << 8); // -1
        WavSpec spec;
        spec.channels = 2;
        spec.bits = 32;
        spec.valid_bits = 24;
        spec.extensible = true;
        spec.tag = 1;
        dstudio::DecodedAudio d;
        REQUIRE(decode(make_wav(spec, data), d) == SE_OK);
        CHECK_EQ(d.frames, 1u);
        CHECK(d.planar[0][0] == 0.5f);
        CHECK(d.planar[1][0] == -1.0f);
    }
    {
        std::vector<uint8_t> data;
        const uint64_t bits = std::bit_cast<uint64_t>(-0.25);
        put32(data, static_cast<uint32_t>(bits));
        put32(data, static_cast<uint32_t>(bits >> 32));
        WavSpec spec;
        spec.tag = 3;
        spec.bits = 64;
        spec.extensible = true;
        dstudio::DecodedAudio d;
        REQUIRE(decode(make_wav(spec, data), d) == SE_OK);
        CHECK(d.planar[0][0] == -0.25f);
    }
}

TEST_CASE(wav_rejects_unsupported_and_malformed) {
    std::vector<uint8_t> data(12, 0);
    dstudio::DecodedAudio d;
    {
        WavSpec spec;
        spec.channels = 3; // >2 channels are rejected (documented)
        CHECK_EQ(decode(make_wav(spec, data), d), SE_ERR_UNSUPPORTED_FORMAT);
    }
    {
        WavSpec spec;
        spec.tag = 2; // MS ADPCM
        CHECK_EQ(decode(make_wav(spec, data), d), SE_ERR_UNSUPPORTED_FORMAT);
    }
    {
        WavSpec spec;
        spec.rate = 0;
        CHECK_EQ(decode(make_wav(spec, data), d), SE_ERR_UNSUPPORTED_FORMAT);
    }
    {
        std::vector<uint8_t> junk = {'n', 'o', 'p', 'e', 0, 0, 0, 0, 'W', 'A', 'V', 'E', 1, 2, 3};
        CHECK_EQ(decode(junk, d), SE_ERR_UNSUPPORTED_FORMAT);
    }
    {
        // Truncated: header claims more data than present -> clamp, still decodes.
        WavSpec spec;
        auto bytes = make_wav(spec, std::vector<uint8_t>(100, 0));
        bytes.resize(bytes.size() - 51); // leaves 49 bytes -> 24 frames of 16-bit mono
        REQUIRE(decode(bytes, d) == SE_OK);
        CHECK_EQ(d.frames, 24u);
    }
    {
        // No data chunk at all.
        auto bytes = make_wav(WavSpec{}, {});
        bytes.resize(bytes.size() - 8);
        CHECK_EQ(decode(bytes, d), SE_ERR_UNSUPPORTED_FORMAT);
    }
    // Missing file -> IO error through the C API.
    EngineHandle e;
    se_asset_info info{};
    CHECK_EQ(se_asset_load_file(e, "/nonexistent/definitely/missing.wav", &info), SE_ERR_IO);
    CHECK(std::strlen(se_last_error()) > 0);
}

TEST_CASE(export_wav_matches_offline_render) {
    EngineHandle e(48000, 128);
    const uint64_t a = load_asset(e, {sine(20000, 440.0, 48000.0, 0.5), sine(20000, 660.0, 48000.0, 0.25)});
    const uint64_t b = load_asset(e, {ramp(5000, 1e-4f)});
    Session s;
    s.add_track(1, {clip(1, a, 100, 50, 15000, 0.8f)}, 0.9f, -0.3f);
    s.add_track(2, {clip(2, b, 3000, 0, 5000), clip(3, b, 9000, 1000, 2000, 0.5f)}, 1.0f, 0.6f);
    s.master_gain = 0.7f;
    REQUIRE(s.publish(e) == SE_OK);

    TempFile f("export.wav");
    const int64_t start = 1000, end = 17001; // odd length, not block aligned
    REQUIRE(se_export_wav(e, f.path.c_str(), start, end) == SE_OK);

    // Reference: same engine, live path.
    REQUIRE(se_transport_seek(e, start) == SE_OK);
    REQUIRE(se_transport_play(e) == SE_OK);
    const auto ref = render(e, static_cast<uint32_t>(end - start), 333);

    dstudio::DecodedAudio d;
    std::string err;
    REQUIRE(dstudio::decode_wav_file(f.path.c_str(), d, err) == SE_OK);
    REQUIRE(d.frames == static_cast<uint64_t>(end - start));
    CHECK_EQ(d.channels, 2u);
    CHECK(d.sample_rate == 48000.0);
    bool exact = true;
    for (std::size_t i = 0; i < d.frames; ++i) {
        exact &= std::bit_cast<uint32_t>(d.planar[0][i]) == std::bit_cast<uint32_t>(ref[2 * i]);
        exact &= std::bit_cast<uint32_t>(d.planar[1][i]) == std::bit_cast<uint32_t>(ref[2 * i + 1]);
    }
    CHECK(exact);

    CHECK_EQ(se_export_wav(e, f.path.c_str(), 10, 10), SE_ERR_INVALID_ARG);
}

namespace {

// The RIFF file from make_wav rewritten as RF64/BW64: a ds64 chunk with the real sizes, and
// 0xFFFFFFFF in the RIFF size, the data size and (with wide_list) the LIST chunk's size.
std::vector<uint8_t> to_wide(const std::vector<uint8_t>& riff, const char* kind, bool wide_list) {
    std::vector<uint8_t> rest(riff.begin() + 12, riff.end());
    uint64_t data_size = 0;
    uint32_t list_size = 0;
    for (std::size_t pos = 0; pos + 8 <= rest.size();) {
        uint32_t sz = 0;
        std::memcpy(&sz, &rest[pos + 4], 4);
        if (std::memcmp(&rest[pos], "data", 4) == 0) {
            data_size = sz;
            std::memset(&rest[pos + 4], 0xFF, 4);
        } else if (wide_list && std::memcmp(&rest[pos], "LIST", 4) == 0) {
            list_size = sz;
            std::memset(&rest[pos + 4], 0xFF, 4);
        }
        pos += 8 + sz + (sz & 1u);
    }
    std::vector<uint8_t> out;
    put_tag(out, kind);
    put32(out, 0xFFFFFFFFu);
    put_tag(out, "WAVE");
    put_tag(out, "ds64");
    put32(out, wide_list ? 40u : 28u);
    const uint64_t riff_size = riff.size() - 8 + (wide_list ? 48 : 36);
    for (uint64_t v : {riff_size, data_size, uint64_t{0}}) {
        put32(out, static_cast<uint32_t>(v));
        put32(out, static_cast<uint32_t>(v >> 32));
    }
    put32(out, wide_list ? 1u : 0u);
    if (wide_list) {
        put_tag(out, "LIST");
        put32(out, list_size);
        put32(out, 0);
    }
    out.insert(out.end(), rest.begin(), rest.end());
    return out;
}

} // namespace

TEST_CASE(wav_decode_rf64_and_bw64_match_riff) {
    std::vector<uint8_t> data;
    const int16_t samples[] = {0, -32768, 32767, 16384, -1, 1, 1234, -4321};
    for (int16_t s : samples) put16(data, static_cast<uint16_t>(s));
    WavSpec spec;
    spec.channels = 2;
    spec.bits = 16;
    spec.junk_chunk_first = true;
    const std::vector<uint8_t> riff = make_wav(spec, data);
    dstudio::DecodedAudio want;
    REQUIRE(decode(riff, want) == SE_OK);
    for (const char* kind : {"RF64", "BW64"}) {
        for (bool wide_list : {false, true}) {
            dstudio::DecodedAudio d;
            std::string err;
            REQUIRE(dstudio::decode_audio_memory(to_wide(riff, kind, wide_list).data(), to_wide(riff, kind, wide_list).size(), d, err) == SE_OK);
            CHECK_EQ(d.frames, 4u);
            CHECK_EQ(d.channels, 2u);
            CHECK(d.planar[0] == want.planar[0] && d.planar[1] == want.planar[1]);
        }
    }
}

TEST_CASE(wav_decode_rf64_with_32_bit_data_size) {
    std::vector<uint8_t> data;
    for (int16_t s : {int16_t{100}, int16_t{-100}, int16_t{200}}) put16(data, static_cast<uint16_t>(s));
    WavSpec spec;
    std::vector<uint8_t> bytes = to_wide(make_wav(spec, data), "RF64", false);
    for (std::size_t p = 12; p + 8 <= bytes.size(); ++p) {
        if (std::memcmp(&bytes[p], "data", 4) == 0) {
            bytes[p + 4] = 6;
            bytes[p + 5] = bytes[p + 6] = bytes[p + 7] = 0;
            break;
        }
    }
    dstudio::DecodedAudio d;
    REQUIRE(decode(bytes, d) == SE_OK);
    CHECK_EQ(d.frames, 3u);
    CHECK(d.planar[0][2] == 200.0f / 32768.0f);
}

TEST_CASE(wav_decode_rf64_without_ds64_is_rejected) {
    std::vector<uint8_t> bytes = make_wav(WavSpec{}, {0, 0, 1, 0});
    std::memcpy(bytes.data(), "RF64", 4);
    dstudio::DecodedAudio d;
    std::string err;
    CHECK_EQ(dstudio::decode_wav_memory(bytes.data(), bytes.size(), d, err), SE_ERR_UNSUPPORTED_FORMAT);
    CHECK(err.find("ds64") != std::string::npos);
}
