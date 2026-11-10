// Audio file import by content: our own WAV reader (wav.cpp) and, through pinned third-party
// decoders (decoders_impl.c), AIFF / AIFC, FLAC, Ogg Vorbis and MP3. Non-RT only.
#include "wav.hpp"

#include "saine_engine.h"

#include <climits>
#include <cstring>

#define DR_MP3_NO_STDIO
#include "dr_mp3.h"
#define DR_FLAC_NO_STDIO
#include "dr_flac.h"
#define DR_WAV_NO_STDIO
#include "dr_wav.h"
#define STB_VORBIS_NO_STDIO
#define STB_VORBIS_NO_PUSHDATA_API
#define STB_VORBIS_HEADER_ONLY
#include "stb_vorbis.c"

namespace saine {

namespace {

bool has_tag(const uint8_t* data, std::size_t size, const char* tag, std::size_t at = 0) {
    const std::size_t n = std::strlen(tag);
    return size >= at + n && std::memcmp(data + at, tag, n) == 0;
}

bool looks_like_mp3(const uint8_t* d, std::size_t size) {
    return has_tag(d, size, "ID3") || (size >= 2 && d[0] == 0xFF && (d[1] & 0xE0) == 0xE0);
}

// Pulls interleaved float frames from `read(buffer, max_frames) -> frames` until it returns 0
// and stores them planar. Shared by every decoder.
template <class Read>
int32_t collect(uint32_t channels, double rate, Read&& read, DecodedAudio& out, std::string& err, const char* what) {
    if (channels < 1 || channels > 2) {
        err = std::string(what) + ": " + std::to_string(channels) + " channels (only mono and stereo are supported)";
        return SE_ERR_UNSUPPORTED_FORMAT;
    }
    if (!(rate > 0.0)) {
        err = std::string(what) + ": invalid sample rate";
        return SE_ERR_UNSUPPORTED_FORMAT;
    }
    constexpr std::size_t kChunk = 4096;
    std::vector<float> buf(kChunk * channels);
    out = DecodedAudio{};
    out.channels = channels;
    out.sample_rate = rate;
    for (;;) {
        const std::size_t got = read(buf.data(), kChunk);
        if (got == 0) break;
        for (uint32_t c = 0; c < channels; ++c) {
            auto& p = out.planar[c];
            const std::size_t base = p.size();
            p.resize(base + got);
            for (std::size_t i = 0; i < got; ++i) p[base + i] = buf[i * channels + c];
        }
        out.frames += got;
    }
    if (out.frames == 0) {
        err = std::string(what) + ": no audio frames";
        return SE_ERR_UNSUPPORTED_FORMAT;
    }
    return SE_OK;
}

int32_t decode_aiff(const uint8_t* data, std::size_t size, DecodedAudio& out, std::string& err) {
    drwav w;
    if (!drwav_init_memory(&w, data, size, nullptr)) {
        err = "unreadable AIFF file";
        return SE_ERR_UNSUPPORTED_FORMAT;
    }
    const int32_t rc = collect(w.channels, w.sampleRate, [&](float* b, std::size_t n) {
        return static_cast<std::size_t>(drwav_read_pcm_frames_f32(&w, n, b));
    }, out, err, "AIFF");
    drwav_uninit(&w);
    return rc;
}

int32_t decode_flac(const uint8_t* data, std::size_t size, DecodedAudio& out, std::string& err) {
    drflac* f = drflac_open_memory(data, size, nullptr);
    if (f == nullptr) {
        err = "unreadable FLAC file";
        return SE_ERR_UNSUPPORTED_FORMAT;
    }
    const int32_t rc = collect(f->channels, f->sampleRate, [&](float* b, std::size_t n) {
        return static_cast<std::size_t>(drflac_read_pcm_frames_f32(f, n, b));
    }, out, err, "FLAC");
    drflac_close(f);
    return rc;
}

int32_t decode_vorbis(const uint8_t* data, std::size_t size, DecodedAudio& out, std::string& err) {
    if (size > static_cast<std::size_t>(INT_MAX)) {
        err = "Ogg file too large";
        return SE_ERR_UNSUPPORTED_FORMAT;
    }
    int e = 0;
    stb_vorbis* v = stb_vorbis_open_memory(data, static_cast<int>(size), &e, nullptr);
    if (v == nullptr) {
        err = "unreadable Ogg file (only Ogg Vorbis is supported)";
        return SE_ERR_UNSUPPORTED_FORMAT;
    }
    const stb_vorbis_info info = stb_vorbis_get_info(v);
    const auto ch = static_cast<uint32_t>(info.channels);
    const int32_t rc = collect(ch, info.sample_rate, [&](float* b, std::size_t n) {
        const int got = stb_vorbis_get_samples_float_interleaved(v, info.channels, b, static_cast<int>(n * ch));
        return got > 0 ? static_cast<std::size_t>(got) : 0;
    }, out, err, "Ogg Vorbis");
    stb_vorbis_close(v);
    return rc;
}

int32_t decode_mp3(const uint8_t* data, std::size_t size, DecodedAudio& out, std::string& err) {
    drmp3 m;
    if (!drmp3_init_memory(&m, data, size, nullptr)) {
        err = "unreadable MP3 file";
        return SE_ERR_UNSUPPORTED_FORMAT;
    }
    const int32_t rc = collect(m.channels, m.sampleRate, [&](float* b, std::size_t n) {
        return static_cast<std::size_t>(drmp3_read_pcm_frames_f32(&m, n, b));
    }, out, err, "MP3");
    drmp3_uninit(&m);
    return rc;
}

} // namespace

int32_t decode_audio_memory(const uint8_t* data, std::size_t size, DecodedAudio& out, std::string& err) {
    if (has_tag(data, size, "RIFF") || has_tag(data, size, "RF64")) return decode_wav_memory(data, size, out, err);
    if (has_tag(data, size, "FORM") && (has_tag(data, size, "AIFF", 8) || has_tag(data, size, "AIFC", 8)))
        return decode_aiff(data, size, out, err);
    if (has_tag(data, size, "fLaC")) return decode_flac(data, size, out, err);
    if (has_tag(data, size, "OggS")) return decode_vorbis(data, size, out, err);
    if (looks_like_mp3(data, size)) return decode_mp3(data, size, out, err);
    err = "not a supported audio file (WAV, AIFF, FLAC, Ogg Vorbis or MP3)";
    return SE_ERR_UNSUPPORTED_FORMAT;
}

int32_t decode_audio_file(const char* utf8_path, DecodedAudio& out, std::string& err) {
    std::vector<uint8_t> bytes;
    const int32_t rc = read_file_bytes(utf8_path, bytes, err);
    return rc != SE_OK ? rc : decode_audio_memory(bytes.data(), bytes.size(), out, err);
}

} // namespace saine
