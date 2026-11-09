// Parameter ids of the synth-v2 instrument in definition order (the same
// table as saine_core::builtin::synth_v2; the Rust test
// engine_registry_matches_core_definitions checks that they agree).
#pragma once

#include <cstdint>

namespace saine::synth_v2 {

enum P : uint16_t {
    osc1_wave, osc1_shape, osc1_octave, osc1_semi, osc1_fine, osc1_level, osc2_wave, osc2_shape, osc2_octave,
    osc2_semi, osc2_fine, osc2_level, cross_fm, sub_level, sub_wave, noise_level, noise_color, unison_voices,
    unison_detune, unison_spread, filter_type, cutoff, resonance, filter_drive, key_track, filter_env,
    filter_velocity, high_pass, amp_attack, amp_decay, amp_sustain, amp_release, env2_attack, env2_decay,
    env2_sustain, env2_release, amp_velocity, lfo1_shape, lfo1_rate, lfo1_sync, lfo1_note, lfo1_retrigger,
    lfo1_fade, lfo1_depth, lfo2_shape, lfo2_rate, lfo2_sync, lfo2_note, lfo2_retrigger, lfo2_fade, lfo2_depth,
    mod1_source, mod1_target, mod1_amount, mod1_on, mod2_source, mod2_target, mod2_amount, mod2_on, mod3_source,
    mod3_target, mod3_amount, mod3_on, mod4_source, mod4_target, mod4_amount, mod4_on, mod5_source, mod5_target,
    mod5_amount, mod5_on, mod6_source, mod6_target, mod6_amount, mod6_on, mod7_source, mod7_target, mod7_amount,
    mod7_on, mod8_source, mod8_target, mod8_amount, mod8_on, mod9_source, mod9_target, mod9_amount, mod9_on,
    mod10_source, mod10_target, mod10_amount, mod10_on, mod11_source, mod11_target, mod11_amount, mod11_on,
    mod12_source, mod12_target, mod12_amount, mod12_on, mod13_source, mod13_target, mod13_amount, mod13_on,
    mod14_source, mod14_target, mod14_amount, mod14_on, mod15_source, mod15_target, mod15_amount, mod15_on,
    mod16_source, mod16_target, mod16_amount, mod16_on, mod17_source, mod17_target, mod17_amount, mod17_on,
    mod18_source, mod18_target, mod18_amount, mod18_on, mod19_source, mod19_target, mod19_amount, mod19_on,
    mod20_source, mod20_target, mod20_amount, mod20_on, mod21_source, mod21_target, mod21_amount, mod21_on,
    mod22_source, mod22_target, mod22_amount, mod22_on, mod23_source, mod23_target, mod23_amount, mod23_on,
    mod24_source, mod24_target, mod24_amount, mod24_on, mod25_source, mod25_target, mod25_amount, mod25_on,
    mod26_source, mod26_target, mod26_amount, mod26_on, mod27_source, mod27_target, mod27_amount, mod27_on,
    mod28_source, mod28_target, mod28_amount, mod28_on, mod29_source, mod29_target, mod29_amount, mod29_on,
    mod30_source, mod30_target, mod30_amount, mod30_on, mod31_source, mod31_target, mod31_amount, mod31_on,
    mod32_source, mod32_target, mod32_amount, mod32_on, voice_mode, glide, bend_range, drift, fx1_type, fx1_on,
    fx1_mix, fx1_tape_drive, fx1_tape_warmth, fx1_tape_compression, fx1_tape_noise, fx1_tape_mode, fx1_tape_output,
    fx1_distortion_drive, fx1_distortion_tone, fx1_distortion_bias, fx1_distortion_level, fx1_distortion_mode,
    fx1_bitcrusher_bits, fx1_bitcrusher_downsample, fx1_bitcrusher_dither, fx1_bitcrusher_jitter, fx1_chorus_rate,
    fx1_chorus_depth, fx1_chorus_voices, fx1_chorus_width, fx1_chorus_feedback, fx1_chorus_sync, fx1_chorus_note,
    fx1_flanger_rate, fx1_flanger_depth, fx1_flanger_feedback, fx1_flanger_width, fx1_flanger_mode,
    fx1_flanger_sync, fx1_flanger_note, fx1_phaser_rate, fx1_phaser_depth, fx1_phaser_feedback, fx1_phaser_stages,
    fx1_phaser_width, fx1_phaser_sync, fx1_phaser_note, fx1_filter_type, fx1_filter_cutoff, fx1_filter_resonance,
    fx1_filter_drive, fx1_filter_env, fx1_filter_lfo_amount, fx1_filter_lfo_rate, fx1_filter_lfo_shape,
    fx1_filter_lfo_sync, fx1_filter_lfo_note, fx1_delay_sync, fx1_delay_link, fx1_delay_note_l, fx1_delay_note_r,
    fx1_delay_time_l, fx1_delay_time_r, fx1_delay_feedback, fx1_delay_mode, fx1_delay_grit, fx1_delay_mod_depth,
    fx1_delay_width, fx1_reverb_mode, fx1_reverb_size, fx1_reverb_decay, fx1_reverb_pre_delay, fx1_reverb_damping,
    fx1_reverb_width, fx1_reverb_low_cut, fx1_reverb_high_cut, fx1_compressor_threshold, fx1_compressor_ratio,
    fx1_compressor_attack, fx1_compressor_release, fx1_compressor_knee, fx1_compressor_makeup, fx1_compressor_style,
    fx1_widener_width, fx1_widener_mid, fx1_widener_side, fx1_widener_low_cut, fx2_type, fx2_on, fx2_mix,
    fx2_tape_drive, fx2_tape_warmth, fx2_tape_compression, fx2_tape_noise, fx2_tape_mode, fx2_tape_output,
    fx2_distortion_drive, fx2_distortion_tone, fx2_distortion_bias, fx2_distortion_level, fx2_distortion_mode,
    fx2_bitcrusher_bits, fx2_bitcrusher_downsample, fx2_bitcrusher_dither, fx2_bitcrusher_jitter, fx2_chorus_rate,
    fx2_chorus_depth, fx2_chorus_voices, fx2_chorus_width, fx2_chorus_feedback, fx2_chorus_sync, fx2_chorus_note,
    fx2_flanger_rate, fx2_flanger_depth, fx2_flanger_feedback, fx2_flanger_width, fx2_flanger_mode,
    fx2_flanger_sync, fx2_flanger_note, fx2_phaser_rate, fx2_phaser_depth, fx2_phaser_feedback, fx2_phaser_stages,
    fx2_phaser_width, fx2_phaser_sync, fx2_phaser_note, fx2_filter_type, fx2_filter_cutoff, fx2_filter_resonance,
    fx2_filter_drive, fx2_filter_env, fx2_filter_lfo_amount, fx2_filter_lfo_rate, fx2_filter_lfo_shape,
    fx2_filter_lfo_sync, fx2_filter_lfo_note, fx2_delay_sync, fx2_delay_link, fx2_delay_note_l, fx2_delay_note_r,
    fx2_delay_time_l, fx2_delay_time_r, fx2_delay_feedback, fx2_delay_mode, fx2_delay_grit, fx2_delay_mod_depth,
    fx2_delay_width, fx2_reverb_mode, fx2_reverb_size, fx2_reverb_decay, fx2_reverb_pre_delay, fx2_reverb_damping,
    fx2_reverb_width, fx2_reverb_low_cut, fx2_reverb_high_cut, fx2_compressor_threshold, fx2_compressor_ratio,
    fx2_compressor_attack, fx2_compressor_release, fx2_compressor_knee, fx2_compressor_makeup, fx2_compressor_style,
    fx2_widener_width, fx2_widener_mid, fx2_widener_side, fx2_widener_low_cut, fx3_type, fx3_on, fx3_mix,
    fx3_tape_drive, fx3_tape_warmth, fx3_tape_compression, fx3_tape_noise, fx3_tape_mode, fx3_tape_output,
    fx3_distortion_drive, fx3_distortion_tone, fx3_distortion_bias, fx3_distortion_level, fx3_distortion_mode,
    fx3_bitcrusher_bits, fx3_bitcrusher_downsample, fx3_bitcrusher_dither, fx3_bitcrusher_jitter, fx3_chorus_rate,
    fx3_chorus_depth, fx3_chorus_voices, fx3_chorus_width, fx3_chorus_feedback, fx3_chorus_sync, fx3_chorus_note,
    fx3_flanger_rate, fx3_flanger_depth, fx3_flanger_feedback, fx3_flanger_width, fx3_flanger_mode,
    fx3_flanger_sync, fx3_flanger_note, fx3_phaser_rate, fx3_phaser_depth, fx3_phaser_feedback, fx3_phaser_stages,
    fx3_phaser_width, fx3_phaser_sync, fx3_phaser_note, fx3_filter_type, fx3_filter_cutoff, fx3_filter_resonance,
    fx3_filter_drive, fx3_filter_env, fx3_filter_lfo_amount, fx3_filter_lfo_rate, fx3_filter_lfo_shape,
    fx3_filter_lfo_sync, fx3_filter_lfo_note, fx3_delay_sync, fx3_delay_link, fx3_delay_note_l, fx3_delay_note_r,
    fx3_delay_time_l, fx3_delay_time_r, fx3_delay_feedback, fx3_delay_mode, fx3_delay_grit, fx3_delay_mod_depth,
    fx3_delay_width, fx3_reverb_mode, fx3_reverb_size, fx3_reverb_decay, fx3_reverb_pre_delay, fx3_reverb_damping,
    fx3_reverb_width, fx3_reverb_low_cut, fx3_reverb_high_cut, fx3_compressor_threshold, fx3_compressor_ratio,
    fx3_compressor_attack, fx3_compressor_release, fx3_compressor_knee, fx3_compressor_makeup, fx3_compressor_style,
    fx3_widener_width, fx3_widener_mid, fx3_widener_side, fx3_widener_low_cut, fx4_type, fx4_on, fx4_mix,
    fx4_tape_drive, fx4_tape_warmth, fx4_tape_compression, fx4_tape_noise, fx4_tape_mode, fx4_tape_output,
    fx4_distortion_drive, fx4_distortion_tone, fx4_distortion_bias, fx4_distortion_level, fx4_distortion_mode,
    fx4_bitcrusher_bits, fx4_bitcrusher_downsample, fx4_bitcrusher_dither, fx4_bitcrusher_jitter, fx4_chorus_rate,
    fx4_chorus_depth, fx4_chorus_voices, fx4_chorus_width, fx4_chorus_feedback, fx4_chorus_sync, fx4_chorus_note,
    fx4_flanger_rate, fx4_flanger_depth, fx4_flanger_feedback, fx4_flanger_width, fx4_flanger_mode,
    fx4_flanger_sync, fx4_flanger_note, fx4_phaser_rate, fx4_phaser_depth, fx4_phaser_feedback, fx4_phaser_stages,
    fx4_phaser_width, fx4_phaser_sync, fx4_phaser_note, fx4_filter_type, fx4_filter_cutoff, fx4_filter_resonance,
    fx4_filter_drive, fx4_filter_env, fx4_filter_lfo_amount, fx4_filter_lfo_rate, fx4_filter_lfo_shape,
    fx4_filter_lfo_sync, fx4_filter_lfo_note, fx4_delay_sync, fx4_delay_link, fx4_delay_note_l, fx4_delay_note_r,
    fx4_delay_time_l, fx4_delay_time_r, fx4_delay_feedback, fx4_delay_mode, fx4_delay_grit, fx4_delay_mod_depth,
    fx4_delay_width, fx4_reverb_mode, fx4_reverb_size, fx4_reverb_decay, fx4_reverb_pre_delay, fx4_reverb_damping,
    fx4_reverb_width, fx4_reverb_low_cut, fx4_reverb_high_cut, fx4_compressor_threshold, fx4_compressor_ratio,
    fx4_compressor_attack, fx4_compressor_release, fx4_compressor_knee, fx4_compressor_makeup, fx4_compressor_style,
    fx4_widener_width, fx4_widener_mid, fx4_widener_side, fx4_widener_low_cut, volume, mod_wheel, macro1, macro2,
    macro3, macro4, macro5, macro6, macro7, macro8, macro1_name, macro2_name, macro3_name, macro4_name, macro5_name,
    macro6_name, macro7_name, macro8_name, map1_macro, map1_param, map1_from, map1_to, map2_macro, map2_param,
    map2_from, map2_to, map3_macro, map3_param, map3_from, map3_to, map4_macro, map4_param, map4_from, map4_to,
    map5_macro, map5_param, map5_from, map5_to, map6_macro, map6_param, map6_from, map6_to, map7_macro, map7_param,
    map7_from, map7_to, map8_macro, map8_param, map8_from, map8_to, map9_macro, map9_param, map9_from, map9_to,
    map10_macro, map10_param, map10_from, map10_to, map11_macro, map11_param, map11_from, map11_to, map12_macro,
    map12_param, map12_from, map12_to, map13_macro, map13_param, map13_from, map13_to, map14_macro, map14_param,
    map14_from, map14_to, map15_macro, map15_param, map15_from, map15_to, map16_macro, map16_param, map16_from,
    map16_to, preset, noise_type, sub_octave, sub_shape, noise_width, map1_late_param, map2_late_param,
    map3_late_param, map4_late_param, map5_late_param, map6_late_param, map7_late_param, map8_late_param,
    map9_late_param, map10_late_param, map11_late_param, map12_late_param, map13_late_param, map14_late_param,
    map15_late_param, map16_late_param, sub_tune, note_priority, sub_waveform,
    kParamCount
};

inline constexpr const char* kParamIds[kParamCount] = {
    "osc1_wave", "osc1_shape", "osc1_octave", "osc1_semi", "osc1_fine", "osc1_level", "osc2_wave", "osc2_shape",
    "osc2_octave", "osc2_semi", "osc2_fine", "osc2_level", "cross_fm", "sub_level", "sub_wave", "noise_level",
    "noise_color", "unison_voices", "unison_detune", "unison_spread", "filter_type", "cutoff", "resonance",
    "filter_drive", "key_track", "filter_env", "filter_velocity", "high_pass", "amp_attack", "amp_decay",
    "amp_sustain", "amp_release", "env2_attack", "env2_decay", "env2_sustain", "env2_release", "amp_velocity",
    "lfo1_shape", "lfo1_rate", "lfo1_sync", "lfo1_note", "lfo1_retrigger", "lfo1_fade", "lfo1_depth", "lfo2_shape",
    "lfo2_rate", "lfo2_sync", "lfo2_note", "lfo2_retrigger", "lfo2_fade", "lfo2_depth", "mod1_source",
    "mod1_target", "mod1_amount", "mod1_on", "mod2_source", "mod2_target", "mod2_amount", "mod2_on", "mod3_source",
    "mod3_target", "mod3_amount", "mod3_on", "mod4_source", "mod4_target", "mod4_amount", "mod4_on", "mod5_source",
    "mod5_target", "mod5_amount", "mod5_on", "mod6_source", "mod6_target", "mod6_amount", "mod6_on", "mod7_source",
    "mod7_target", "mod7_amount", "mod7_on", "mod8_source", "mod8_target", "mod8_amount", "mod8_on", "mod9_source",
    "mod9_target", "mod9_amount", "mod9_on", "mod10_source", "mod10_target", "mod10_amount", "mod10_on",
    "mod11_source", "mod11_target", "mod11_amount", "mod11_on", "mod12_source", "mod12_target", "mod12_amount",
    "mod12_on", "mod13_source", "mod13_target", "mod13_amount", "mod13_on", "mod14_source", "mod14_target",
    "mod14_amount", "mod14_on", "mod15_source", "mod15_target", "mod15_amount", "mod15_on", "mod16_source",
    "mod16_target", "mod16_amount", "mod16_on", "mod17_source", "mod17_target", "mod17_amount", "mod17_on",
    "mod18_source", "mod18_target", "mod18_amount", "mod18_on", "mod19_source", "mod19_target", "mod19_amount",
    "mod19_on", "mod20_source", "mod20_target", "mod20_amount", "mod20_on", "mod21_source", "mod21_target",
    "mod21_amount", "mod21_on", "mod22_source", "mod22_target", "mod22_amount", "mod22_on", "mod23_source",
    "mod23_target", "mod23_amount", "mod23_on", "mod24_source", "mod24_target", "mod24_amount", "mod24_on",
    "mod25_source", "mod25_target", "mod25_amount", "mod25_on", "mod26_source", "mod26_target", "mod26_amount",
    "mod26_on", "mod27_source", "mod27_target", "mod27_amount", "mod27_on", "mod28_source", "mod28_target",
    "mod28_amount", "mod28_on", "mod29_source", "mod29_target", "mod29_amount", "mod29_on", "mod30_source",
    "mod30_target", "mod30_amount", "mod30_on", "mod31_source", "mod31_target", "mod31_amount", "mod31_on",
    "mod32_source", "mod32_target", "mod32_amount", "mod32_on", "voice_mode", "glide", "bend_range", "drift",
    "fx1_type", "fx1_on", "fx1_mix", "fx1_tape_drive", "fx1_tape_warmth", "fx1_tape_compression", "fx1_tape_noise",
    "fx1_tape_mode", "fx1_tape_output", "fx1_distortion_drive", "fx1_distortion_tone", "fx1_distortion_bias",
    "fx1_distortion_level", "fx1_distortion_mode", "fx1_bitcrusher_bits", "fx1_bitcrusher_downsample",
    "fx1_bitcrusher_dither", "fx1_bitcrusher_jitter", "fx1_chorus_rate", "fx1_chorus_depth", "fx1_chorus_voices",
    "fx1_chorus_width", "fx1_chorus_feedback", "fx1_chorus_sync", "fx1_chorus_note", "fx1_flanger_rate",
    "fx1_flanger_depth", "fx1_flanger_feedback", "fx1_flanger_width", "fx1_flanger_mode", "fx1_flanger_sync",
    "fx1_flanger_note", "fx1_phaser_rate", "fx1_phaser_depth", "fx1_phaser_feedback", "fx1_phaser_stages",
    "fx1_phaser_width", "fx1_phaser_sync", "fx1_phaser_note", "fx1_filter_type", "fx1_filter_cutoff",
    "fx1_filter_resonance", "fx1_filter_drive", "fx1_filter_env", "fx1_filter_lfo_amount", "fx1_filter_lfo_rate",
    "fx1_filter_lfo_shape", "fx1_filter_lfo_sync", "fx1_filter_lfo_note", "fx1_delay_sync", "fx1_delay_link",
    "fx1_delay_note_l", "fx1_delay_note_r", "fx1_delay_time_l", "fx1_delay_time_r", "fx1_delay_feedback",
    "fx1_delay_mode", "fx1_delay_grit", "fx1_delay_mod_depth", "fx1_delay_width", "fx1_reverb_mode",
    "fx1_reverb_size", "fx1_reverb_decay", "fx1_reverb_pre_delay", "fx1_reverb_damping", "fx1_reverb_width",
    "fx1_reverb_low_cut", "fx1_reverb_high_cut", "fx1_compressor_threshold", "fx1_compressor_ratio",
    "fx1_compressor_attack", "fx1_compressor_release", "fx1_compressor_knee", "fx1_compressor_makeup",
    "fx1_compressor_style", "fx1_widener_width", "fx1_widener_mid", "fx1_widener_side", "fx1_widener_low_cut",
    "fx2_type", "fx2_on", "fx2_mix", "fx2_tape_drive", "fx2_tape_warmth", "fx2_tape_compression", "fx2_tape_noise",
    "fx2_tape_mode", "fx2_tape_output", "fx2_distortion_drive", "fx2_distortion_tone", "fx2_distortion_bias",
    "fx2_distortion_level", "fx2_distortion_mode", "fx2_bitcrusher_bits", "fx2_bitcrusher_downsample",
    "fx2_bitcrusher_dither", "fx2_bitcrusher_jitter", "fx2_chorus_rate", "fx2_chorus_depth", "fx2_chorus_voices",
    "fx2_chorus_width", "fx2_chorus_feedback", "fx2_chorus_sync", "fx2_chorus_note", "fx2_flanger_rate",
    "fx2_flanger_depth", "fx2_flanger_feedback", "fx2_flanger_width", "fx2_flanger_mode", "fx2_flanger_sync",
    "fx2_flanger_note", "fx2_phaser_rate", "fx2_phaser_depth", "fx2_phaser_feedback", "fx2_phaser_stages",
    "fx2_phaser_width", "fx2_phaser_sync", "fx2_phaser_note", "fx2_filter_type", "fx2_filter_cutoff",
    "fx2_filter_resonance", "fx2_filter_drive", "fx2_filter_env", "fx2_filter_lfo_amount", "fx2_filter_lfo_rate",
    "fx2_filter_lfo_shape", "fx2_filter_lfo_sync", "fx2_filter_lfo_note", "fx2_delay_sync", "fx2_delay_link",
    "fx2_delay_note_l", "fx2_delay_note_r", "fx2_delay_time_l", "fx2_delay_time_r", "fx2_delay_feedback",
    "fx2_delay_mode", "fx2_delay_grit", "fx2_delay_mod_depth", "fx2_delay_width", "fx2_reverb_mode",
    "fx2_reverb_size", "fx2_reverb_decay", "fx2_reverb_pre_delay", "fx2_reverb_damping", "fx2_reverb_width",
    "fx2_reverb_low_cut", "fx2_reverb_high_cut", "fx2_compressor_threshold", "fx2_compressor_ratio",
    "fx2_compressor_attack", "fx2_compressor_release", "fx2_compressor_knee", "fx2_compressor_makeup",
    "fx2_compressor_style", "fx2_widener_width", "fx2_widener_mid", "fx2_widener_side", "fx2_widener_low_cut",
    "fx3_type", "fx3_on", "fx3_mix", "fx3_tape_drive", "fx3_tape_warmth", "fx3_tape_compression", "fx3_tape_noise",
    "fx3_tape_mode", "fx3_tape_output", "fx3_distortion_drive", "fx3_distortion_tone", "fx3_distortion_bias",
    "fx3_distortion_level", "fx3_distortion_mode", "fx3_bitcrusher_bits", "fx3_bitcrusher_downsample",
    "fx3_bitcrusher_dither", "fx3_bitcrusher_jitter", "fx3_chorus_rate", "fx3_chorus_depth", "fx3_chorus_voices",
    "fx3_chorus_width", "fx3_chorus_feedback", "fx3_chorus_sync", "fx3_chorus_note", "fx3_flanger_rate",
    "fx3_flanger_depth", "fx3_flanger_feedback", "fx3_flanger_width", "fx3_flanger_mode", "fx3_flanger_sync",
    "fx3_flanger_note", "fx3_phaser_rate", "fx3_phaser_depth", "fx3_phaser_feedback", "fx3_phaser_stages",
    "fx3_phaser_width", "fx3_phaser_sync", "fx3_phaser_note", "fx3_filter_type", "fx3_filter_cutoff",
    "fx3_filter_resonance", "fx3_filter_drive", "fx3_filter_env", "fx3_filter_lfo_amount", "fx3_filter_lfo_rate",
    "fx3_filter_lfo_shape", "fx3_filter_lfo_sync", "fx3_filter_lfo_note", "fx3_delay_sync", "fx3_delay_link",
    "fx3_delay_note_l", "fx3_delay_note_r", "fx3_delay_time_l", "fx3_delay_time_r", "fx3_delay_feedback",
    "fx3_delay_mode", "fx3_delay_grit", "fx3_delay_mod_depth", "fx3_delay_width", "fx3_reverb_mode",
    "fx3_reverb_size", "fx3_reverb_decay", "fx3_reverb_pre_delay", "fx3_reverb_damping", "fx3_reverb_width",
    "fx3_reverb_low_cut", "fx3_reverb_high_cut", "fx3_compressor_threshold", "fx3_compressor_ratio",
    "fx3_compressor_attack", "fx3_compressor_release", "fx3_compressor_knee", "fx3_compressor_makeup",
    "fx3_compressor_style", "fx3_widener_width", "fx3_widener_mid", "fx3_widener_side", "fx3_widener_low_cut",
    "fx4_type", "fx4_on", "fx4_mix", "fx4_tape_drive", "fx4_tape_warmth", "fx4_tape_compression", "fx4_tape_noise",
    "fx4_tape_mode", "fx4_tape_output", "fx4_distortion_drive", "fx4_distortion_tone", "fx4_distortion_bias",
    "fx4_distortion_level", "fx4_distortion_mode", "fx4_bitcrusher_bits", "fx4_bitcrusher_downsample",
    "fx4_bitcrusher_dither", "fx4_bitcrusher_jitter", "fx4_chorus_rate", "fx4_chorus_depth", "fx4_chorus_voices",
    "fx4_chorus_width", "fx4_chorus_feedback", "fx4_chorus_sync", "fx4_chorus_note", "fx4_flanger_rate",
    "fx4_flanger_depth", "fx4_flanger_feedback", "fx4_flanger_width", "fx4_flanger_mode", "fx4_flanger_sync",
    "fx4_flanger_note", "fx4_phaser_rate", "fx4_phaser_depth", "fx4_phaser_feedback", "fx4_phaser_stages",
    "fx4_phaser_width", "fx4_phaser_sync", "fx4_phaser_note", "fx4_filter_type", "fx4_filter_cutoff",
    "fx4_filter_resonance", "fx4_filter_drive", "fx4_filter_env", "fx4_filter_lfo_amount", "fx4_filter_lfo_rate",
    "fx4_filter_lfo_shape", "fx4_filter_lfo_sync", "fx4_filter_lfo_note", "fx4_delay_sync", "fx4_delay_link",
    "fx4_delay_note_l", "fx4_delay_note_r", "fx4_delay_time_l", "fx4_delay_time_r", "fx4_delay_feedback",
    "fx4_delay_mode", "fx4_delay_grit", "fx4_delay_mod_depth", "fx4_delay_width", "fx4_reverb_mode",
    "fx4_reverb_size", "fx4_reverb_decay", "fx4_reverb_pre_delay", "fx4_reverb_damping", "fx4_reverb_width",
    "fx4_reverb_low_cut", "fx4_reverb_high_cut", "fx4_compressor_threshold", "fx4_compressor_ratio",
    "fx4_compressor_attack", "fx4_compressor_release", "fx4_compressor_knee", "fx4_compressor_makeup",
    "fx4_compressor_style", "fx4_widener_width", "fx4_widener_mid", "fx4_widener_side", "fx4_widener_low_cut",
    "volume", "mod_wheel", "macro1", "macro2", "macro3", "macro4", "macro5", "macro6", "macro7", "macro8",
    "macro1_name", "macro2_name", "macro3_name", "macro4_name", "macro5_name", "macro6_name", "macro7_name",
    "macro8_name", "map1_macro", "map1_param", "map1_from", "map1_to", "map2_macro", "map2_param", "map2_from",
    "map2_to", "map3_macro", "map3_param", "map3_from", "map3_to", "map4_macro", "map4_param", "map4_from",
    "map4_to", "map5_macro", "map5_param", "map5_from", "map5_to", "map6_macro", "map6_param", "map6_from",
    "map6_to", "map7_macro", "map7_param", "map7_from", "map7_to", "map8_macro", "map8_param", "map8_from",
    "map8_to", "map9_macro", "map9_param", "map9_from", "map9_to", "map10_macro", "map10_param", "map10_from",
    "map10_to", "map11_macro", "map11_param", "map11_from", "map11_to", "map12_macro", "map12_param", "map12_from",
    "map12_to", "map13_macro", "map13_param", "map13_from", "map13_to", "map14_macro", "map14_param", "map14_from",
    "map14_to", "map15_macro", "map15_param", "map15_from", "map15_to", "map16_macro", "map16_param", "map16_from",
    "map16_to", "preset", "noise_type", "sub_octave", "sub_shape", "noise_width", "map1_late_param",
    "map2_late_param", "map3_late_param", "map4_late_param", "map5_late_param", "map6_late_param",
    "map7_late_param", "map8_late_param", "map9_late_param", "map10_late_param", "map11_late_param",
    "map12_late_param", "map13_late_param", "map14_late_param", "map15_late_param", "map16_late_param", "sub_tune",
    "note_priority", "sub_waveform",
};


// Plain range of each parameter: [min, max], taper (0 linear, 1 log),
// linear skew, discrete (choices / toggles: the value is an index).
struct Range {
    float min, max;
    uint8_t log;
    float skew;
    uint8_t discrete;
};

inline constexpr Range kRanges[kParamCount] = {
    {0.0f, 5.0f, 0, 1.0f, 1}, // osc1_wave
    {0.0f, 1.0f, 0, 1.0f, 0}, // osc1_shape
    {0.0f, 4.0f, 0, 1.0f, 1}, // osc1_octave
    {0.0f, 24.0f, 0, 1.0f, 1}, // osc1_semi
    {-50.0f, 50.0f, 0, 1.0f, 0}, // osc1_fine
    {0.0f, 1.0f, 0, 1.0f, 0}, // osc1_level
    {0.0f, 5.0f, 0, 1.0f, 1}, // osc2_wave
    {0.0f, 1.0f, 0, 1.0f, 0}, // osc2_shape
    {0.0f, 4.0f, 0, 1.0f, 1}, // osc2_octave
    {0.0f, 24.0f, 0, 1.0f, 1}, // osc2_semi
    {-50.0f, 50.0f, 0, 1.0f, 0}, // osc2_fine
    {0.0f, 1.0f, 0, 1.0f, 0}, // osc2_level
    {0.0f, 1.0f, 0, 1.0f, 0}, // cross_fm
    {0.0f, 1.0f, 0, 1.0f, 0}, // sub_level
    {0.0f, 2.0f, 0, 1.0f, 1}, // sub_wave
    {0.0f, 1.0f, 0, 1.0f, 0}, // noise_level
    {0.0f, 1.0f, 0, 1.0f, 0}, // noise_color
    {0.0f, 7.0f, 0, 1.0f, 1}, // unison_voices
    {0.0f, 1.0f, 0, 1.0f, 0}, // unison_detune
    {0.0f, 1.0f, 0, 1.0f, 0}, // unison_spread
    {0.0f, 4.0f, 0, 1.0f, 1}, // filter_type
    {20.0f, 20000.0f, 1, 1.0f, 0}, // cutoff
    {0.0f, 1.0f, 0, 1.0f, 0}, // resonance
    {0.0f, 1.0f, 0, 1.0f, 0}, // filter_drive
    {0.0f, 1.0f, 0, 1.0f, 0}, // key_track
    {-1.0f, 1.0f, 0, 1.0f, 0}, // filter_env
    {0.0f, 1.0f, 0, 1.0f, 0}, // filter_velocity
    {10.0f, 2000.0f, 1, 1.0f, 0}, // high_pass
    {0.0005f, 10.0f, 1, 1.0f, 0}, // amp_attack
    {0.005f, 15.0f, 1, 1.0f, 0}, // amp_decay
    {0.0f, 1.0f, 0, 1.0f, 0}, // amp_sustain
    {0.005f, 15.0f, 1, 1.0f, 0}, // amp_release
    {0.0005f, 10.0f, 1, 1.0f, 0}, // env2_attack
    {0.005f, 15.0f, 1, 1.0f, 0}, // env2_decay
    {0.0f, 1.0f, 0, 1.0f, 0}, // env2_sustain
    {0.005f, 15.0f, 1, 1.0f, 0}, // env2_release
    {0.0f, 1.0f, 0, 1.0f, 0}, // amp_velocity
    {0.0f, 6.0f, 0, 1.0f, 1}, // lfo1_shape
    {0.02f, 40.0f, 1, 1.0f, 0}, // lfo1_rate
    {0.0f, 1.0f, 0, 1.0f, 1}, // lfo1_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // lfo1_note
    {0.0f, 1.0f, 0, 1.0f, 1}, // lfo1_retrigger
    {0.0f, 5.0f, 0, 0.5f, 0}, // lfo1_fade
    {0.0f, 1.0f, 0, 1.0f, 0}, // lfo1_depth
    {0.0f, 6.0f, 0, 1.0f, 1}, // lfo2_shape
    {0.02f, 40.0f, 1, 1.0f, 0}, // lfo2_rate
    {0.0f, 1.0f, 0, 1.0f, 1}, // lfo2_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // lfo2_note
    {0.0f, 1.0f, 0, 1.0f, 1}, // lfo2_retrigger
    {0.0f, 5.0f, 0, 0.5f, 0}, // lfo2_fade
    {0.0f, 1.0f, 0, 1.0f, 0}, // lfo2_depth
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod1_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod1_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod1_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod1_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod2_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod2_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod2_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod2_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod3_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod3_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod3_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod3_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod4_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod4_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod4_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod4_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod5_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod5_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod5_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod5_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod6_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod6_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod6_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod6_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod7_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod7_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod7_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod7_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod8_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod8_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod8_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod8_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod9_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod9_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod9_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod9_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod10_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod10_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod10_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod10_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod11_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod11_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod11_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod11_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod12_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod12_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod12_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod12_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod13_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod13_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod13_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod13_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod14_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod14_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod14_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod14_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod15_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod15_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod15_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod15_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod16_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod16_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod16_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod16_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod17_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod17_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod17_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod17_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod18_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod18_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod18_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod18_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod19_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod19_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod19_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod19_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod20_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod20_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod20_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod20_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod21_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod21_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod21_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod21_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod22_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod22_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod22_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod22_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod23_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod23_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod23_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod23_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod24_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod24_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod24_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod24_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod25_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod25_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod25_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod25_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod26_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod26_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod26_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod26_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod27_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod27_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod27_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod27_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod28_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod28_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod28_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod28_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod29_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod29_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod29_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod29_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod30_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod30_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod30_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod30_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod31_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod31_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod31_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod31_on
    {0.0f, 9.0f, 0, 1.0f, 1}, // mod32_source
    {0.0f, 29.0f, 0, 1.0f, 1}, // mod32_target
    {-1.0f, 1.0f, 0, 1.0f, 0}, // mod32_amount
    {0.0f, 1.0f, 0, 1.0f, 1}, // mod32_on
    {0.0f, 2.0f, 0, 1.0f, 1}, // voice_mode
    {0.0f, 2.0f, 0, 0.4f, 0}, // glide
    {0.0f, 24.0f, 0, 1.0f, 1}, // bend_range
    {0.0f, 1.0f, 0, 1.0f, 0}, // drift
    {0.0f, 10.0f, 0, 1.0f, 1}, // fx1_type
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx1_on
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_mix
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_tape_drive
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_tape_warmth
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_tape_compression
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_tape_noise
    {0.0f, 2.0f, 0, 1.0f, 1}, // fx1_tape_mode
    {-12.0f, 12.0f, 0, 1.0f, 0}, // fx1_tape_output
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_distortion_drive
    {200.0f, 20000.0f, 1, 1.0f, 0}, // fx1_distortion_tone
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_distortion_bias
    {-24.0f, 6.0f, 0, 1.0f, 0}, // fx1_distortion_level
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx1_distortion_mode
    {1.0f, 16.0f, 0, 1.0f, 0}, // fx1_bitcrusher_bits
    {1.0f, 100.0f, 1, 1.0f, 0}, // fx1_bitcrusher_downsample
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_bitcrusher_dither
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_bitcrusher_jitter
    {0.1f, 10.0f, 1, 1.0f, 0}, // fx1_chorus_rate
    {0.0f, 0.008f, 0, 1.0f, 0}, // fx1_chorus_depth
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx1_chorus_voices
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_chorus_width
    {0.0f, 0.9f, 0, 1.0f, 0}, // fx1_chorus_feedback
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx1_chorus_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx1_chorus_note
    {0.01f, 10.0f, 1, 1.0f, 0}, // fx1_flanger_rate
    {0.0f, 0.01f, 0, 1.0f, 0}, // fx1_flanger_depth
    {-0.9f, 0.9f, 0, 1.0f, 0}, // fx1_flanger_feedback
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_flanger_width
    {0.0f, 2.0f, 0, 1.0f, 1}, // fx1_flanger_mode
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx1_flanger_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx1_flanger_note
    {0.01f, 10.0f, 1, 1.0f, 0}, // fx1_phaser_rate
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_phaser_depth
    {-0.95f, 0.95f, 0, 1.0f, 0}, // fx1_phaser_feedback
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx1_phaser_stages
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_phaser_width
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx1_phaser_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx1_phaser_note
    {0.0f, 7.0f, 0, 1.0f, 1}, // fx1_filter_type
    {20.0f, 20000.0f, 1, 1.0f, 0}, // fx1_filter_cutoff
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_filter_resonance
    {0.0f, 24.0f, 0, 1.0f, 0}, // fx1_filter_drive
    {-72.0f, 72.0f, 0, 1.0f, 0}, // fx1_filter_env
    {0.0f, 72.0f, 0, 1.0f, 0}, // fx1_filter_lfo_amount
    {0.01f, 30.0f, 1, 1.0f, 0}, // fx1_filter_lfo_rate
    {0.0f, 5.0f, 0, 1.0f, 1}, // fx1_filter_lfo_shape
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx1_filter_lfo_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx1_filter_lfo_note
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx1_delay_sync
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx1_delay_link
    {0.0f, 12.0f, 0, 1.0f, 1}, // fx1_delay_note_l
    {0.0f, 12.0f, 0, 1.0f, 1}, // fx1_delay_note_r
    {0.001f, 4.0f, 1, 1.0f, 0}, // fx1_delay_time_l
    {0.001f, 4.0f, 1, 1.0f, 0}, // fx1_delay_time_r
    {0.0f, 0.95f, 0, 1.0f, 0}, // fx1_delay_feedback
    {0.0f, 2.0f, 0, 1.0f, 1}, // fx1_delay_mode
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_delay_grit
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_delay_mod_depth
    {0.0f, 2.0f, 0, 1.0f, 0}, // fx1_delay_width
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx1_reverb_mode
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_reverb_size
    {0.1f, 10.0f, 1, 1.0f, 0}, // fx1_reverb_decay
    {0.0f, 0.1f, 0, 1.0f, 0}, // fx1_reverb_pre_delay
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_reverb_damping
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx1_reverb_width
    {20.0f, 2000.0f, 1, 1.0f, 0}, // fx1_reverb_low_cut
    {1000.0f, 20000.0f, 1, 1.0f, 0}, // fx1_reverb_high_cut
    {-60.0f, 0.0f, 0, 1.0f, 0}, // fx1_compressor_threshold
    {1.0f, 20.0f, 1, 1.0f, 0}, // fx1_compressor_ratio
    {0.001f, 1.0f, 1, 1.0f, 0}, // fx1_compressor_attack
    {0.01f, 3.0f, 1, 1.0f, 0}, // fx1_compressor_release
    {0.0f, 40.0f, 0, 1.0f, 0}, // fx1_compressor_knee
    {-12.0f, 24.0f, 0, 1.0f, 0}, // fx1_compressor_makeup
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx1_compressor_style
    {0.0f, 2.0f, 0, 1.0f, 0}, // fx1_widener_width
    {-12.0f, 12.0f, 0, 1.0f, 0}, // fx1_widener_mid
    {-12.0f, 12.0f, 0, 1.0f, 0}, // fx1_widener_side
    {20.0f, 500.0f, 1, 1.0f, 0}, // fx1_widener_low_cut
    {0.0f, 10.0f, 0, 1.0f, 1}, // fx2_type
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx2_on
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_mix
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_tape_drive
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_tape_warmth
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_tape_compression
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_tape_noise
    {0.0f, 2.0f, 0, 1.0f, 1}, // fx2_tape_mode
    {-12.0f, 12.0f, 0, 1.0f, 0}, // fx2_tape_output
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_distortion_drive
    {200.0f, 20000.0f, 1, 1.0f, 0}, // fx2_distortion_tone
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_distortion_bias
    {-24.0f, 6.0f, 0, 1.0f, 0}, // fx2_distortion_level
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx2_distortion_mode
    {1.0f, 16.0f, 0, 1.0f, 0}, // fx2_bitcrusher_bits
    {1.0f, 100.0f, 1, 1.0f, 0}, // fx2_bitcrusher_downsample
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_bitcrusher_dither
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_bitcrusher_jitter
    {0.1f, 10.0f, 1, 1.0f, 0}, // fx2_chorus_rate
    {0.0f, 0.008f, 0, 1.0f, 0}, // fx2_chorus_depth
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx2_chorus_voices
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_chorus_width
    {0.0f, 0.9f, 0, 1.0f, 0}, // fx2_chorus_feedback
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx2_chorus_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx2_chorus_note
    {0.01f, 10.0f, 1, 1.0f, 0}, // fx2_flanger_rate
    {0.0f, 0.01f, 0, 1.0f, 0}, // fx2_flanger_depth
    {-0.9f, 0.9f, 0, 1.0f, 0}, // fx2_flanger_feedback
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_flanger_width
    {0.0f, 2.0f, 0, 1.0f, 1}, // fx2_flanger_mode
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx2_flanger_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx2_flanger_note
    {0.01f, 10.0f, 1, 1.0f, 0}, // fx2_phaser_rate
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_phaser_depth
    {-0.95f, 0.95f, 0, 1.0f, 0}, // fx2_phaser_feedback
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx2_phaser_stages
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_phaser_width
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx2_phaser_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx2_phaser_note
    {0.0f, 7.0f, 0, 1.0f, 1}, // fx2_filter_type
    {20.0f, 20000.0f, 1, 1.0f, 0}, // fx2_filter_cutoff
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_filter_resonance
    {0.0f, 24.0f, 0, 1.0f, 0}, // fx2_filter_drive
    {-72.0f, 72.0f, 0, 1.0f, 0}, // fx2_filter_env
    {0.0f, 72.0f, 0, 1.0f, 0}, // fx2_filter_lfo_amount
    {0.01f, 30.0f, 1, 1.0f, 0}, // fx2_filter_lfo_rate
    {0.0f, 5.0f, 0, 1.0f, 1}, // fx2_filter_lfo_shape
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx2_filter_lfo_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx2_filter_lfo_note
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx2_delay_sync
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx2_delay_link
    {0.0f, 12.0f, 0, 1.0f, 1}, // fx2_delay_note_l
    {0.0f, 12.0f, 0, 1.0f, 1}, // fx2_delay_note_r
    {0.001f, 4.0f, 1, 1.0f, 0}, // fx2_delay_time_l
    {0.001f, 4.0f, 1, 1.0f, 0}, // fx2_delay_time_r
    {0.0f, 0.95f, 0, 1.0f, 0}, // fx2_delay_feedback
    {0.0f, 2.0f, 0, 1.0f, 1}, // fx2_delay_mode
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_delay_grit
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_delay_mod_depth
    {0.0f, 2.0f, 0, 1.0f, 0}, // fx2_delay_width
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx2_reverb_mode
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_reverb_size
    {0.1f, 10.0f, 1, 1.0f, 0}, // fx2_reverb_decay
    {0.0f, 0.1f, 0, 1.0f, 0}, // fx2_reverb_pre_delay
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_reverb_damping
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx2_reverb_width
    {20.0f, 2000.0f, 1, 1.0f, 0}, // fx2_reverb_low_cut
    {1000.0f, 20000.0f, 1, 1.0f, 0}, // fx2_reverb_high_cut
    {-60.0f, 0.0f, 0, 1.0f, 0}, // fx2_compressor_threshold
    {1.0f, 20.0f, 1, 1.0f, 0}, // fx2_compressor_ratio
    {0.001f, 1.0f, 1, 1.0f, 0}, // fx2_compressor_attack
    {0.01f, 3.0f, 1, 1.0f, 0}, // fx2_compressor_release
    {0.0f, 40.0f, 0, 1.0f, 0}, // fx2_compressor_knee
    {-12.0f, 24.0f, 0, 1.0f, 0}, // fx2_compressor_makeup
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx2_compressor_style
    {0.0f, 2.0f, 0, 1.0f, 0}, // fx2_widener_width
    {-12.0f, 12.0f, 0, 1.0f, 0}, // fx2_widener_mid
    {-12.0f, 12.0f, 0, 1.0f, 0}, // fx2_widener_side
    {20.0f, 500.0f, 1, 1.0f, 0}, // fx2_widener_low_cut
    {0.0f, 10.0f, 0, 1.0f, 1}, // fx3_type
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx3_on
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_mix
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_tape_drive
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_tape_warmth
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_tape_compression
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_tape_noise
    {0.0f, 2.0f, 0, 1.0f, 1}, // fx3_tape_mode
    {-12.0f, 12.0f, 0, 1.0f, 0}, // fx3_tape_output
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_distortion_drive
    {200.0f, 20000.0f, 1, 1.0f, 0}, // fx3_distortion_tone
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_distortion_bias
    {-24.0f, 6.0f, 0, 1.0f, 0}, // fx3_distortion_level
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx3_distortion_mode
    {1.0f, 16.0f, 0, 1.0f, 0}, // fx3_bitcrusher_bits
    {1.0f, 100.0f, 1, 1.0f, 0}, // fx3_bitcrusher_downsample
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_bitcrusher_dither
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_bitcrusher_jitter
    {0.1f, 10.0f, 1, 1.0f, 0}, // fx3_chorus_rate
    {0.0f, 0.008f, 0, 1.0f, 0}, // fx3_chorus_depth
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx3_chorus_voices
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_chorus_width
    {0.0f, 0.9f, 0, 1.0f, 0}, // fx3_chorus_feedback
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx3_chorus_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx3_chorus_note
    {0.01f, 10.0f, 1, 1.0f, 0}, // fx3_flanger_rate
    {0.0f, 0.01f, 0, 1.0f, 0}, // fx3_flanger_depth
    {-0.9f, 0.9f, 0, 1.0f, 0}, // fx3_flanger_feedback
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_flanger_width
    {0.0f, 2.0f, 0, 1.0f, 1}, // fx3_flanger_mode
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx3_flanger_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx3_flanger_note
    {0.01f, 10.0f, 1, 1.0f, 0}, // fx3_phaser_rate
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_phaser_depth
    {-0.95f, 0.95f, 0, 1.0f, 0}, // fx3_phaser_feedback
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx3_phaser_stages
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_phaser_width
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx3_phaser_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx3_phaser_note
    {0.0f, 7.0f, 0, 1.0f, 1}, // fx3_filter_type
    {20.0f, 20000.0f, 1, 1.0f, 0}, // fx3_filter_cutoff
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_filter_resonance
    {0.0f, 24.0f, 0, 1.0f, 0}, // fx3_filter_drive
    {-72.0f, 72.0f, 0, 1.0f, 0}, // fx3_filter_env
    {0.0f, 72.0f, 0, 1.0f, 0}, // fx3_filter_lfo_amount
    {0.01f, 30.0f, 1, 1.0f, 0}, // fx3_filter_lfo_rate
    {0.0f, 5.0f, 0, 1.0f, 1}, // fx3_filter_lfo_shape
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx3_filter_lfo_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx3_filter_lfo_note
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx3_delay_sync
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx3_delay_link
    {0.0f, 12.0f, 0, 1.0f, 1}, // fx3_delay_note_l
    {0.0f, 12.0f, 0, 1.0f, 1}, // fx3_delay_note_r
    {0.001f, 4.0f, 1, 1.0f, 0}, // fx3_delay_time_l
    {0.001f, 4.0f, 1, 1.0f, 0}, // fx3_delay_time_r
    {0.0f, 0.95f, 0, 1.0f, 0}, // fx3_delay_feedback
    {0.0f, 2.0f, 0, 1.0f, 1}, // fx3_delay_mode
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_delay_grit
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_delay_mod_depth
    {0.0f, 2.0f, 0, 1.0f, 0}, // fx3_delay_width
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx3_reverb_mode
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_reverb_size
    {0.1f, 10.0f, 1, 1.0f, 0}, // fx3_reverb_decay
    {0.0f, 0.1f, 0, 1.0f, 0}, // fx3_reverb_pre_delay
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_reverb_damping
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx3_reverb_width
    {20.0f, 2000.0f, 1, 1.0f, 0}, // fx3_reverb_low_cut
    {1000.0f, 20000.0f, 1, 1.0f, 0}, // fx3_reverb_high_cut
    {-60.0f, 0.0f, 0, 1.0f, 0}, // fx3_compressor_threshold
    {1.0f, 20.0f, 1, 1.0f, 0}, // fx3_compressor_ratio
    {0.001f, 1.0f, 1, 1.0f, 0}, // fx3_compressor_attack
    {0.01f, 3.0f, 1, 1.0f, 0}, // fx3_compressor_release
    {0.0f, 40.0f, 0, 1.0f, 0}, // fx3_compressor_knee
    {-12.0f, 24.0f, 0, 1.0f, 0}, // fx3_compressor_makeup
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx3_compressor_style
    {0.0f, 2.0f, 0, 1.0f, 0}, // fx3_widener_width
    {-12.0f, 12.0f, 0, 1.0f, 0}, // fx3_widener_mid
    {-12.0f, 12.0f, 0, 1.0f, 0}, // fx3_widener_side
    {20.0f, 500.0f, 1, 1.0f, 0}, // fx3_widener_low_cut
    {0.0f, 10.0f, 0, 1.0f, 1}, // fx4_type
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx4_on
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_mix
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_tape_drive
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_tape_warmth
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_tape_compression
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_tape_noise
    {0.0f, 2.0f, 0, 1.0f, 1}, // fx4_tape_mode
    {-12.0f, 12.0f, 0, 1.0f, 0}, // fx4_tape_output
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_distortion_drive
    {200.0f, 20000.0f, 1, 1.0f, 0}, // fx4_distortion_tone
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_distortion_bias
    {-24.0f, 6.0f, 0, 1.0f, 0}, // fx4_distortion_level
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx4_distortion_mode
    {1.0f, 16.0f, 0, 1.0f, 0}, // fx4_bitcrusher_bits
    {1.0f, 100.0f, 1, 1.0f, 0}, // fx4_bitcrusher_downsample
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_bitcrusher_dither
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_bitcrusher_jitter
    {0.1f, 10.0f, 1, 1.0f, 0}, // fx4_chorus_rate
    {0.0f, 0.008f, 0, 1.0f, 0}, // fx4_chorus_depth
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx4_chorus_voices
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_chorus_width
    {0.0f, 0.9f, 0, 1.0f, 0}, // fx4_chorus_feedback
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx4_chorus_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx4_chorus_note
    {0.01f, 10.0f, 1, 1.0f, 0}, // fx4_flanger_rate
    {0.0f, 0.01f, 0, 1.0f, 0}, // fx4_flanger_depth
    {-0.9f, 0.9f, 0, 1.0f, 0}, // fx4_flanger_feedback
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_flanger_width
    {0.0f, 2.0f, 0, 1.0f, 1}, // fx4_flanger_mode
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx4_flanger_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx4_flanger_note
    {0.01f, 10.0f, 1, 1.0f, 0}, // fx4_phaser_rate
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_phaser_depth
    {-0.95f, 0.95f, 0, 1.0f, 0}, // fx4_phaser_feedback
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx4_phaser_stages
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_phaser_width
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx4_phaser_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx4_phaser_note
    {0.0f, 7.0f, 0, 1.0f, 1}, // fx4_filter_type
    {20.0f, 20000.0f, 1, 1.0f, 0}, // fx4_filter_cutoff
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_filter_resonance
    {0.0f, 24.0f, 0, 1.0f, 0}, // fx4_filter_drive
    {-72.0f, 72.0f, 0, 1.0f, 0}, // fx4_filter_env
    {0.0f, 72.0f, 0, 1.0f, 0}, // fx4_filter_lfo_amount
    {0.01f, 30.0f, 1, 1.0f, 0}, // fx4_filter_lfo_rate
    {0.0f, 5.0f, 0, 1.0f, 1}, // fx4_filter_lfo_shape
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx4_filter_lfo_sync
    {0.0f, 16.0f, 0, 1.0f, 1}, // fx4_filter_lfo_note
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx4_delay_sync
    {0.0f, 1.0f, 0, 1.0f, 1}, // fx4_delay_link
    {0.0f, 12.0f, 0, 1.0f, 1}, // fx4_delay_note_l
    {0.0f, 12.0f, 0, 1.0f, 1}, // fx4_delay_note_r
    {0.001f, 4.0f, 1, 1.0f, 0}, // fx4_delay_time_l
    {0.001f, 4.0f, 1, 1.0f, 0}, // fx4_delay_time_r
    {0.0f, 0.95f, 0, 1.0f, 0}, // fx4_delay_feedback
    {0.0f, 2.0f, 0, 1.0f, 1}, // fx4_delay_mode
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_delay_grit
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_delay_mod_depth
    {0.0f, 2.0f, 0, 1.0f, 0}, // fx4_delay_width
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx4_reverb_mode
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_reverb_size
    {0.1f, 10.0f, 1, 1.0f, 0}, // fx4_reverb_decay
    {0.0f, 0.1f, 0, 1.0f, 0}, // fx4_reverb_pre_delay
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_reverb_damping
    {0.0f, 1.0f, 0, 1.0f, 0}, // fx4_reverb_width
    {20.0f, 2000.0f, 1, 1.0f, 0}, // fx4_reverb_low_cut
    {1000.0f, 20000.0f, 1, 1.0f, 0}, // fx4_reverb_high_cut
    {-60.0f, 0.0f, 0, 1.0f, 0}, // fx4_compressor_threshold
    {1.0f, 20.0f, 1, 1.0f, 0}, // fx4_compressor_ratio
    {0.001f, 1.0f, 1, 1.0f, 0}, // fx4_compressor_attack
    {0.01f, 3.0f, 1, 1.0f, 0}, // fx4_compressor_release
    {0.0f, 40.0f, 0, 1.0f, 0}, // fx4_compressor_knee
    {-12.0f, 24.0f, 0, 1.0f, 0}, // fx4_compressor_makeup
    {0.0f, 3.0f, 0, 1.0f, 1}, // fx4_compressor_style
    {0.0f, 2.0f, 0, 1.0f, 0}, // fx4_widener_width
    {-12.0f, 12.0f, 0, 1.0f, 0}, // fx4_widener_mid
    {-12.0f, 12.0f, 0, 1.0f, 0}, // fx4_widener_side
    {20.0f, 500.0f, 1, 1.0f, 0}, // fx4_widener_low_cut
    {-36.0f, 12.0f, 0, 1.0f, 0}, // volume
    {0.0f, 1.0f, 0, 1.0f, 0}, // mod_wheel
    {0.0f, 1.0f, 0, 1.0f, 0}, // macro1
    {0.0f, 1.0f, 0, 1.0f, 0}, // macro2
    {0.0f, 1.0f, 0, 1.0f, 0}, // macro3
    {0.0f, 1.0f, 0, 1.0f, 0}, // macro4
    {0.0f, 1.0f, 0, 1.0f, 0}, // macro5
    {0.0f, 1.0f, 0, 1.0f, 0}, // macro6
    {0.0f, 1.0f, 0, 1.0f, 0}, // macro7
    {0.0f, 1.0f, 0, 1.0f, 0}, // macro8
    {0.0f, 40.0f, 0, 1.0f, 1}, // macro1_name
    {0.0f, 40.0f, 0, 1.0f, 1}, // macro2_name
    {0.0f, 40.0f, 0, 1.0f, 1}, // macro3_name
    {0.0f, 40.0f, 0, 1.0f, 1}, // macro4_name
    {0.0f, 40.0f, 0, 1.0f, 1}, // macro5_name
    {0.0f, 40.0f, 0, 1.0f, 1}, // macro6_name
    {0.0f, 40.0f, 0, 1.0f, 1}, // macro7_name
    {0.0f, 40.0f, 0, 1.0f, 1}, // macro8_name
    {0.0f, 8.0f, 0, 1.0f, 1}, // map1_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map1_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map1_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map1_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map2_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map2_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map2_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map2_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map3_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map3_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map3_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map3_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map4_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map4_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map4_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map4_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map5_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map5_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map5_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map5_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map6_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map6_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map6_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map6_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map7_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map7_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map7_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map7_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map8_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map8_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map8_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map8_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map9_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map9_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map9_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map9_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map10_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map10_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map10_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map10_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map11_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map11_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map11_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map11_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map12_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map12_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map12_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map12_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map13_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map13_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map13_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map13_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map14_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map14_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map14_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map14_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map15_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map15_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map15_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map15_to
    {0.0f, 8.0f, 0, 1.0f, 1}, // map16_macro
    {0.0f, 501.0f, 0, 1.0f, 1}, // map16_param
    {0.0f, 1.0f, 0, 1.0f, 0}, // map16_from
    {0.0f, 1.0f, 0, 1.0f, 0}, // map16_to
    {0.0f, 127.0f, 0, 1.0f, 1}, // preset
    {0.0f, 6.0f, 0, 1.0f, 1}, // noise_type
    {0.0f, 1.0f, 0, 1.0f, 1}, // sub_octave
    {0.0f, 1.0f, 0, 1.0f, 0}, // sub_shape
    {0.0f, 1.0f, 0, 1.0f, 0}, // noise_width
    {0.0f, 127.0f, 0, 1.0f, 1}, // map1_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map2_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map3_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map4_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map5_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map6_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map7_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map8_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map9_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map10_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map11_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map12_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map13_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map14_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map15_late_param
    {0.0f, 127.0f, 0, 1.0f, 1}, // map16_late_param
    {0.0f, 48.0f, 0, 1.0f, 1}, // sub_tune
    {0.0f, 2.0f, 0, 1.0f, 1}, // note_priority
    {0.0f, 3.0f, 0, 1.0f, 1}, // sub_waveform
};

// Default values (the core definition's defaults).
inline constexpr float kDefaults[kParamCount] = {
    2.0f, 0.3f, 2.0f, 12.0f, 0.0f, 0.8f, 2.0f, 0.0f, 2.0f, 12.0f, 7.0f, 0.55f, 0.0f, 0.2f, 0.0f, 0.0f, 0.5f, 1.0f,
    0.2f, 0.7f, 0.0f, 1800.0f, 0.22f, 0.2f, 0.5f, 0.3f, 0.3f, 10.0f, 0.003f, 0.9f, 0.7f, 0.35f, 0.002f, 0.55f,
    0.25f, 0.4f, 0.5f, 0.0f, 0.6f, 0.0f, 8.0f, 0.0f, 0.0f, 1.0f, 1.0f, 5.0f, 0.0f, 11.0f, 1.0f, 0.0f, 0.0f, 3.0f,
    4.0f, 0.15f, 1.0f, 4.0f, 1.0f, 0.0125f, 1.0f, 7.0f, 21.0f, 1.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f,
    1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f,
    0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f,
    1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f,
    0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f,
    1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f,
    0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f,
    1.0f, 0.0f, 0.0f, 2.0f, 0.35f, 0.0f, 0.0f, 1.0f, 0.3f, 0.4f, 0.3f, 0.0f, 0.0f, 0.0f, 0.3f, 4000.0f, 0.0f, 0.0f,
    0.0f, 8.0f, 2.0f, 0.0f, 0.0f, 0.5f, 0.004f, 0.0f, 1.0f, 0.0f, 0.0f, 8.0f, 0.3f, 0.003f, 0.6f, 0.5f, 0.0f, 0.0f,
    8.0f, 0.5f, 0.7f, 0.4f, 1.0f, 0.5f, 0.0f, 8.0f, 0.0f, 1200.0f, 0.25f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 8.0f,
    1.0f, 1.0f, 6.0f, 6.0f, 0.5f, 0.5f, 0.35f, 0.0f, 0.0f, 0.0f, 2.0f, 1.0f, 0.5f, 2.0f, 0.01f, 0.5f, 1.0f, 20.0f,
    12000.0f, -18.0f, 4.0f, 0.01f, 0.1f, 6.0f, 0.0f, 0.0f, 1.5f, 0.0f, 0.0f, 120.0f, 3.0f, 1.0f, 0.3333333f, 0.3f,
    0.4f, 0.3f, 0.0f, 0.0f, 0.0f, 0.3f, 4000.0f, 0.0f, 0.0f, 0.0f, 8.0f, 2.0f, 0.0f, 0.0f, 0.5f, 0.004f, 0.0f, 1.0f,
    0.0f, 0.0f, 8.0f, 0.3f, 0.003f, 0.6f, 0.5f, 0.0f, 0.0f, 8.0f, 0.5f, 0.7f, 0.4f, 1.0f, 0.5f, 0.0f, 8.0f, 0.0f,
    1200.0f, 0.25f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 8.0f, 1.0f, 1.0f, 6.0f, 6.0f, 0.5f, 0.5f, 0.35f, 0.0f, 0.0f,
    0.0f, 2.0f, 1.0f, 0.5f, 2.0f, 0.01f, 0.5f, 1.0f, 20.0f, 12000.0f, -18.0f, 4.0f, 0.01f, 0.1f, 6.0f, 0.0f, 0.0f,
    1.5f, 0.0f, 0.0f, 120.0f, 7.0f, 0.0f, 0.2f, 0.3f, 0.4f, 0.3f, 0.0f, 0.0f, 0.0f, 0.3f, 4000.0f, 0.0f, 0.0f, 0.0f,
    8.0f, 2.0f, 0.0f, 0.0f, 0.5f, 0.004f, 0.0f, 1.0f, 0.0f, 0.0f, 8.0f, 0.3f, 0.003f, 0.6f, 0.5f, 0.0f, 0.0f, 8.0f,
    0.5f, 0.7f, 0.4f, 1.0f, 0.5f, 0.0f, 8.0f, 0.0f, 1200.0f, 0.25f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 8.0f, 1.0f,
    1.0f, 6.0f, 6.0f, 0.5f, 0.5f, 0.35f, 0.0f, 0.0f, 0.0f, 2.0f, 1.0f, 0.5f, 2.0f, 0.01f, 0.5f, 1.0f, 20.0f,
    12000.0f, -18.0f, 4.0f, 0.01f, 0.1f, 6.0f, 0.0f, 0.0f, 1.5f, 0.0f, 0.0f, 120.0f, 8.0f, 1.0f, 0.14f, 0.3f, 0.4f,
    0.3f, 0.0f, 0.0f, 0.0f, 0.3f, 4000.0f, 0.0f, 0.0f, 0.0f, 8.0f, 2.0f, 0.0f, 0.0f, 0.5f, 0.004f, 0.0f, 1.0f, 0.0f,
    0.0f, 8.0f, 0.3f, 0.003f, 0.6f, 0.5f, 0.0f, 0.0f, 8.0f, 0.5f, 0.7f, 0.4f, 1.0f, 0.5f, 0.0f, 8.0f, 0.0f, 1200.0f,
    0.25f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 8.0f, 1.0f, 1.0f, 6.0f, 6.0f, 0.5f, 0.5f, 0.35f, 0.0f, 0.0f, 0.0f,
    2.0f, 1.0f, 0.5f, 2.0f, 0.01f, 0.5f, 1.0f, 20.0f, 12000.0f, -18.0f, 4.0f, 0.01f, 0.1f, 6.0f, 0.0f, 0.0f, 1.5f,
    0.0f, 0.0f, 120.0f, -1.5f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f,
    0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f,
    1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f,
    0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f,
    1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f,
    0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 12.0f, 0.0f,
    0.0f,
};

} // namespace saine::synth_v2
