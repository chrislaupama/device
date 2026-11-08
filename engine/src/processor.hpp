// The block renderer shared by the device callback, se_render_offline and
// se_export_wav. Real-time safe: no allocation, no locks, no I/O.
#pragma once

#include "render_graph.hpp"

#include <cstdint>
#include <vector>

namespace saine {

// Scratch buffers, preallocated to max_block frames. One context per thread
// that renders (the live/RT context, or an export job's private context).
class RenderContext {
public:
    explicit RenderContext(uint32_t max_block);

    uint32_t max_block() const noexcept { return max_block_; }
    float* master_l() noexcept { return master_l_.data(); }
    float* master_r() noexcept { return master_r_.data(); }
    float* track_l() noexcept { return track_l_.data(); }
    float* track_r() noexcept { return track_r_.data(); }

private:
    uint32_t max_block_;
    std::vector<float> track_l_, track_r_, master_l_, master_r_;
};

struct BlockPeaks {
    float l = 0.0f;
    float r = 0.0f;
};

// Renders `frames` (<= ctx.max_block()) frames starting at timeline frame
// `position` into ctx.master_l()/master_r() (overwritten). `graph` may be
// null (silence). When `playing` is false the output is silence.
// If `update_meters` is set, per-track meters in the graph are raised.
// Returns the master output peaks of the block.
BlockPeaks render_block(const RenderGraph* graph, int64_t position, bool playing, uint32_t frames,
                        RenderContext& ctx, bool update_meters) noexcept;

// ---- kernels (exposed for tests/bench) -------------------------------------
// Plain loops over restrict pointers so clang/gcc auto-vectorise them
// (NEON on arm64, SSE/AVX on x86-64).
void kernel_add_scaled(float* __restrict dst, const float* __restrict src, float gain, uint32_t n) noexcept;
void kernel_add_scaled2(float* __restrict dst_l, float* __restrict dst_r, const float* __restrict src_l,
                        const float* __restrict src_r, float gain_l, float gain_r, uint32_t n) noexcept;
void kernel_scale(float* __restrict buf, float gain, uint32_t n) noexcept;
float kernel_peak(const float* __restrict buf, uint32_t n) noexcept;

} // namespace saine
