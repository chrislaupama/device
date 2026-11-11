// The four effect slots of the synth-v2 instrument. Each slot runs one of
// eleven effect types (Tape, Distortion, Bitcrusher, Chorus, Flanger,
// Phaser, Filter, Delay, Reverb, Compressor, Widener) on the stereo synth
// output, with a dry/wet mix. Every type's state is allocated in prepare(),
// so switching types on the audio thread never allocates; a type's state
// is cleared when the slot switches to it.
//
// The designs: saturating
// stages use first-order antiderivative anti-aliasing (ADAA), modulated
// delays read with cubic interpolation, the reverb is an 8-line feedback
// delay network.
#pragma once

#include "synth_v2_dsp.hpp"
#include "synth_v2_params.hpp"

#include <cstdint>
#include <memory>

namespace dstudio::synth_v2 {

// Offsets of a slot's parameters from its "type" parameter (the FX 1 block
// layout; FX 2..4 repeat it).
enum FxParam : uint32_t {
    kFxType = 0,
    kFxOn = fx1_on - fx1_type,
    kFxMix = fx1_mix - fx1_type,
    kTapeDrive = fx1_tape_drive - fx1_type,
    kTapeWarmth = fx1_tape_warmth - fx1_type,
    kTapeCompression = fx1_tape_compression - fx1_type,
    kTapeNoise = fx1_tape_noise - fx1_type,
    kTapeMode = fx1_tape_mode - fx1_type,
    kTapeOutput = fx1_tape_output - fx1_type,
    kDistDrive = fx1_distortion_drive - fx1_type,
    kDistTone = fx1_distortion_tone - fx1_type,
    kDistBias = fx1_distortion_bias - fx1_type,
    kDistLevel = fx1_distortion_level - fx1_type,
    kDistMode = fx1_distortion_mode - fx1_type,
    kCrushBits = fx1_bitcrusher_bits - fx1_type,
    kCrushDownsample = fx1_bitcrusher_downsample - fx1_type,
    kCrushDither = fx1_bitcrusher_dither - fx1_type,
    kCrushJitter = fx1_bitcrusher_jitter - fx1_type,
    kChorusRate = fx1_chorus_rate - fx1_type,
    kChorusDepth = fx1_chorus_depth - fx1_type,
    kChorusVoices = fx1_chorus_voices - fx1_type,
    kChorusWidth = fx1_chorus_width - fx1_type,
    kChorusFeedback = fx1_chorus_feedback - fx1_type,
    kChorusSync = fx1_chorus_sync - fx1_type,
    kChorusNote = fx1_chorus_note - fx1_type,
    kFlangerRate = fx1_flanger_rate - fx1_type,
    kFlangerDepth = fx1_flanger_depth - fx1_type,
    kFlangerFeedback = fx1_flanger_feedback - fx1_type,
    kFlangerWidth = fx1_flanger_width - fx1_type,
    kFlangerMode = fx1_flanger_mode - fx1_type,
    kFlangerSync = fx1_flanger_sync - fx1_type,
    kFlangerNote = fx1_flanger_note - fx1_type,
    kPhaserRate = fx1_phaser_rate - fx1_type,
    kPhaserDepth = fx1_phaser_depth - fx1_type,
    kPhaserFeedback = fx1_phaser_feedback - fx1_type,
    kPhaserStages = fx1_phaser_stages - fx1_type,
    kPhaserWidth = fx1_phaser_width - fx1_type,
    kPhaserSync = fx1_phaser_sync - fx1_type,
    kPhaserNote = fx1_phaser_note - fx1_type,
    kFiltType = fx1_filter_type - fx1_type,
    kFiltCutoff = fx1_filter_cutoff - fx1_type,
    kFiltResonance = fx1_filter_resonance - fx1_type,
    kFiltDrive = fx1_filter_drive - fx1_type,
    kFiltEnv = fx1_filter_env - fx1_type,
    kFiltLfoAmount = fx1_filter_lfo_amount - fx1_type,
    kFiltLfoRate = fx1_filter_lfo_rate - fx1_type,
    kFiltLfoShape = fx1_filter_lfo_shape - fx1_type,
    kFiltLfoSync = fx1_filter_lfo_sync - fx1_type,
    kFiltLfoNote = fx1_filter_lfo_note - fx1_type,
    kDelaySync = fx1_delay_sync - fx1_type,
    kDelayLink = fx1_delay_link - fx1_type,
    kDelayNoteL = fx1_delay_note_l - fx1_type,
    kDelayNoteR = fx1_delay_note_r - fx1_type,
    kDelayTimeL = fx1_delay_time_l - fx1_type,
    kDelayTimeR = fx1_delay_time_r - fx1_type,
    kDelayFeedback = fx1_delay_feedback - fx1_type,
    kDelayMode = fx1_delay_mode - fx1_type,
    kDelayGrit = fx1_delay_grit - fx1_type,
    kDelayModDepth = fx1_delay_mod_depth - fx1_type,
    kDelayWidth = fx1_delay_width - fx1_type,
    kRevMode = fx1_reverb_mode - fx1_type,
    kRevSize = fx1_reverb_size - fx1_type,
    kRevDecay = fx1_reverb_decay - fx1_type,
    kRevPreDelay = fx1_reverb_pre_delay - fx1_type,
    kRevDamping = fx1_reverb_damping - fx1_type,
    kRevWidth = fx1_reverb_width - fx1_type,
    kRevLowCut = fx1_reverb_low_cut - fx1_type,
    kRevHighCut = fx1_reverb_high_cut - fx1_type,
    kCompThreshold = fx1_compressor_threshold - fx1_type,
    kCompRatio = fx1_compressor_ratio - fx1_type,
    kCompAttack = fx1_compressor_attack - fx1_type,
    kCompRelease = fx1_compressor_release - fx1_type,
    kCompKnee = fx1_compressor_knee - fx1_type,
    kCompMakeup = fx1_compressor_makeup - fx1_type,
    kCompStyle = fx1_compressor_style - fx1_type,
    kWideWidth = fx1_widener_width - fx1_type,
    kWideMid = fx1_widener_mid - fx1_type,
    kWideSide = fx1_widener_side - fx1_type,
    kWideLowCut = fx1_widener_low_cut - fx1_type,
    kFxParamCount = fx2_type - fx1_type,
};

enum FxType : int { kTape, kDistortion, kBitcrusher, kChorus, kFlanger, kPhaser, kFilter, kDelay, kReverb, kCompressor, kWidener, kFxTypeCount };

// Transport facts the tempo-synced effects need for one block.
struct FxClock {
    double tempo_bpm = 120.0;
    double beats = 0.0; // quarter notes at the block start
    bool playing = false;
};

// What a slot shows on its panel picture (written per block).
struct FxTelemetry {
    float gr_db = 0.0f;     // compressor gain reduction (<= 0)
    float phase = 0.0f;     // modulation LFO phase 0..1
    float cutoff_hz = 0.0f; // filter effect: current cutoff
};

class FxSlot {
public:
    FxSlot();
    ~FxSlot();
    FxSlot(const FxSlot&) = delete;
    FxSlot& operator=(const FxSlot&) = delete;

    void prepare(double sample_rate, uint32_t max_block);
    void reset() noexcept;
    // RT. The slot is off: the next enable clears the selected type's state.
    void suspend() noexcept;
    // p: the slot's kFxParamCount plain values (clamped, modulation applied).
    // Processes l/r in place, mixing the effect in by p[kFxMix].
    void process(float* l, float* r, uint32_t n, const float* p, const FxClock& clock) noexcept;
    const FxTelemetry& telemetry() const noexcept { return tel_; }

private:
    struct Impl;
    std::unique_ptr<Impl> d_;
    FxTelemetry tel_;
};

} // namespace dstudio::synth_v2
