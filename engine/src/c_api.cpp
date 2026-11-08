// extern "C" surface. Every entry point is noexcept and converts exceptions
// (in practice only std::bad_alloc from non-RT paths) into se_result codes.
// se_last_error() is set only by failing calls (like errno): success paths
// never touch the thread-local, so the RT-adjacent se_render_offline success
// path cannot trigger a lazy TLS allocation.
#include "saine_engine.h"

#include "engine.hpp"

#include <cmath>
#include <new>
#include <string>

struct se_engine {
    std::unique_ptr<saine::Engine> impl;
};

namespace {

thread_local std::string t_last_error;

int32_t fail(int32_t code, std::string msg) noexcept {
    try {
        t_last_error = std::move(msg);
    } catch (...) {
    }
    return code;
}

int32_t fail_from_err(int32_t code, std::string& err) noexcept {
    if (err.empty()) err = "error " + std::to_string(code);
    return fail(code, std::move(err));
}

template <class F>
int32_t guarded(F&& f) noexcept {
    try {
        return f();
    } catch (const std::bad_alloc&) {
        return fail(SE_ERR_OUT_OF_MEMORY, "out of memory");
    } catch (const std::exception& e) {
        return fail(SE_ERR_STATE, std::string("internal error: ") + e.what());
    } catch (...) {
        return fail(SE_ERR_STATE, "internal error");
    }
}

#define SE_REQUIRE_ENGINE(e)                                                                                           \
    do {                                                                                                               \
        if ((e) == nullptr || !(e)->impl) return fail(SE_ERR_INVALID_ARG, "engine is NULL");                           \
    } while (0)

int32_t check(int32_t rc, std::string& err) noexcept { return rc == SE_OK ? SE_OK : fail_from_err(rc, err); }

} // namespace

extern "C" {

uint32_t se_abi_version(void) { return SE_ABI_VERSION; }

const char* se_last_error(void) { return t_last_error.c_str(); }

se_engine* se_engine_create(const se_config* config) {
    if (config == nullptr) {
        fail(SE_ERR_INVALID_ARG, "config is NULL");
        return nullptr;
    }
    try {
        std::string err;
        auto impl = saine::Engine::create(*config, err);
        if (!impl) {
            fail(SE_ERR_INVALID_ARG, err);
            return nullptr;
        }
        return new se_engine{std::move(impl)};
    } catch (const std::bad_alloc&) {
        fail(SE_ERR_OUT_OF_MEMORY, "out of memory");
    } catch (...) {
        fail(SE_ERR_STATE, "internal error creating engine");
    }
    return nullptr;
}

void se_engine_destroy(se_engine* engine) {
    try {
        delete engine;
    } catch (...) {
    }
}

int32_t se_device_start(se_engine* engine) {
    SE_REQUIRE_ENGINE(engine);
    return guarded([&] {
        std::string err;
        return check(engine->impl->device_start(err), err);
    });
}

int32_t se_device_stop(se_engine* engine) {
    SE_REQUIRE_ENGINE(engine);
    return guarded([&] { return engine->impl->device_stop(); });
}

int32_t se_device_is_running(const se_engine* engine) {
    if (engine == nullptr || !engine->impl) return 0;
    return engine->impl->device_running() ? 1 : 0;
}

double se_engine_sample_rate(const se_engine* engine) {
    if (engine == nullptr || !engine->impl) return 0.0;
    return engine->impl->sample_rate();
}

// ---- assets

int32_t se_asset_load_file(se_engine* engine, const char* utf8_path, se_asset_info* out_info) {
    SE_REQUIRE_ENGINE(engine);
    if (utf8_path == nullptr) return fail(SE_ERR_INVALID_ARG, "path is NULL");
    return guarded([&] {
        std::string err;
        return check(engine->impl->asset_load_file(utf8_path, out_info, err), err);
    });
}

int32_t se_asset_release(se_engine* engine, uint64_t asset_id) {
    SE_REQUIRE_ENGINE(engine);
    return guarded([&] {
        const int32_t rc = engine->impl->asset_release(asset_id);
        return rc == SE_OK ? SE_OK : fail(rc, "unknown asset id " + std::to_string(asset_id));
    });
}

int32_t se_asset_get_info(const se_engine* engine, uint64_t asset_id, se_asset_info* out_info) {
    SE_REQUIRE_ENGINE(engine);
    if (out_info == nullptr) return fail(SE_ERR_INVALID_ARG, "out_info is NULL");
    return guarded([&] {
        const int32_t rc = engine->impl->asset_get_info(asset_id, out_info);
        return rc == SE_OK ? SE_OK : fail(rc, "unknown asset id " + std::to_string(asset_id));
    });
}

int32_t se_asset_compute_peaks(const se_engine* engine, uint64_t asset_id, uint64_t start_frame,
                               uint32_t frames_per_peak, float* out_minmax, size_t capacity, size_t* out_count) {
    SE_REQUIRE_ENGINE(engine);
    if (frames_per_peak == 0) return fail(SE_ERR_INVALID_ARG, "frames_per_peak must be > 0");
    if (capacity > 0 && out_minmax == nullptr) return fail(SE_ERR_INVALID_ARG, "out_minmax is NULL");
    return guarded([&] {
        const int32_t rc = engine->impl->asset_compute_peaks(asset_id, start_frame, frames_per_peak, out_minmax,
                                                             capacity, out_count);
        return rc == SE_OK ? SE_OK : fail(rc, "unknown asset id " + std::to_string(asset_id));
    });
}

// ---- session

int32_t se_session_publish(se_engine* engine, const se_session_desc* session) {
    SE_REQUIRE_ENGINE(engine);
    if (session == nullptr) return fail(SE_ERR_INVALID_ARG, "session is NULL");
    return guarded([&] {
        std::string err;
        return check(engine->impl->publish(*session, err), err);
    });
}

int32_t se_session_publish_v2(se_engine* engine, const se_session_desc_v2* session) {
    SE_REQUIRE_ENGINE(engine);
    if (session == nullptr) return fail(SE_ERR_INVALID_ARG, "session is NULL");
    return guarded([&] {
        std::string err;
        return check(engine->impl->publish(*session, err), err);
    });
}

int32_t se_session_publish_v3(se_engine* engine, const se_session_desc_v3* session) {
    SE_REQUIRE_ENGINE(engine);
    if (session == nullptr) return fail(SE_ERR_INVALID_ARG, "session is NULL");
    return guarded([&] {
        std::string err;
        return check(engine->impl->publish(*session, err), err);
    });
}

void se_synth_params_default(se_synth_params* out) {
    if (out != nullptr) *out = saine::default_synth_params();
}

// ---- transport

static int32_t push_cmd(se_engine* engine, const saine::TransportCommand& cmd) {
    SE_REQUIRE_ENGINE(engine);
    const int32_t rc = engine->impl->push_command(cmd);
    return rc == SE_OK ? SE_OK : fail(rc, "transport command queue is full");
}

static int32_t push(se_engine* engine, saine::TransportCommand::Type type, int64_t frame) {
    saine::TransportCommand cmd;
    cmd.type = type;
    cmd.frame = frame;
    return push_cmd(engine, cmd);
}

int32_t se_transport_play(se_engine* engine) { return push(engine, saine::TransportCommand::Type::Play, 0); }
int32_t se_transport_pause(se_engine* engine) { return push(engine, saine::TransportCommand::Type::Pause, 0); }
int32_t se_transport_stop(se_engine* engine) { return push(engine, saine::TransportCommand::Type::Stop, 0); }
int32_t se_transport_seek(se_engine* engine, int64_t frame) {
    return push(engine, saine::TransportCommand::Type::Seek, frame);
}

int32_t se_transport_set_loop(se_engine* engine, uint8_t enabled, int64_t start_frame, int64_t end_frame) {
    SE_REQUIRE_ENGINE(engine);
    if (enabled != 0 && !(start_frame < end_frame))
        return fail(SE_ERR_INVALID_ARG, "loop start must be before loop end");
    saine::TransportCommand cmd;
    cmd.type = saine::TransportCommand::Type::SetLoop;
    cmd.enabled = enabled != 0;
    cmd.frame = start_frame;
    cmd.frame2 = end_frame;
    return push_cmd(engine, cmd);
}

int32_t se_metronome_set(se_engine* engine, const se_metronome_desc* desc) {
    SE_REQUIRE_ENGINE(engine);
    if (desc == nullptr) return fail(SE_ERR_INVALID_ARG, "desc is NULL");
    saine::TransportCommand cmd;
    cmd.type = saine::TransportCommand::Type::SetMetronome;
    cmd.enabled = desc->enabled != 0;
    if (cmd.enabled) {
        if (!std::isfinite(desc->gain) || desc->gain < 0.0f)
            return fail(SE_ERR_INVALID_ARG, "metronome gain must be finite and >= 0");
        if (!std::isfinite(desc->frames_per_beat) || desc->frames_per_beat < 1.0)
            return fail(SE_ERR_INVALID_ARG, "metronome frames_per_beat must be finite and >= 1");
        if (desc->beats_per_bar == 0) return fail(SE_ERR_INVALID_ARG, "metronome beats_per_bar must be >= 1");
        cmd.gain = desc->gain;
        cmd.frames_per_beat = desc->frames_per_beat;
        cmd.beats_per_bar = desc->beats_per_bar;
    }
    return push_cmd(engine, cmd);
}

int32_t se_transport_get(const se_engine* engine, se_transport_info* out_info) {
    SE_REQUIRE_ENGINE(engine);
    if (out_info == nullptr) return fail(SE_ERR_INVALID_ARG, "out_info is NULL");
    engine->impl->transport_get(out_info);
    return SE_OK;
}

// ---- meters & stats

int32_t se_meters_read(se_engine* engine, se_meter* out, size_t capacity, size_t* out_count) {
    SE_REQUIRE_ENGINE(engine);
    if (capacity > 0 && out == nullptr) return fail(SE_ERR_INVALID_ARG, "out is NULL");
    return guarded([&] { return engine->impl->meters_read(out, capacity, out_count); });
}

int32_t se_meters_peek(se_engine* engine, se_meter* out, size_t capacity, size_t* out_count) {
    SE_REQUIRE_ENGINE(engine);
    if (capacity > 0 && out == nullptr) return fail(SE_ERR_INVALID_ARG, "out is NULL");
    return guarded([&] { return engine->impl->meters_peek(out, capacity, out_count); });
}

int32_t se_stats_get(const se_engine* engine, se_stats* out) {
    SE_REQUIRE_ENGINE(engine);
    if (out == nullptr) return fail(SE_ERR_INVALID_ARG, "out is NULL");
    engine->impl->stats_get(out);
    return SE_OK;
}

void se_stats_reset(se_engine* engine) {
    if (engine != nullptr && engine->impl) engine->impl->stats_reset();
}

// ---- offline

int32_t se_render_offline(se_engine* engine, float* out_interleaved, uint32_t frames) {
    SE_REQUIRE_ENGINE(engine);
    if (out_interleaved == nullptr && frames > 0) return fail(SE_ERR_INVALID_ARG, "output buffer is NULL");
    // The success path must not allocate: an empty std::string does not, and
    // guarded() is a plain try/catch.
    return guarded([&] {
        std::string err;
        return check(engine->impl->render_offline(out_interleaved, frames, err), err);
    });
}

int32_t se_export_wav(se_engine* engine, const char* utf8_path, int64_t start_frame, int64_t end_frame) {
    SE_REQUIRE_ENGINE(engine);
    if (utf8_path == nullptr) return fail(SE_ERR_INVALID_ARG, "path is NULL");
    return guarded([&] {
        std::string err;
        return check(engine->impl->export_wav(utf8_path, start_frame, end_frame, err), err);
    });
}

} // extern "C"
