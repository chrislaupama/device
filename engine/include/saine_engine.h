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

/* ---- built-in synth parameters (ABI v1 addition) ------------------------
 * se_session_publish_v3 is a superset of se_session_publish_v2: every v3
 * track carries the v2 fields (same meaning, same validation) plus a pointer
 * to the parameters of its built-in synth. NULL (and every v1/v2 publish)
 * means the defaults, which se_synth_params_default() writes (they equal
 * saine_core::instrument::SynthParams::default()). Parameters are copied
 * into the render graph, so they change atomically with the rest of the
 * session; continuous parameters are smoothed by the engine (~10 ms), so a
 * knob sweep published as many snapshots does not zipper. Parameters on
 * non-instrument tracks are validated but unused.
 *
 * Validation (else SE_ERR_INVALID_ARG): enums in range, every float finite
 * and inside the documented range, octave -3..3, semitones -12..12,
 * polyphony 1..16, unison 1..4. Reserved/padding bytes are ignored.
 *
 * Enum values: waveform 0 saw, 1 square, 2 triangle, 3 sine;
 * filter_mode 0 low-pass, 1 high-pass, 2 band-pass. */
enum { SE_WAVE_SAW = 0, SE_WAVE_SQUARE = 1, SE_WAVE_TRIANGLE = 2, SE_WAVE_SINE = 3 };
enum { SE_FILTER_LOWPASS = 0, SE_FILTER_HIGHPASS = 1, SE_FILTER_BANDPASS = 2 };

typedef struct se_synth_params {
    uint8_t osc1_wave;     /* SE_WAVE_* */
    int8_t osc1_octave;    /* -3..3 */
    int8_t osc1_semitones; /* -12..12 */
    uint8_t osc2_wave;
    int8_t osc2_octave;
    int8_t osc2_semitones;
    uint8_t filter_mode; /* SE_FILTER_* */
    uint8_t lfo_wave;    /* SE_WAVE_* */
    uint8_t polyphony;   /* 1..16 (1 = mono, legato) */
    uint8_t unison;      /* 1..4 oscillator stacks per voice */
    uint8_t _pad[6];
    float osc1_shape;      /* 0..1: square pulse width 50%->5%, saw<->triangle morph, sine fold */
    float osc1_fine_cents; /* -100..100 */
    float osc1_level;      /* 0..1 */
    float osc2_shape;
    float osc2_fine_cents;
    float osc2_level;
    float cross_fm;    /* 0..1: osc2 -> osc1 linear FM */
    float sub_level;   /* 0..1: sine one octave below osc1 */
    float noise_level; /* 0..1: white noise */
    float filter_cutoff_hz;  /* 20..20000 (at C4 when key tracking is on) */
    float filter_resonance;  /* 0..1 */
    float filter_key_track;  /* 0..1 (1 = one octave per octave around C4) */
    float filter_env_amount; /* -1..1 (x 6 octaves of the filter envelope) */
    float amp_attack, amp_decay, amp_sustain, amp_release;         /* s 0.001..10, 0.001..10, 0..1, 0.001..20 */
    float filter_attack, filter_decay, filter_sustain, filter_release; /* same ranges */
    float lfo_rate_hz;        /* 0.01..20 */
    float lfo_to_pitch_cents; /* 0..1200 */
    float lfo_to_cutoff_oct;  /* 0..6 */
    float lfo_to_amp;         /* 0..1 tremolo depth */
    float glide;               /* portamento seconds 0..2 */
    float unison_detune_cents; /* total spread 0..50 */
    float output;              /* linear 0..2 */
} se_synth_params; /* 128 bytes, align 4 */

typedef struct se_track_desc_v3 {
    uint64_t track_id;
    float gain; /* linear */
    float pan;  /* -1 .. +1, constant-power */
    uint8_t mute;
    uint8_t solo;
    uint8_t instrument; /* != 0: render `notes` through the built-in synth */
    uint8_t _pad[5];
    const se_clip_desc* clips;
    uint32_t clip_count;
    uint32_t note_count;
    const se_note_desc* notes;
    const se_synth_params* synth; /* NULL = defaults */
} se_track_desc_v3; /* 56 bytes, align 8 */

typedef struct se_session_desc_v3 {
    const se_track_desc_v3* tracks;
    uint32_t track_count;
    float master_gain; /* linear */
} se_session_desc_v3; /* 16 bytes, align 8 */

int32_t se_session_publish_v3(se_engine* engine, const se_session_desc_v3* session);

/* Writes the default synth parameters (no-op for NULL). */
void se_synth_params_default(se_synth_params* out);


/* ---- audio input, monitoring and recording (ABI v1 addition) ------------
 * See docs/adr/0002-recording.md and engine/README.md ("Input, monitoring
 * and recording").
 *
 * se_session_publish_v4 is a superset of se_session_publish_v3: every v4
 * track carries the v3 fields (same meaning, same validation) plus its
 * record-arm / input-monitoring state and its device input routing.
 * v1/v2/v3 publishes are equivalent to v4 with arm = monitor = 0 and no
 * input.
 *
 * Input routing: input_mode SE_INPUT_MONO feeds device input channel
 * `input_channel` (0-based) to both sides of the track (then the track's
 * pan law), SE_INPUT_STEREO feeds channels input_channel / input_channel+1
 * to left / right (pan acts as balance, like stereo clips). Channels the
 * device does not have read as silence.
 *  - monitor != 0: the input is heard through the track (summed with its
 *    clips/notes, then gain/pan/mute/solo, meters, master), also while the
 *    transport is stopped.
 *  - arm != 0 and monitor == 0: the input is NOT heard, but the track meter
 *    shows the raw input level (pre-fader), so levels can be set before
 *    recording ("meter input when armed").
 *  - se_record_start captures every track with arm != 0 and an input.
 * Validation: input_mode <= SE_INPUT_STEREO, else SE_ERR_INVALID_ARG. */
enum { SE_INPUT_NONE = 0, SE_INPUT_MONO = 1, SE_INPUT_STEREO = 2 };

typedef struct se_track_desc_v4 {
    uint64_t track_id;
    float gain; /* linear */
    float pan;  /* -1 .. +1, constant-power */
    uint8_t mute;
    uint8_t solo;
    uint8_t instrument;     /* != 0: render `notes` through the built-in synth */
    uint8_t arm;            /* armed for recording */
    uint8_t monitor;        /* input monitoring */
    uint8_t input_mode;     /* SE_INPUT_* */
    uint16_t input_channel; /* first device input channel, 0-based */
    const se_clip_desc* clips;
    uint32_t clip_count;
    uint32_t note_count;
    const se_note_desc* notes;
    const se_synth_params* synth; /* NULL = defaults */
} se_track_desc_v4; /* 56 bytes, align 8 */

typedef struct se_session_desc_v4 {
    const se_track_desc_v4* tracks;
    uint32_t track_count;
    float master_gain; /* linear */
} se_session_desc_v4; /* 16 bytes, align 8 */

int32_t se_session_publish_v4(se_engine* engine, const se_session_desc_v4* session);

/* ---- devices ------------------------------------------------------------
 * Enumeration only reads device properties; it never opens a device (and
 * so never triggers the microphone-permission prompt). */
#define SE_DEVICE_STRING_MAX 256
#define SE_DEVICE_MAX_RATES 16

typedef struct se_device_info {
    char uid[SE_DEVICE_STRING_MAX];  /* stable id (Core Audio device UID), UTF-8 */
    char name[SE_DEVICE_STRING_MAX]; /* display name, UTF-8 (truncated) */
    uint32_t input_channels;
    uint32_t output_channels;
    uint8_t is_default_input;
    uint8_t is_default_output;
    uint8_t _pad[2];
    uint32_t sample_rate_count;
    double sample_rates[SE_DEVICE_MAX_RATES]; /* supported standard nominal rates, ascending */
} se_device_info; /* 656 bytes, align 8 */

/* Writes up to `capacity` devices; *out_count receives the total number of
 * devices (which may exceed capacity). */
int32_t se_device_list(se_device_info* out, size_t capacity, size_t* out_count);

/* Device setup (Settings > Audio). Takes effect at the next se_device_start;
 * if the device is running it is restarted with the new setup.
 *  - output_uid: NULL or "" = system default output.
 *  - input_enabled = 0: output only (no input device is opened, ever).
 *    input_enabled != 0: open input_uid (NULL or "" = system default
 *    input). Input is only ever opened by se_device_start / a running
 *    restart after this was requested; the default engine never opens one.
 *  - sample_rate: device nominal rate; 0 = leave it (output only) or the
 *    engine rate (with input). With an input it must equal the engine rate
 *    (the input path does not resample); output-only it may be any rate the
 *    output device supports (the output unit converts).
 *  - buffer_size: device IO buffer in frames, 0 = device default; else
 *    16..4096 and inside the output device's range.
 * Validation happens first, without opening anything: unknown uid ->
 * SE_ERR_NOT_FOUND, a device without the needed channels, an unsupported
 * rate/buffer or rate/input combination -> SE_ERR_INVALID_ARG.
 * If the output opens but the input does not (e.g. microphone permission
 * denied), output keeps running, the status reports no input and
 * input_error says why; a running restart then returns SE_ERR_DEVICE. */
typedef struct se_device_config {
    const char* output_uid;
    const char* input_uid;
    double sample_rate;
    uint32_t buffer_size;
    uint8_t input_enabled;
    uint8_t _pad[3];
} se_device_config; /* 32 bytes, align 8 */

int32_t se_device_configure(se_engine* engine, const se_device_config* config);

typedef struct se_device_status {
    char output_name[SE_DEVICE_STRING_MAX];
    char input_name[SE_DEVICE_STRING_MAX];  /* "" when no input is open */
    char input_error[SE_DEVICE_STRING_MAX]; /* why the configured input is not open, "" if none */
    uint8_t running;
    uint8_t input_open;
    uint8_t _pad[6];
    double sample_rate;             /* device nominal rate (engine rate when not running) */
    uint32_t buffer_size;           /* device IO buffer, frames */
    uint32_t input_channels;
    uint32_t output_channels;
    uint32_t input_latency_frames;  /* engine frames: device + safety offset + stream + buffer (input side) */
    uint32_t output_latency_frames; /* same, output side. Takes are shifted by input + output. */
    uint32_t _pad2;
} se_device_status; /* 808 bytes, align 8 */

int32_t se_device_status_get(se_engine* engine, se_device_status* out);

/* Silence all device input (monitoring, input meters and recordings get
 * zeros; recordings keep their timing). */
int32_t se_input_set_muted(se_engine* engine, uint8_t muted);

/* ---- recording ----------------------------------------------------------
 * se_record_start(dir): from the next audio block, captures the raw input
 * (pre-fader, pre-mute) of every track that is armed and has an input in
 * the published session, and starts the transport if it is stopped. The
 * set of tracks is fixed at start. The audio thread copies input into
 * preallocated per-track lock-free rings; a writer thread drains them into
 * 32-bit float WAV files in `dir` (created if missing) named
 * rec-<date>-<time>-<n>-t<track_id>-<take>.wav. A ring overrun (writer too
 * slow) drops that block for that track and counts it; the take ends there
 * and a new take starts with the next block that fits, so every take stays
 * sample-aligned.
 *
 * Takes: a take is a run of contiguous playback. A seek or stop ends the
 * current take; the next captured block starts a new take at its position.
 * Pause/play at the same position continues the take. A loop wrap does NOT
 * end the take: recording while looping writes linearly (no take lanes).
 * start_frame = timeline frame of the block's first sample minus the device
 * round-trip latency (input + output latency, se_device_status), so the
 * take lines up with what was heard; 0 compensation for injected input.
 *
 * se_record_stop: stops capturing (the transport keeps its state), waits
 * for the writer, finalizes every file and returns the number of takes,
 * readable with se_record_get_take until the next se_record_start.
 * Errors: start while recording / stop while not, no armed track with an
 * input, or a running device without an open input -> SE_ERR_STATE; dir not
 * creatable -> SE_ERR_IO; non-integer engine rate -> SE_ERR_UNSUPPORTED_FORMAT. */
int32_t se_record_start(se_engine* engine, const char* utf8_dir);
int32_t se_record_stop(se_engine* engine, size_t* out_take_count);

#define SE_PATH_MAX 1024
typedef struct se_recorded_take {
    uint64_t track_id;
    int64_t start_frame; /* timeline frame of the first sample (latency compensated) */
    uint64_t frames;
    uint32_t channels;   /* 1 (mono input) or 2 (stereo input) */
    uint32_t _pad;
    double sample_rate;  /* engine rate */
    char path[SE_PATH_MAX]; /* UTF-8 */
} se_recorded_take; /* 1064 bytes, align 8 */

int32_t se_record_get_take(se_engine* engine, size_t index, se_recorded_take* out);

/* Live view. Cheap, callable from any non-RT thread at UI rate. Every take
 * of the current recording, in start order (frames lag the audio thread by
 * at most one writer period, ~5 ms). *out_count = total takes. */
typedef struct se_live_take {
    uint64_t track_id;
    int64_t start_frame;
    uint64_t frames;
} se_live_take; /* 24 bytes, align 8 */

int32_t se_record_state(se_engine* engine, uint8_t* out_active, se_live_take* out, size_t capacity, size_t* out_count);

/* Min/max pairs (mono mix) of the newest take of `track_id` while
 * recording, from its first frame; resolution 64 frames (frames_per_peak
 * is rounded to whole 64-frame buckets). Count 0 when there is none. */
int32_t se_record_peaks(se_engine* engine, uint64_t track_id, uint32_t frames_per_peak, float* out_minmax,
                        size_t capacity, size_t* out_count);

typedef struct se_record_stats {
    uint64_t overruns;       /* blocks dropped because a ring was full (all tracks) */
    uint64_t dropped_frames; /* frames lost to overruns (sum over tracks) */
    uint64_t write_errors;   /* takes that could not be written completely */
    uint64_t takes;          /* takes started so far */
} se_record_stats; /* 32 bytes */

/* Counters of the current recording, or of the last one after stop
 * (reset by se_record_start). */
int32_t se_record_stats_get(se_engine* engine, se_record_stats* out);

/* se_render_offline with injected device input: `input_interleaved` holds
 * `frames` frames of `input_channels` (1..32) interleaved channels, used
 * exactly like device input (routing, monitoring, meters, recording with 0
 * latency compensation). Same rules as se_render_offline. */
int32_t se_render_offline_with_input(se_engine* engine, const float* input_interleaved, uint32_t input_channels,
                                     float* out_interleaved, uint32_t frames);

#ifdef __cplusplus
}
#endif
#endif /* SAINE_ENGINE_H */
