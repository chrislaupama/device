/*
 * saine_engine.h — C ABI between the Rust application core and the C++23
 * real-time audio engine.
 *
 * Contract (see docs/architecture.md):
 *  - The Rust core owns the authoritative project model. The engine only ever
 *    receives immutable *session snapshots* via se_session_publish().
 *  - se_session_publish() is called from a non-real-time thread. The engine
 *    copies the description, builds a render graph off the audio thread, and
 *    publishes it to the audio callback with an atomic pointer swap. Retired
 *    graphs are reclaimed on a non-RT thread (never freed in the callback).
 *  - Transport commands are pushed through a lock-free SPSC queue and are
 *    applied at the start of the next audio block while the device runs;
 *    with no device running they apply immediately (see additions below).
 *  - All timeline positions/lengths are in sample frames at the engine sample
 *    rate. Musical time (bars/beats/tempo) is converted by the Rust core.
 *  - Every function is safe to call from any non-RT thread, but calls on a
 *    single engine must not race with se_engine_destroy().
 *  - Strings are UTF-8, NUL terminated. Return value SE_OK (0) on success,
 *    negative se_result on failure; se_last_error() gives a message for the
 *    calling thread.
 */
#ifndef SAINE_ENGINE_H
#define SAINE_ENGINE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define SE_ABI_VERSION 1u

typedef enum se_result {
    SE_OK = 0,
    SE_ERR_INVALID_ARG = -1,
    SE_ERR_IO = -2,
    SE_ERR_UNSUPPORTED_FORMAT = -3,
    SE_ERR_DEVICE = -4,
    SE_ERR_NOT_FOUND = -5,
    SE_ERR_STATE = -6,
    SE_ERR_OUT_OF_MEMORY = -7
} se_result;

typedef struct se_engine se_engine;

typedef struct se_config {
    uint32_t abi_version;     /* must be SE_ABI_VERSION */
    double sample_rate;       /* requested; device may override (query se_engine_sample_rate) */
    uint32_t max_block_size;  /* largest block the engine preallocates for */
    uint32_t output_channels; /* 2 for the first milestone */
} se_config;

/* ---- lifecycle ---------------------------------------------------------- */
uint32_t se_abi_version(void);
se_engine* se_engine_create(const se_config* config); /* NULL on failure */
void se_engine_destroy(se_engine* engine);
const char* se_last_error(void); /* thread-local, valid until next call on this thread */

/* Open and start the platform default output device (Core Audio on macOS). */
int32_t se_device_start(se_engine* engine);
int32_t se_device_stop(se_engine* engine);
int32_t se_device_is_running(const se_engine* engine);
double se_engine_sample_rate(const se_engine* engine);

/* ---- media assets ------------------------------------------------------- */
typedef struct se_asset_info {
    uint64_t asset_id;
    uint64_t frames;          /* at engine sample rate (resampled on import) */
    uint32_t channels;        /* 1 or 2 */
    double source_sample_rate;
} se_asset_info;

/* Decode an audio file (WAV PCM 16/24/32, float 32) into engine memory.
 * Blocking; call from a worker thread. Resamples to the engine rate. */
int32_t se_asset_load_file(se_engine* engine, const char* utf8_path, se_asset_info* out_info);
int32_t se_asset_release(se_engine* engine, uint64_t asset_id);
int32_t se_asset_get_info(const se_engine* engine, uint64_t asset_id, se_asset_info* out_info);

/* Min/max peak pairs for waveform drawing, mixed to mono.
 * Writes up to `capacity` pairs (2 floats each) into out_minmax. */
int32_t se_asset_compute_peaks(const se_engine* engine, uint64_t asset_id,
                               uint64_t start_frame, uint32_t frames_per_peak,
                               float* out_minmax, size_t capacity, size_t* out_count);

/* ---- session snapshots -------------------------------------------------- */
typedef struct se_clip_desc {
    uint64_t clip_id;
    uint64_t asset_id;
    int64_t timeline_start; /* frame where the clip starts on the timeline */
    int64_t source_offset;  /* frame offset into the asset (trim start) */
    int64_t length;         /* frames played (trim end = offset + length) */
    float gain;             /* linear */
} se_clip_desc;

typedef struct se_track_desc {
    uint64_t track_id;
    float gain;  /* linear */
    float pan;   /* -1 .. +1, constant-power */
    uint8_t mute;
    uint8_t solo;
    uint8_t _pad[6];
    const se_clip_desc* clips;
    uint32_t clip_count;
} se_track_desc;

typedef struct se_session_desc {
    const se_track_desc* tracks;
    uint32_t track_count;
    float master_gain; /* linear */
} se_session_desc;

/* Copy the description and atomically publish it to the audio thread. */
int32_t se_session_publish(se_engine* engine, const se_session_desc* session);

/* ---- transport ---------------------------------------------------------- */
typedef struct se_transport_info {
    int64_t position;  /* frames */
    uint8_t playing;
    uint8_t _pad[7];
} se_transport_info;

int32_t se_transport_play(se_engine* engine);
int32_t se_transport_pause(se_engine* engine);
int32_t se_transport_stop(se_engine* engine); /* pause + return to position 0 */
int32_t se_transport_seek(se_engine* engine, int64_t frame);
int32_t se_transport_get(const se_engine* engine, se_transport_info* out_info);

/* ---- metering & stats --------------------------------------------------- */
typedef struct se_meter {
    uint64_t track_id; /* 0 = master */
    float peak_l, peak_r; /* linear, decaying peak hold since last read */
} se_meter;

/* Fills up to `capacity` meters (master first). */
int32_t se_meters_read(se_engine* engine, se_meter* out, size_t capacity, size_t* out_count);

typedef struct se_stats {
    uint64_t callbacks;
    uint64_t xruns_or_overloads;    /* callbacks exceeding their deadline */
    double last_callback_us;
    double max_callback_us;
    double avg_callback_us;
    double block_deadline_us;
} se_stats;
int32_t se_stats_get(const se_engine* engine, se_stats* out);
void se_stats_reset(se_engine* engine);

/* ---- offline rendering (tests, export) ---------------------------------- */
/* Render `frames` interleaved stereo frames using the same process path as
 * the device callback. Fails with SE_ERR_STATE while the device is running. */
int32_t se_render_offline(se_engine* engine, float* out_interleaved, uint32_t frames);

/* Render the whole session range to a 32-bit float WAV file. */
int32_t se_export_wav(se_engine* engine, const char* utf8_path, int64_t start_frame, int64_t end_frame);

/* ---- additions (ABI v1, append-only) ------------------------------------
 * Added after the first milestone without changing any existing struct or
 * signature; SE_ABI_VERSION stays 1. Callers that need them can probe with
 * a link-time check; there is no runtime feature query.
 *
 * Transport commands (play/pause/stop/seek and se_transport_set_loop /
 * se_metronome_set) are applied synchronously on the calling thread when no
 * device is running and no se_render_offline call is in progress, so
 * se_transport_get reflects them as soon as the call returns. While the
 * device runs they are queued and applied at the start of the next block. */

/* Non-destructive meter read. Per meter (master first, then tracks in
 * session order, same as se_meters_read) the max absolute peak over the last
 * ~50 ms of rendered audio (a ring of 5 x 10 ms buckets; a single block
 * longer than that counts as one bucket). Resets nothing and does not
 * interfere with se_meters_read. The window advances with rendered frames,
 * so it decays to 0 after ~50 ms of rendered silence (paused output counts),
 * and holds its value while nothing renders. Excludes the metronome. */
int32_t se_meters_peek(se_engine* engine, se_meter* out, size_t capacity, size_t* out_count);

/* Loop playback. When enabled and playing, a playhead in [start, end) (or
 * reaching start from before it) wraps sample-accurately from `end` back to
 * `start`, also mid-block. A playhead at or past `end` plays linearly.
 * Enabling requires start_frame < end_frame (else SE_ERR_INVALID_ARG);
 * when disabling, start/end are ignored. Disabling mid-play continues
 * linearly. se_export_wav ignores the loop. */
int32_t se_transport_set_loop(se_engine* engine, uint8_t enabled, int64_t start_frame, int64_t end_frame);

/* Metronome: a ~15 ms decaying sine click (1.2 kHz downbeat, 800 Hz other
 * beats) mixed into the master output after master gain, while playing.
 * Beat k sounds at timeline frame llround(k * frames_per_beat) (beat 0 at
 * frame 0; negative k in pre-roll), beats where k mod beats_per_bar == 0
 * are downbeats. Clicks ring out across block boundaries, loop wraps and
 * seeks; they stop on pause/stop/disable. Excluded from se_export_wav and
 * from the meters. Validation: gain finite and >= 0, frames_per_beat
 * finite and >= 1, beats_per_bar >= 1 (only checked when enabled). */
typedef struct se_metronome_desc {
    uint8_t enabled;
    uint8_t _pad[3];
    float gain; /* linear */
    double frames_per_beat;
    uint32_t beats_per_bar;
    uint32_t _pad2;
} se_metronome_desc;

int32_t se_metronome_set(se_engine* engine, const se_metronome_desc* desc);

/* ---- MIDI notes + built-in instrument (ABI v1 addition) -----------------
 * se_session_publish_v2 is a superset of se_session_publish: every v2 track
 * carries the v1 fields (same meaning, same validation) plus `instrument`
 * and a note list. Tracks with instrument != 0 play their notes through the
 * engine's built-in polyphonic synthesizer (16 voices per track); the synth
 * output is summed with the track's audio clips and then goes through the
 * track's gain/pan/mute/solo and meters like any other track signal.
 * Notes on tracks with instrument == 0 are ignored. se_session_publish keeps
 * working and is equivalent to v2 with instrument = 0 and no notes.
 *
 * Note semantics: note-on at timeline frame `start`, note-off at `end`
 * (the release tail continues past it); both are sample-accurate. Notes may
 * be given in any order (the engine sorts by start). Validation: pitch
 * 0..127 (60 = C4, A4 = 69 = 440 Hz equal temperament), velocity 1..127,
 * NULL `notes` with note_count > 0 -> SE_ERR_INVALID_ARG. Notes with
 * end <= start are dropped.
 *
 * Republishing keeps sounding notes: voices are matched to notes by
 * (track_id, start, pitch); a matched voice keeps playing (its note-off
 * follows the new `end`), a voice whose note disappeared is released.
 * Seek, stop, pause and loop wraps release every sounding voice with a
 * ~5 ms fade; notes starting exactly at the new position are played, notes
 * already in progress there are not retriggered. se_export_wav renders
 * notes with its own private synth state. */
typedef struct se_note_desc {
    int64_t start;    /* note-on timeline frame */
    int64_t end;      /* note-off timeline frame (exclusive) */
    uint8_t pitch;    /* 0..127 */
    uint8_t velocity; /* 1..127 */
    uint8_t _pad[6];
} se_note_desc; /* 24 bytes, align 8 */

typedef struct se_track_desc_v2 {
    uint64_t track_id;
    float gain;  /* linear */
    float pan;   /* -1 .. +1, constant-power */
    uint8_t mute;
    uint8_t solo;
    uint8_t instrument; /* != 0: render `notes` through the built-in synth */
    uint8_t _pad[5];
    const se_clip_desc* clips;
    uint32_t clip_count;
    uint32_t note_count;
    const se_note_desc* notes;
} se_track_desc_v2; /* 48 bytes, align 8 */

typedef struct se_session_desc_v2 {
    const se_track_desc_v2* tracks;
    uint32_t track_count;
    float master_gain; /* linear */
} se_session_desc_v2; /* 16 bytes, align 8 */

int32_t se_session_publish_v2(se_engine* engine, const se_session_desc_v2* session);

#ifdef __cplusplus
}
#endif
#endif /* SAINE_ENGINE_H */
