#include "processor.hpp"

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
      synth_(max_block), zeros_(max_block, 0.0f), plugin_l_(max_block), plugin_r_(max_block) {}

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
    if (!playing && inst.synths == nullptr && !live_input && !plugins_on) return {};
    const auto input_channel = [&](uint32_t c) { return c < input.count ? input.channels[c] : ctx.zeros(); };

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
        if (synth == nullptr && !routed && !track_plugins && (!track.audible || !playing)) continue;

        bool any = false;
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
        if (synth != nullptr &&
            synth->render(graph->notes.data() + track.note_begin, track.note_count, &graph->synth_params[t], graph->seq,
                          inst.transport_gen,
                          position, frames, playing, sb) &&
            track.audible) {
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
            iplug->process(pl, pr, frames, iplug->event_scratch(), ne, *inst.transport);
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
            for (uint32_t i = 0; i < track.fx_count; ++i, ++slot)
                if (slot->enabled && slot->inst != nullptr) slot->inst->process(tl, tr, frames, nullptr, 0, *inst.transport);
        }
        if (!any || !track.audible) continue;

        kernel_add_scaled2(ml, mr, tl, tr, track.gain_l, track.gain_r, frames);
        if (update_meters) {
            TrackMeter& m = graph->meters[t];
            const float pl = kernel_peak(tl, frames) * std::fabs(track.gain_l);
            const float pr = kernel_peak(tr, frames) * std::fabs(track.gain_r);
            atomic_store_max(m.peak_l, pl);
            atomic_store_max(m.peak_r, pr);
            atomic_store_max(m.win_l[peek_slot], pl);
            atomic_store_max(m.win_r[peek_slot], pr);
        }
    }

    if (graph->master_gain != 1.0f) {
        kernel_scale(ml, graph->master_gain, frames);
        kernel_scale(mr, graph->master_gain, frames);
    }
    if (!update_meters) return {};
    return {kernel_peak(ml, frames), kernel_peak(mr, frames)};
}

} // namespace saine
