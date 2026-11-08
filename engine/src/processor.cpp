#include "processor.hpp"

#include "delay_line.hpp"
#include "plugins/plugin_instance.hpp"
#include "rt_util.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>

#if defined(__ARM_NEON) || defined(__ARM_NEON__)
#include <arm_neon.h>
#elif defined(__SSE2__) || defined(_M_X64)
#include <emmintrin.h>
#endif

namespace saine {

RenderContext::RenderContext(uint32_t max_block)
    : max_block_(max_block), track_l_(max_block), track_r_(max_block), master_l_(max_block), master_r_(max_block),
      synth_(max_block), zeros_(max_block, 0.0f), plugin_l_(max_block), plugin_r_(max_block), gain_l_(max_block),
      gain_r_(max_block), auto_a_(max_block), auto_b_(max_block), param_events_(PluginInstance::kMaxParamEvents) {}

void kernel_add_scaled(float* __restrict dst, const float* __restrict src, float gain, uint32_t n) noexcept {
    for (uint32_t i = 0; i < n; ++i) dst[i] += src[i] * gain;
}

void kernel_add_scaled2(float* __restrict dst_l, float* __restrict dst_r, const float* __restrict src_l,
                        const float* __restrict src_r, float gain_l, float gain_r, uint32_t n) noexcept {
    for (uint32_t i = 0; i < n; ++i) {
        dst_l[i] += src_l[i] * gain_l;
        dst_r[i] += src_r[i] * gain_r;
    }
}

void kernel_scale(float* __restrict buf, float gain, uint32_t n) noexcept {
    for (uint32_t i = 0; i < n; ++i) buf[i] *= gain;
}

void kernel_add_mul2(float* __restrict dst_l, float* __restrict dst_r, const float* __restrict src_l,
                     const float* __restrict src_r, const float* __restrict gain_l, const float* __restrict gain_r,
                     uint32_t n) noexcept {
    for (uint32_t i = 0; i < n; ++i) {
        dst_l[i] += src_l[i] * gain_l[i];
        dst_r[i] += src_r[i] * gain_r[i];
    }
}

void kernel_mul(float* __restrict buf, const float* __restrict gain, uint32_t n) noexcept {
    for (uint32_t i = 0; i < n; ++i) buf[i] *= gain[i];
}

namespace {

int64_t floor_mod(int64_t a, int64_t m) noexcept {
    const int64_t r = a % m;
    return r < 0 ? r + m : r;
}

int64_t floor_div(int64_t a, int64_t b) noexcept {
    const int64_t q = a / b;
    return (a % b != 0 && (a < 0) != (b < 0)) ? q - 1 : q;
}

// Grid of plugin parameter events (timeline frames) and the most lane
// points sent per lane and processed segment.
constexpr int64_t kParamGrid = 64;
constexpr uint32_t kMaxLanePoints = 64;

} // namespace

float kernel_peak(const float* __restrict buf, uint32_t n) noexcept {
    // Max-abs reduction. Compilers will not auto-vectorise this without
    // -ffast-math (std::max has NaN-ordering semantics), so use intrinsics.
    uint32_t i = 0;
    float m = 0.0f;
#if defined(__ARM_NEON) || defined(__ARM_NEON__)
    float32x4_t v0 = vdupq_n_f32(0.0f), v1 = vdupq_n_f32(0.0f);
    for (; i + 8 <= n; i += 8) {
        v0 = vmaxq_f32(v0, vabsq_f32(vld1q_f32(buf + i)));
        v1 = vmaxq_f32(v1, vabsq_f32(vld1q_f32(buf + i + 4)));
    }
    m = vmaxvq_f32(vmaxq_f32(v0, v1));
#elif defined(__SSE2__) || defined(_M_X64)
    const __m128 abs_mask = _mm_castsi128_ps(_mm_set1_epi32(0x7fffffff));
    __m128 v0 = _mm_setzero_ps(), v1 = _mm_setzero_ps();
    for (; i + 8 <= n; i += 8) {
        v0 = _mm_max_ps(v0, _mm_and_ps(_mm_loadu_ps(buf + i), abs_mask));
        v1 = _mm_max_ps(v1, _mm_and_ps(_mm_loadu_ps(buf + i + 4), abs_mask));
    }
    alignas(16) float lanes[4];
    _mm_store_ps(lanes, _mm_max_ps(v0, v1));
    m = std::max(std::max(lanes[0], lanes[1]), std::max(lanes[2], lanes[3]));
#endif
    for (; i < n; ++i) m = std::max(m, std::fabs(buf[i]));
    return m;
}

BlockPeaks render_block(const RenderGraph* graph, int64_t position, bool playing, uint32_t frames,
                        RenderContext& ctx, bool update_meters, uint32_t peek_slot,
                        const InstrumentInputs& inst, const InputView& input) noexcept {
    float* const ml = ctx.master_l();
    float* const mr = ctx.master_r();
    std::memset(ml, 0, sizeof(float) * frames);
    std::memset(mr, 0, sizeof(float) * frames);
    if (graph == nullptr || frames == 0) return {};
    const bool live_input = input.count > 0 && graph->uses_input;
    const bool plugins_on = inst.transport != nullptr && graph->has_plugins;
    DelayLine* const* const lines = graph->pdc.need.empty() ? nullptr : inst.lines;
    if (!playing && inst.synths == nullptr && !live_input && !plugins_on && lines == nullptr) return {};
    const auto input_channel = [&](uint32_t c) { return c < input.count ? input.channels[c] : ctx.zeros(); };
    const uint64_t gen = inst.pdc_gen;

    // ---- automation helpers
    // Output offsets [0, frames) seen `delay` samples late -> timeline pieces.
    const auto pieces = [&](uint32_t delay, auto&& f) {
        if (inst.history != nullptr)
            inst.history->pieces(position, playing, frames, delay, f);
        else
            linear_pieces(position, playing, frames, delay, f);
    };
    // dst[i] = map(lane value) for the block (ramps, see auto_fill).
    const auto fill_lane = [&](const AutoLane& lane, uint32_t delay, auto&& map, float* dst) {
        const LaneView v = lane_view(*graph, lane);
        pieces(delay, [&](uint32_t i0, uint32_t i1, int64_t tl0, bool moving) {
            if (moving) {
                auto_fill(v, tl0, i1 - i0, map, dst + i0);
            } else {
                const float g = map(auto_eval(v, tl0));
                for (uint32_t i = i0; i < i1; ++i) dst[i] = g;
            }
        });
    };
    // Parameter events of `count` lanes from `begin` (sorted by offset):
    // the value at the block start (and each timeline piece), every
    // kParamGrid timeline frames and at every point, unchanged values
    // skipped. Returns the event count.
    const auto plugin_events = [&](uint32_t begin, uint32_t count, uint32_t delay) -> uint32_t {
        ParamEvent* const ev = ctx.param_events();
        // (static: MSVC does not see a constexpr local used in a nested lambda, C4189)
        static constexpr uint32_t cap = PluginInstance::kMaxParamEvents;
        uint32_t n = 0;
        for (uint32_t li = begin; li < begin + count; ++li) {
            const AutoLane& lane = graph->lanes[li];
            if (!lane.active) continue;
            const LaneView v = lane_view(*graph, lane);
            uint32_t points_left = kMaxLanePoints;
            bool have_last = false;
            float last = 0.0f;
            const auto emit = [&](uint32_t offset, float value) {
                if (have_last && value == last) return;
                if (n >= cap) return;
                have_last = true;
                last = value;
                const double nv = std::clamp(static_cast<double>(value), 0.0, 1.0);
                ev[n++] = ParamEvent{offset, lane.param_id, lane.native_min + nv * (lane.native_max - lane.native_min)};
            };
            pieces(delay, [&](uint32_t i0, uint32_t i1, int64_t tl0, bool moving) {
                emit(i0, auto_eval(v, tl0));
                if (!moving) return;
                const int64_t tl_end = tl0 + (i1 - i0);
                uint32_t idx = auto_upper(v, tl0);
                int64_t grid = (floor_div(tl0, kParamGrid) + 1) * kParamGrid;
                for (;;) {
                    const bool point_next = idx < v.n && v.p[idx].frame <= grid;
                    const int64_t next = point_next ? v.p[idx].frame : grid;
                    if (next >= tl_end) break;
                    const bool on_grid = next == grid;
                    if (on_grid) grid += kParamGrid;
                    if (point_next) {
                        while (idx < v.n && v.p[idx].frame == next) ++idx;
                        if (!on_grid) {
                            if (points_left == 0) continue; // point budget spent: grid only
                            --points_left;
                        }
                    }
                    emit(i0 + static_cast<uint32_t>(next - tl0), auto_eval(v, next));
                }
            });
        }
        std::sort(ev, ev + n, [](const ParamEvent& a, const ParamEvent& b) { return a.offset < b.offset; });
        return n;
    };

    float* const tl = ctx.track_l();
    float* const tr = ctx.track_r();
    float* const sb = ctx.synth();
    const int64_t block_start = position;
    const int64_t block_end = position + frames;

    const std::size_t track_count = graph->tracks.size();
    for (std::size_t t = 0; t < track_count; ++t) {
        const TrackNode& track = graph->tracks[t];
        Synth* const synth = track.instrument && !track.has_instrument_plugin && inst.synths != nullptr
                                 ? inst.synths[t]
                                 : nullptr;
        PluginInstance* const iplug = plugins_on && track.instrument_plugin_enabled ? track.instrument_plugin : nullptr;
        const bool track_plugins = plugins_on && (track.fx_count > 0 || iplug != nullptr);
        const bool routed = live_input && track.input_mode != SE_INPUT_NONE && (track.monitor || track.arm);
        // Delay compensation lines of this track (null when not needed).
        DelayLine* main_line = nullptr;
        DelayLine* src_line = nullptr;
        bool pending = false; // a line still has output to give
        if (lines != nullptr) {
            main_line = track.pdc_delay != 0 ? lines[t] : nullptr;
            src_line = track.pdc_src_delay != 0 ? lines[graph->pdc.src_index(track_count, t)] : nullptr;
            if (!track.audible) { // not heard: drop what the lines hold (they restart silent)
                if (main_line != nullptr) main_line->discard(frames, gen);
                if (src_line != nullptr) src_line->discard(frames, gen);
                main_line = src_line = nullptr;
            }
            pending = (main_line != nullptr && !main_line->idle(track.pdc_delay, gen)) ||
                      (src_line != nullptr && !src_line->idle(track.pdc_src_delay, gen));
        }
        if (synth == nullptr && !routed && !track_plugins && (!track.audible || !playing) && !pending) {
            if (main_line != nullptr) main_line->discard(frames, gen);
            if (src_line != nullptr) src_line->discard(frames, gen);
            continue;
        }
        // Runs `line` over the track buffer: the buffer's content if any,
        // else silence while the line still has a tail, else just advances.
        bool any = false;
        const auto delay_track = [&](DelayLine* line, uint32_t d) {
            if (any) {
                line->process(tl, tr, frames, d, gen);
            } else if (!line->idle(d, gen)) {
                std::memset(tl, 0, sizeof(float) * frames);
                std::memset(tr, 0, sizeof(float) * frames);
                any = true;
                line->process(tl, tr, frames, d, gen);
            } else {
                line->discard(frames, gen);
            }
        };

        const ClipNode* clip = graph->clips.data() + track.clip_begin;
        const ClipNode* const clip_end = track.audible && playing ? clip + track.clip_count : clip;
        for (; clip != clip_end; ++clip) {
            if (clip->timeline_start >= block_end) break; // sorted by start
            if (clip->timeline_end <= block_start) continue;
            if (!any) {
                std::memset(tl, 0, sizeof(float) * frames);
                std::memset(tr, 0, sizeof(float) * frames);
                any = true;
            }
            const int64_t from = std::max(clip->timeline_start, block_start);
            const int64_t to = std::min(clip->timeline_end, block_end);
            const auto out_off = static_cast<uint32_t>(from - block_start);
            const auto n = static_cast<uint32_t>(to - from);
            const int64_t src = clip->source_offset + (from - clip->timeline_start);
            kernel_add_scaled(tl + out_off, clip->src_l + src, clip->gain, n);
            kernel_add_scaled(tr + out_off, clip->src_r + src, clip->gain, n);
        }
        // Clips line up with the instrument plugin's (latent) output.
        if (src_line != nullptr) delay_track(src_line, track.pdc_src_delay);
        bool synth_out = false;
        if (synth != nullptr && track.synth_lane_count == 0) {
            synth_out = synth->render(graph->notes.data() + track.note_begin, track.note_count,
                                      &graph->synth_params[t], graph->seq, inst.transport_gen, position, frames,
                                      playing, sb);
        } else if (synth != nullptr) {
            // Automated synth parameters: segments ending on the 16-frame
            // timeline grid, each with the lane values at its first frame.
            se_synth_params& sp = ctx.synth_params();
            sp = graph->synth_params[t];
            for (uint32_t i = 0; i < frames;) {
                const int64_t at = playing ? position + i : position;
                const uint32_t n = playing ? std::min<uint32_t>(frames - i, static_cast<uint32_t>(
                                                     Synth::kControlFrames - floor_mod(at, Synth::kControlFrames)))
                                           : frames - i;
                for (uint32_t li = track.synth_lane_begin; li < track.synth_lane_begin + track.synth_lane_count; ++li) {
                    const AutoLane& lane = graph->lanes[li];
                    if (float* f = synth_param_field(sp, lane.synth_param)) *f = auto_eval(lane_view(*graph, lane), at);
                }
                if (synth->render(graph->notes.data() + track.note_begin, track.note_count, &sp, graph->seq,
                                  inst.transport_gen, position + (playing ? i : 0), n, playing, sb + i))
                    synth_out = true;
                else
                    std::memset(sb + i, 0, sizeof(float) * n);
                i += n;
            }
        }
        if (synth_out && track.audible) {
            if (any) {
                kernel_add_scaled(tl, sb, 1.0f, frames);
                kernel_add_scaled(tr, sb, 1.0f, frames);
            } else {
                std::memcpy(tl, sb, sizeof(float) * frames);
                std::memcpy(tr, sb, sizeof(float) * frames);
                any = true;
            }
        }
        if (iplug != nullptr) {
            float* const pl = ctx.plugin_l();
            float* const pr = ctx.plugin_r();
            std::memset(pl, 0, sizeof(float) * frames);
            std::memset(pr, 0, sizeof(float) * frames);
            const uint32_t ne = iplug->feeder().build(graph->notes.data() + track.note_begin, track.note_count,
                                                      graph->seq, inst.transport_gen, position, frames, playing,
                                                      iplug->event_scratch(), PluginInstance::kMaxEvents);
            const uint32_t np = track.iplug_lane_count > 0 ? plugin_events(track.iplug_lane_begin, track.iplug_lane_count, 0) : 0;
            iplug->run(pl, pr, frames, iplug->event_scratch(), ne, *inst.transport, ctx.param_events(), np);
            if (track.audible) {
                if (any) {
                    kernel_add_scaled(tl, pl, 1.0f, frames);
                    kernel_add_scaled(tr, pr, 1.0f, frames);
                } else {
                    std::memcpy(tl, pl, sizeof(float) * frames);
                    std::memcpy(tr, pr, sizeof(float) * frames);
                    any = true;
                }
            }
        }
        // Monitoring input: only the playback source is compensated.
        if (main_line != nullptr && track.pdc_pre) delay_track(main_line, track.pdc_delay);
        if (routed) {
            const float* il = input_channel(track.input_channel);
            const float* ir = track.input_mode == SE_INPUT_STEREO ? input_channel(track.input_channel + 1u) : il;
            if (track.monitor) {
                if (track.audible) {
                    if (any) {
                        kernel_add_scaled(tl, il, 1.0f, frames);
                        kernel_add_scaled(tr, ir, 1.0f, frames);
                    } else {
                        std::memcpy(tl, il, sizeof(float) * frames);
                        std::memcpy(tr, ir, sizeof(float) * frames);
                        any = true;
                    }
                }
            } else if (update_meters) {
                // Armed, not monitoring: the meter shows the raw input level.
                TrackMeter& m = graph->meters[t];
                const float pl = kernel_peak(il, frames);
                const float pr = ir == il ? pl : kernel_peak(ir, frames);
                atomic_store_max(m.peak_l, pl);
                atomic_store_max(m.peak_r, pr);
                atomic_store_max(m.win_l[peek_slot], pl);
                atomic_store_max(m.win_r[peek_slot], pr);
            }
        }
        if (plugins_on && track.fx_count > 0) {
            if (!any) {
                std::memset(tl, 0, sizeof(float) * frames);
                std::memset(tr, 0, sizeof(float) * frames);
                any = true;
            }
            const FxSlot* slot = graph->fx.data() + track.fx_begin;
            for (uint32_t i = 0; i < track.fx_count; ++i, ++slot) {
                DelayLine* const bypass = lines != nullptr && slot->pdc_bypass_delay != 0
                                              ? lines[graph->pdc.slot_index(track_count, track.fx_begin + i)]
                                              : nullptr;
                if (slot->enabled && slot->inst != nullptr) {
                    const uint32_t np = slot->lane_count > 0
                                            ? plugin_events(slot->lane_begin, slot->lane_count,
                                                            lines != nullptr ? slot->auto_delay : 0)
                                            : 0;
                    slot->inst->run(tl, tr, frames, nullptr, 0, *inst.transport, ctx.param_events(), np);
                    if (bypass != nullptr) bypass->discard(frames, gen);
                } else if (bypass != nullptr) {
                    bypass->process(tl, tr, frames, slot->pdc_bypass_delay, gen); // same timing as the plugin
                }
            }
        }
        if (main_line != nullptr && !track.pdc_pre) delay_track(main_line, track.pdc_delay);
        if (!any || !track.audible) continue;

        float meter_gl = std::fabs(track.gain_l);
        float meter_gr = std::fabs(track.gain_r);
        if (track.vol_lane < 0 && track.pan_lane < 0) {
            kernel_add_scaled2(ml, mr, tl, tr, track.gain_l, track.gain_r, frames);
        } else {
            // Automated fader: the lanes replace the static gain / pan.
            const uint32_t d = lines != nullptr ? track.auto_delay : 0;
            float* const gl = ctx.gain_l();
            float* const gr = ctx.gain_r();
            if (track.vol_lane >= 0) {
                fill_lane(graph->lanes[track.vol_lane], d, auto_db_to_gain, gl);
            } else {
                for (uint32_t i = 0; i < frames; ++i) gl[i] = track.gain;
            }
            if (track.pan_lane >= 0) {
                float* const pa = ctx.auto_a();
                float* const pb = ctx.auto_b();
                const AutoLane& lane = graph->lanes[track.pan_lane];
                fill_lane(lane, d, [](float p) { float l = 0, r = 0; constant_power_pan(p, l, r); return l; }, pa);
                fill_lane(lane, d, [](float p) { float l = 0, r = 0; constant_power_pan(p, l, r); return r; }, pb);
                for (uint32_t i = 0; i < frames; ++i) {
                    gr[i] = gl[i] * pb[i];
                    gl[i] *= pa[i];
                }
            } else {
                float pl = 0.0f, pr = 0.0f;
                constant_power_pan(track.pan, pl, pr);
                for (uint32_t i = 0; i < frames; ++i) {
                    gr[i] = gl[i] * pr;
                    gl[i] *= pl;
                }
            }
            kernel_add_mul2(ml, mr, tl, tr, gl, gr, frames);
            meter_gl = kernel_peak(gl, frames);
            meter_gr = kernel_peak(gr, frames);
        }
        if (update_meters) {
            TrackMeter& m = graph->meters[t];
            const float pl = kernel_peak(tl, frames) * meter_gl;
            const float pr = kernel_peak(tr, frames) * meter_gr;
            atomic_store_max(m.peak_l, pl);
            atomic_store_max(m.peak_r, pr);
            atomic_store_max(m.win_l[peek_slot], pl);
            atomic_store_max(m.win_r[peek_slot], pr);
        }
    }

    if (graph->master_vol_lane >= 0) {
        float* const gm = ctx.auto_a();
        fill_lane(graph->lanes[graph->master_vol_lane], lines != nullptr ? graph->pdc.total : 0, auto_db_to_gain, gm);
        kernel_mul(ml, gm, frames);
        kernel_mul(mr, gm, frames);
    } else if (graph->master_gain != 1.0f) {
        kernel_scale(ml, graph->master_gain, frames);
        kernel_scale(mr, graph->master_gain, frames);
    }
    if (!update_meters) return {};
    return {kernel_peak(ml, frames), kernel_peak(mr, frames)};
}

} // namespace saine
