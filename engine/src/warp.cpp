// Audio clip warping: map building, voice slots, rendering, onset detection.
// See warp.hpp and engine/README.md ("Clip warping").
#include "warp.hpp"

#include "devices/sampler_engine.hpp"
#include "devices/spectral.hpp"
#include "render_graph.hpp"

#include "signalsmith-stretch.h"

#include <algorithm>
#include <bit>
#include <cmath>
#include <cstring>
#include <limits>
#include <unordered_set>
#include <utility>

namespace saine {

namespace {

constexpr double kSliceFadeSec = 0.002;  // a slice fades out over its last 2 ms of source
constexpr double kSliceCutSec = 0.002;   // and crossfades for 2 ms where the next slice cuts it
constexpr double kAutoFadeSec = 0.003;   // auto mode: slicing <-> stretching crossfade
constexpr long kStretchSeed = 0x5a1e;    // fixed: renders are reproducible
constexpr int64_t kMaxInputPerBlock = int64_t{1} << 24;

// Last segment whose frame <= t (0 when t precedes them all).
uint32_t segment_at(const WarpSegment* s, uint32_t n, int64_t t) noexcept {
    uint32_t lo = 0, hi = n;
    while (hi - lo > 1) {
        const uint32_t mid = lo + (hi - lo) / 2;
        if (s[mid].frame <= t)
            lo = mid;
        else
            hi = mid;
    }
    return lo;
}

double source_at(const WarpSegment* s, uint32_t n, int64_t t) noexcept {
    const WarpSegment& g = s[segment_at(s, n, t)];
    return g.source + static_cast<double>(t - g.frame) * g.slope;
}

// Inverse map: the (fractional) timeline frame where the map reaches `source`.
double frame_at(const WarpSegment* s, uint32_t n, double source) noexcept {
    uint32_t lo = 0, hi = n;
    while (hi - lo > 1) {
        const uint32_t mid = lo + (hi - lo) / 2;
        if (s[mid].source <= source)
            lo = mid;
        else
            hi = mid;
    }
    return static_cast<double>(s[lo].frame) + (source - s[lo].source) / s[lo].slope;
}

// Last slice whose start <= t, or -1.
int64_t slice_at(const WarpSlice* s, uint32_t n, int64_t t) noexcept {
    if (n == 0 || s[0].start > t) return -1;
    uint32_t lo = 0, hi = n;
    while (hi - lo > 1) {
        const uint32_t mid = lo + (hi - lo) / 2;
        if (s[mid].start <= t)
            lo = mid;
        else
            hi = mid;
    }
    return lo;
}

// Reads the asset with zeros outside it: the input type signalsmith-stretch
// takes (inputs[channel][index]).
struct AssetInput {
    const float* ch[2] = {nullptr, nullptr};
    int64_t frames = 0;
    int64_t base = 0; // source frame of index 0
    struct Channel {
        const float* p;
        int64_t frames;
        int64_t base;
        float operator[](int i) const noexcept {
            const int64_t k = base + i;
            return k >= 0 && k < frames ? p[k] : 0.0f;
        }
    };
    Channel operator[](int c) const noexcept { return Channel{ch[c], frames, base}; }
};

// Slice playback shared by SE_WARP_SLICE and SE_WARP_AUTO.
struct Slicer {
    const sampler::SincTables& tables;
    const ClipNode& clip;
    const WarpSlice* slices;
    uint32_t count;
    double rate;
    int cutoff;
    double fade;     // source frames
    double cut;      // timeline frames

    // One slice's sample at timeline frame t, faded where it runs out.
    bool read(const WarpSlice& s, int64_t t, float& a, float& b) const noexcept {
        const double pos = s.source + static_cast<double>(t - s.start) * rate;
        if (pos >= s.end) return false;
        sampler::read_frame(tables, cutoff, clip.src_l, clip.src_r, clip.source_frames, pos, a, b);
        const double left = s.end - pos;
        if (left < fade) {
            const auto w = static_cast<float>(left / fade);
            a *= w;
            b *= w;
        }
        return true;
    }

    // The slicer's output at t, with slice k = slice_at(t).
    void sample(int64_t k, int64_t t, float& a, float& b) const noexcept {
        a = b = 0.0f;
        if (k < 0) return;
        const WarpSlice& s = slices[k];
        read(s, t, a, b);
        const int64_t into = t - s.start;
        if (k > 0 && static_cast<double>(into) < cut) {
            float pa = 0.0f, pb = 0.0f;
            if (read(slices[k - 1], t, pa, pb)) {
                const auto w = static_cast<float>(1.0 - (static_cast<double>(into) + 1.0) / (cut + 1.0));
                a += pa * w;
                b += pb * w;
            }
        }
    }
};

Slicer make_slicer(const RenderGraph& g, const WarpClip& w, const ClipNode& c, double sample_rate_hint) noexcept {
    const sampler::SincTables& tables = sampler::sinc_tables();
    return Slicer{tables,
                  c,
                  g.warp_slices.data() + w.slice_begin,
                  w.slice_count,
                  w.rate,
                  sampler::cutoff_index(tables, w.rate),
                  kSliceFadeSec * sample_rate_hint,
                  kSliceCutSec * sample_rate_hint};
}

void add_varispeed(const WarpSegment* segs, uint32_t n, const ClipNode& c, int64_t from, int64_t to, float* dl,
                   float* dr) noexcept {
    const sampler::SincTables& tables = sampler::sinc_tables();
    uint32_t k = segment_at(segs, n, from);
    for (int64_t t = from; t < to;) {
        const int64_t end = k + 1 < n ? std::clamp(segs[k + 1].frame, t + 1, to) : to;
        const WarpSegment& s = segs[k];
        const int cutoff = sampler::cutoff_index(tables, s.slope);
        for (; t < end; ++t) {
            float a = 0.0f, b = 0.0f;
            sampler::read_frame(tables, cutoff, c.src_l, c.src_r, c.source_frames,
                                s.source + static_cast<double>(t - s.frame) * s.slope, a, b);
            dl[t - from] += a * c.gain;
            dr[t - from] += b * c.gain;
        }
        if (k + 1 < n) ++k;
    }
}

} // namespace

// ---------------------------------------------------------------- voice

struct WarpVoice::Impl {
    explicit Impl(uint32_t ch) : stretch(kStretchSeed), channels(ch) {}

    signalsmith::stretch::SignalsmithStretch<float> stretch;
    uint32_t channels;
    std::vector<float> out[2];
    // Alignment: the clip and the next timeline frame the stretcher's output
    // stands for, and the source frame one past the last input it was given.
    uint64_t clip_id = 0;
    const float* source = nullptr;
    int64_t next = std::numeric_limits<int64_t>::min();
    int64_t fed = 0;
    float transpose = 0.0f;
    uint8_t mode = 0xff;
    double rate = 48000.0;

    void invalidate() noexcept { next = std::numeric_limits<int64_t>::min(); }

    // Playing this clip, and the next frame it would give is `from`.
    bool aligned(const WarpClip& w, const ClipNode& c, int64_t from) const noexcept {
        return clip_id == w.clip_id && source == c.src_l && next == from;
    }

    void configure_for(const WarpClip& w) noexcept {
        if (w.mode != mode) {
            mode = w.mode;
            stretch.setFormantFactor(1.0f, mode == SE_WARP_STRETCH_FORMANT);
        }
        if (w.transpose != transpose) {
            transpose = w.transpose;
            stretch.setTransposeSemitones(transpose);
        }
    }

    // Restart so that the next output is the stretched audio at timeline frame t.
    void restart(const WarpClip& w, const ClipNode& c, const WarpSegment* segs, uint32_t n, int64_t t) noexcept {
        const double slope = segs[segment_at(segs, n, t)].slope;
        const int len = stretch.outputSeekLength(static_cast<float>(slope));
        const auto base = static_cast<int64_t>(std::llround(source_at(segs, n, t)));
        stretch.outputSeek(AssetInput{{c.src_l, c.src_r}, c.source_frames, base}, len);
        fed = base + len;
        next = t;
        clip_id = w.clip_id;
        source = c.src_l;
    }

    // Stretched frames [from, to) into out[0..channels).
    void process(const WarpClip& w, const ClipNode& c, const WarpSegment* segs, uint32_t n, int64_t from,
                 int64_t to) noexcept {
        configure_for(w);
        if (!aligned(w, c, from)) restart(w, c, segs, n, from);
        // Keep the input inputLatency ahead of the processing time, which runs
        // outputLatency ahead of the output.
        const auto wanted = [&] {
            return std::llround(source_at(segs, n, to + stretch.outputLatency()) + stretch.inputLatency()) - fed;
        };
        int64_t in = wanted();
        // A republished map that moved the source under the voice (a tempo or
        // marker change): restart aligned rather than rush or stall.
        const double expect = segs[segment_at(segs, n, from)].slope * static_cast<double>(to - from);
        if (in < 0 || std::fabs(static_cast<double>(in) - expect) > expect + 0.02 * rate) {
            restart(w, c, segs, n, from);
            in = wanted();
        }
        in = std::clamp<int64_t>(in, 0, kMaxInputPerBlock);
        float* outs[2] = {out[0].data(), out[1].data()};
        stretch.process(AssetInput{{c.src_l, c.src_r}, c.source_frames, fed}, static_cast<int>(in), outs,
                        static_cast<int>(to - from));
        fed += in;
        next = to;
    }
};

WarpVoice::WarpVoice(double sample_rate, uint32_t channels, uint32_t max_block)
    : impl_(std::make_unique<Impl>(std::clamp(channels, 1u, 2u))) {
    Impl& v = *impl_;
    v.rate = sample_rate;
    v.stretch.presetDefault(static_cast<int>(v.channels), static_cast<float>(sample_rate), true);
    v.stretch.setFormantBase(0.0f);
    for (auto& o : v.out) o.assign(max_block, 0.0f);
    sampler::sinc_tables(); // build the interpolation tables off the audio thread
}

WarpVoice::~WarpVoice() = default;

uint32_t WarpVoice::channels() const noexcept { return impl_->channels; }
int WarpVoice::input_latency() const noexcept { return impl_->stretch.inputLatency(); }
int WarpVoice::output_latency() const noexcept { return impl_->stretch.outputLatency(); }

// ---------------------------------------------------------------- graph building

const char* validate_warp(const se_clip_warp_desc& d) noexcept {
    if (d.mode > SE_WARP_AUTO) return "invalid warp mode";
    if (!std::isfinite(d.transpose) || d.transpose < -48.0f || d.transpose > 48.0f)
        return "warp transpose must be finite and within -48..48";
    if (d.point_count < 2) return "a warp map needs at least two points";
    if (d.points == nullptr) return "point_count > 0 but points == NULL";
    if (d.slice_count > 0 && d.slices == nullptr) return "slice_count > 0 but slices == NULL";
    for (uint32_t i = 0; i < d.point_count; ++i) {
        if (!std::isfinite(d.points[i].source)) return "warp point source is not finite";
        if (i > 0 && (d.points[i].frame <= d.points[i - 1].frame || d.points[i].source <= d.points[i - 1].source))
            return "warp points must increase in frame and source";
    }
    for (uint32_t i = 0; i < d.slice_count; ++i) {
        if (!std::isfinite(d.slices[i].source)) return "warp slice source is not finite";
        if (i > 0 && d.slices[i].source <= d.slices[i - 1].source) return "warp slices must increase in source";
    }
    return nullptr;
}

int32_t add_warp(RenderGraph& g, const se_clip_warp_desc& d, const AssetRef& asset, int64_t timeline_start,
                 int64_t length) {
    const auto asset_frames = static_cast<int64_t>(asset->frames);
    WarpClip w;
    w.clip_id = d.clip_id;
    w.asset = asset;
    w.start = timeline_start;
    w.length = length;
    w.mode = d.mode;
    w.channels = static_cast<uint8_t>(std::clamp(asset->channels, 1u, 2u));
    w.transpose = d.transpose;
    w.rate = std::exp2(static_cast<double>(d.transpose) / 12.0);
    w.seg_begin = static_cast<uint32_t>(g.warp_segments.size());
    w.seg_count = d.point_count;
    for (uint32_t i = 0; i < d.point_count; ++i) {
        const se_warp_point& p = d.points[i];
        const se_warp_point& a = d.points[i + 1 < d.point_count ? i : i - 1];
        const se_warp_point& b = d.points[i + 1 < d.point_count ? i + 1 : i];
        g.warp_segments.push_back(
            WarpSegment{p.frame, p.source, (b.source - a.source) / static_cast<double>(b.frame - a.frame)});
    }
    const WarpSegment* segs = g.warp_segments.data() + w.seg_begin;
    w.slice_begin = static_cast<uint32_t>(g.warp_slices.size());
    if (d.mode == SE_WARP_SLICE || d.mode == SE_WARP_AUTO) {
        for (uint32_t i = 0; i < d.slice_count; ++i) {
            const double src = d.slices[i].source;
            if (src >= static_cast<double>(asset_frames)) break;
            const double end = i + 1 < d.slice_count ? std::min(d.slices[i + 1].source, static_cast<double>(asset_frames))
                                                     : static_cast<double>(asset_frames);
            g.warp_slices.push_back(WarpSlice{static_cast<int64_t>(std::llround(frame_at(segs, d.point_count, src))),
                                              src, end, d.slices[i].percussive != 0});
        }
        // Slices whose rounded starts collide keep the later one.
        auto first = g.warp_slices.begin() + w.slice_begin;
        auto last = std::unique(std::make_reverse_iterator(g.warp_slices.end()), std::make_reverse_iterator(first),
                                [](const WarpSlice& x, const WarpSlice& y) { return x.start == y.start; });
        g.warp_slices.erase(first, last.base());
    }
    w.slice_count = static_cast<uint32_t>(g.warp_slices.size()) - w.slice_begin;
    // Without slices, the slicing modes stretch.
    if (w.slice_count == 0 && (d.mode == SE_WARP_SLICE || d.mode == SE_WARP_AUTO)) w.mode = SE_WARP_STRETCH;
    // Render key: FNV-1a over everything that shapes the audio, with timeline
    // frames taken from the clip start.
    uint64_t h = 1469598103934665603ull;
    const auto mix = [&h](const void* data, std::size_t n) {
        const auto* b = static_cast<const unsigned char*>(data);
        for (std::size_t i = 0; i < n; ++i) h = (h ^ b[i]) * 1099511628211ull;
    };
    const auto mix_value = [&mix](auto v) { mix(&v, sizeof v); };
    mix_value(asset->id);
    mix_value(w.mode);
    mix_value(w.transpose);
    mix_value(w.channels);
    mix_value(length);
    for (uint32_t i = 0; i < w.seg_count; ++i) {
        const WarpSegment& s = g.warp_segments[w.seg_begin + i];
        mix_value(s.frame - timeline_start);
        mix_value(s.source);
        mix_value(s.slope);
    }
    for (uint32_t i = 0; i < w.slice_count; ++i) {
        const WarpSlice& s = g.warp_slices[w.slice_begin + i];
        mix_value(s.start - timeline_start);
        mix_value(s.source);
        mix_value(s.end);
        mix_value(s.percussive);
    }
    w.key = h;
    g.warps.push_back(w);
    sampler::sinc_tables();
    return static_cast<int32_t>(g.warps.size() - 1);
}

void assign_warp_voices(RenderGraph& g) {
    g.warp_voice_keys.clear();
    for (const TrackNode& track : g.tracks) {
        // Slot -> (channels, end of the last clip it plays); greedy interval colouring.
        std::vector<std::pair<uint32_t, int64_t>> slots;
        std::vector<int32_t> global;
        for (uint32_t i = track.clip_begin; i < track.clip_begin + track.clip_count; ++i) {
            ClipNode& c = g.clips[i];
            if (c.warp < 0) continue;
            WarpClip& w = g.warps[static_cast<std::size_t>(c.warp)];
            if (!warp_needs_voice(w.mode)) continue;
            std::size_t s = 0;
            while (s < slots.size() && (slots[s].first != w.channels || slots[s].second > c.timeline_start)) ++s;
            if (s == slots.size()) {
                slots.emplace_back(w.channels, 0);
                global.push_back(static_cast<int32_t>(g.warp_voice_keys.size()));
                g.warp_voice_keys.push_back(WarpVoiceKey{track.track_id, static_cast<uint32_t>(s), w.channels});
            }
            slots[s].second = c.timeline_end;
            w.voice = global[s];
        }
    }
}

// ---------------------------------------------------------------- rendering

void render_warped_clip(const RenderGraph& g, const ClipNode& clip, WarpVoice* voice, int64_t from, int64_t to,
                        float* dst_l, float* dst_r) noexcept {
    const WarpClip& w = g.warps[static_cast<std::size_t>(clip.warp)];
    const WarpSegment* segs = g.warp_segments.data() + w.seg_begin;
    const uint32_t n = w.seg_count;
    const double sr = g.sample_rate;
    const auto frames = static_cast<uint32_t>(to - from);

    if (const WarpRender* r = w.rendered.get()) {
        const float* rl = r->left.data() + (from - clip.timeline_start);
        const float* rr = (r->right.empty() ? r->left.data() : r->right.data()) + (from - clip.timeline_start);
        if (voice != nullptr && voice->impl().aligned(w, clip, from)) {
            // The render arrived while the voice played this clip: crossfade
            // from the voice over this block, then the voice is free.
            WarpVoice::Impl& v = voice->impl();
            v.process(w, clip, segs, n, from, to);
            const float* vl = v.out[0].data();
            const float* vr = v.channels > 1 ? v.out[1].data() : vl;
            for (uint32_t i = 0; i < frames; ++i) {
                const float x = (static_cast<float>(i) + 1.0f) / static_cast<float>(frames);
                dst_l[i] += (vl[i] + (rl[i] - vl[i]) * x) * clip.gain;
                dst_r[i] += (vr[i] + (rr[i] - vr[i]) * x) * clip.gain;
            }
            v.invalidate();
            return;
        }
        for (uint32_t i = 0; i < frames; ++i) {
            dst_l[i] += rl[i] * clip.gain;
            dst_r[i] += rr[i] * clip.gain;
        }
        return;
    }

    if (w.mode == SE_WARP_REPITCH || (voice == nullptr && w.mode != SE_WARP_SLICE)) {
        add_varispeed(segs, n, clip, from, to, dst_l, dst_r);
        return;
    }
    if (w.mode == SE_WARP_SLICE) {
        const Slicer sl = make_slicer(g, w, clip, sr);
        int64_t k = slice_at(sl.slices, sl.count, from);
        for (int64_t t = from; t < to; ++t) {
            while (k + 1 < static_cast<int64_t>(sl.count) && sl.slices[k + 1].start <= t) ++k;
            float a = 0.0f, b = 0.0f;
            sl.sample(k, t, a, b);
            dst_l[t - from] += a * clip.gain;
            dst_r[t - from] += b * clip.gain;
        }
        return;
    }

    WarpVoice::Impl& v = voice->impl();
    const float* ol = v.out[0].data();
    const float* orr = v.channels > 1 ? v.out[1].data() : ol;
    if (w.mode != SE_WARP_AUTO) {
        v.process(w, clip, segs, n, from, to);
        for (uint32_t i = 0; i < frames; ++i) {
            dst_l[i] += ol[i] * clip.gain;
            dst_r[i] += orr[i] * clip.gain;
        }
        return;
    }

    // Auto: per slice, slicing (percussive) or stretching, crossfaded.
    const Slicer sl = make_slicer(g, w, clip, sr);
    const double ramp = kAutoFadeSec * sr;
    const auto weight = [&](int64_t k, int64_t t) noexcept {
        if (k < 0) return 0.0;
        const double cur = sl.slices[k].percussive ? 1.0 : 0.0;
        const double prev = k > 0 && sl.slices[k - 1].percussive ? 1.0 : 0.0;
        const double into = static_cast<double>(t - sl.slices[k].start);
        return into >= ramp ? cur : prev + (cur - prev) * (into / ramp);
    };
    const int64_t k0 = slice_at(sl.slices, sl.count, from);
    // Stretch only when some frame of the block needs it: a tonal slice, or
    // the ramp out of one.
    bool stretch = k0 < 0;
    for (int64_t k = std::max<int64_t>(k0, 0); !stretch && k < static_cast<int64_t>(sl.count) &&
                                               (k == k0 || sl.slices[k].start < to);
         ++k) {
        const WarpSlice& s = sl.slices[k];
        stretch = !s.percussive ||
                  (k > 0 && !sl.slices[k - 1].percussive && static_cast<double>(std::max(from, s.start) - s.start) < ramp);
    }
    if (stretch)
        v.process(w, clip, segs, n, from, to);
    else
        v.invalidate(); // restarts aligned when a tonal passage comes
    int64_t k = k0;
    for (int64_t t = from; t < to; ++t) {
        while (k + 1 < static_cast<int64_t>(sl.count) && sl.slices[k + 1].start <= t) ++k;
        const auto ws = static_cast<float>(weight(k, t));
        float a = 0.0f, b = 0.0f;
        if (ws > 0.0f) sl.sample(k, t, a, b);
        const auto i = static_cast<std::size_t>(t - from);
        const float sa = stretch ? ol[i] : 0.0f;
        const float sb = stretch ? orr[i] : 0.0f;
        dst_l[i] += (sa + (a - sa) * ws) * clip.gain;
        dst_r[i] += (sb + (b - sb) * ws) * clip.gain;
    }
}

// ---------------------------------------------------------------- renders

struct WarpRenderer::Job {
    uint64_t key = 0;
    WarpClip warp;                     // seg_begin / slice_begin = 0, voice = 0
    std::vector<WarpSegment> segments; // timeline frames from the clip start
    std::vector<WarpSlice> slices;
};

WarpRenderer::WarpRenderer(double sample_rate, uint32_t max_block)
    : sample_rate_(sample_rate), max_block_(max_block), thread_([this] { run(); }) {}

WarpRenderer::~WarpRenderer() {
    {
        std::lock_guard lock(mutex_);
        stop_ = true;
    }
    cv_.notify_all();
    thread_.join();
}

void WarpRenderer::set_enabled(bool enabled) {
    std::lock_guard lock(mutex_);
    enabled_ = enabled;
}

void WarpRenderer::resolve(RenderGraph& g) {
    std::lock_guard lock(mutex_);
    if (!enabled_) return;
    ++generation_;
    bool queued = false;
    for (WarpClip& w : g.warps) {
        if (!warp_needs_voice(w.mode) || w.length <= 0 || w.length > kMaxFrames || !w.asset) continue;
        last_used_[w.key] = generation_;
        if (auto it = done_.find(w.key); it != done_.end()) {
            w.rendered = it->second;
            continue;
        }
        const bool waiting = std::any_of(queue_.begin(), queue_.end(), [&](const auto& j) { return j->key == w.key; });
        if (waiting || in_flight_ == w.key) continue;
        auto job = std::make_shared<Job>();
        job->key = w.key;
        job->warp = w;
        job->warp.rendered.reset();
        job->warp.seg_begin = 0;
        job->warp.slice_begin = 0;
        job->warp.voice = 0;
        const int64_t start = w.start;
        for (uint32_t i = 0; i < w.seg_count; ++i) {
            WarpSegment s = g.warp_segments[w.seg_begin + i];
            s.frame -= start;
            job->segments.push_back(s);
        }
        for (uint32_t i = 0; i < w.slice_count; ++i) {
            WarpSlice s = g.warp_slices[w.slice_begin + i];
            s.start -= start;
            job->slices.push_back(s);
        }
        queue_.push_back(std::move(job));
        queued = true;
    }
    if (queued) cv_.notify_one();
}

bool WarpRenderer::take_finished() {
    std::lock_guard lock(mutex_);
    return std::exchange(finished_, false);
}

void WarpRenderer::evict(const RenderGraph& current) {
    std::unordered_set<uint64_t> used;
    for (const WarpClip& w : current.warps) used.insert(w.key);
    std::lock_guard lock(mutex_);
    // Queued renders nothing plays any more (a tempo drag publishes many maps).
    std::erase_if(queue_, [&](const auto& j) { return used.find(j->key) == used.end(); });
    std::vector<std::pair<uint64_t, uint64_t>> unused; // (generation, key)
    std::size_t unused_bytes = 0;
    for (const auto& [key, r] : done_)
        if (used.find(key) == used.end()) {
            unused.emplace_back(last_used_[key], key);
            unused_bytes += r->bytes();
        }
    std::sort(unused.begin(), unused.end());
    for (std::size_t i = 0; i < unused.size() && (unused.size() - i > kKeepUnused || unused_bytes > kUnusedBytes); ++i) {
        unused_bytes -= done_[unused[i].second]->bytes();
        done_.erase(unused[i].second);
    }
    // Generations of keys that are neither rendered, queued nor rendering.
    std::erase_if(last_used_, [&](const auto& kv) {
        const uint64_t key = kv.first;
        return done_.find(key) == done_.end() && key != in_flight_ &&
               std::none_of(queue_.begin(), queue_.end(), [&](const auto& j) { return j->key == key; });
    });
}

std::size_t WarpRenderer::pending() const {
    std::lock_guard lock(mutex_);
    return queue_.size() + (in_flight_ != 0 ? 1 : 0);
}

void WarpRenderer::run() {
    constexpr uint32_t kChunk = 4096;
    for (;;) {
        std::shared_ptr<Job> job;
        {
            std::unique_lock lock(mutex_);
            cv_.wait(lock, [&] { return stop_ || !queue_.empty(); });
            if (stop_) return;
            job = std::move(queue_.front());
            queue_.pop_front();
            in_flight_ = job->key;
        }
        // The live code path on a private one-clip graph.
        RenderGraph g;
        g.sample_rate = sample_rate_;
        g.warps.push_back(job->warp);
        g.warp_segments = job->segments;
        g.warp_slices = job->slices;
        const Asset& asset = *job->warp.asset;
        ClipNode c;
        c.src_l = asset.channel(0);
        c.src_r = asset.channel(1);
        c.source_frames = static_cast<int64_t>(asset.frames);
        c.timeline_start = 0;
        c.timeline_end = job->warp.length;
        c.warp = 0;
        auto out = std::make_shared<WarpRender>();
        out->key = job->key;
        out->frames = job->warp.length;
        out->left.assign(static_cast<std::size_t>(out->frames), 0.0f);
        out->right.assign(static_cast<std::size_t>(out->frames), 0.0f);
        WarpVoice voice(sample_rate_, job->warp.channels, std::max(kChunk, max_block_));
        bool abandoned = false;
        for (int64_t t = 0; t < out->frames && !abandoned; t += kChunk) {
            const int64_t to = std::min<int64_t>(out->frames, t + kChunk);
            render_warped_clip(g, c, &voice, t, to, out->left.data() + t, out->right.data() + t);
            std::lock_guard lock(mutex_);
            abandoned = stop_;
        }
        if (job->warp.channels < 2) out->right = {};
        std::lock_guard lock(mutex_);
        in_flight_ = 0;
        if (abandoned) return;
        done_[job->key] = std::move(out);
        finished_ = true;
    }
}

// ---------------------------------------------------------------- onsets

void detect_onsets(const Asset& asset, double sample_rate, std::vector<se_onset>& out) {
    out.clear();
    const auto frames = static_cast<int64_t>(asset.frames);
    const uint32_t fft_n = sample_rate > 64000.0 ? 2048 : 1024;
    const uint32_t hop = fft_n / 4;
    if (frames < static_cast<int64_t>(fft_n)) return;
    const float* l = asset.channel(0);
    const float* r = asset.channel(asset.channels > 1 ? 1 : 0);
    const auto mono = [&](int64_t i) noexcept { return i < 0 || i >= frames ? 0.0f : 0.5f * (l[i] + r[i]); };

    // Spectral flux of log magnitudes against a frequency max-filtered
    // previous frame (vibrato does not read as onsets).
    dsp::RealFft fft;
    fft.prepare(fft_n);
    std::vector<float> win(fft_n), x(fft_n), re(fft_n / 2 + 1), im(fft_n / 2 + 1);
    dsp::fill_window(dsp::Window::Hann, fft_n, win.data());
    const auto bin_of = [&](double hz) { return static_cast<uint32_t>(std::clamp(hz / sample_rate * fft_n, 1.0, fft_n / 2.0)); };
    const uint32_t lo_bin = bin_of(30.0), hi_bin = bin_of(16000.0);
    std::vector<float> mag(fft_n / 2 + 1, 0.0f), prev(fft_n / 2 + 1, 0.0f);
    const auto count = static_cast<std::size_t>((frames + hop - 1) / hop);
    std::vector<float> odf(count, 0.0f);
    const float scale = 4.0f / static_cast<float>(fft_n);
    for (std::size_t j = 0; j < count; ++j) {
        const int64_t centre = static_cast<int64_t>(j) * hop;
        for (uint32_t i = 0; i < fft_n; ++i) x[i] = mono(centre - fft_n / 2 + i) * win[i];
        fft.forward(x.data(), re.data(), im.data());
        float flux = 0.0f;
        for (uint32_t k = lo_bin; k <= hi_bin; ++k) {
            mag[k] = std::log1p(100.0f * scale * std::sqrt(re[k] * re[k] + im[k] * im[k]));
            const float ref = std::max({prev[k - 1], prev[k], prev[std::min(k + 1, fft_n / 2)]});
            flux += std::max(0.0f, mag[k] - ref);
        }
        odf[j] = j == 0 ? 0.0f : flux;
        std::swap(mag, prev);
    }
    const float peak = *std::max_element(odf.begin(), odf.end());
    if (peak <= 0.0f) return;
    for (float& v : odf) v /= peak;

    // Peak picking: local maximum over +-16 ms, above the local mean by a
    // margin, at least 30 ms after the previous onset.
    const auto frames_of = [&](double sec) { return std::max<int64_t>(1, std::llround(sec * sample_rate / hop)); };
    const int64_t w_max = frames_of(0.016), w_pre = frames_of(0.05), w_post = frames_of(0.035), gap = frames_of(0.03);
    const auto n = static_cast<int64_t>(count);
    std::vector<int64_t> picks;
    int64_t last = -gap;
    for (int64_t j = 1; j < n; ++j) {
        const float v = odf[static_cast<std::size_t>(j)];
        if (v < 0.04f || j - last < gap) continue;
        bool is_max = true;
        for (int64_t k = std::max<int64_t>(0, j - w_max); k <= std::min(n - 1, j + w_max) && is_max; ++k)
            if (odf[static_cast<std::size_t>(k)] > v) is_max = false;
        if (!is_max) continue;
        double mean = 0.0;
        int64_t m = 0;
        for (int64_t k = std::max<int64_t>(0, j - w_pre); k <= std::min(n - 1, j + w_post); ++k, ++m)
            mean += static_cast<double>(odf[static_cast<std::size_t>(k)]);
        if (static_cast<double>(v) < mean / static_cast<double>(m) + 0.06) continue;
        picks.push_back(j);
        last = j;
    }

    // Each onset moves to the start of its attack: the 64-frame block with the
    // steepest energy rise (among blocks holding a tenth of the peak energy)
    // in the window that produced the flux peak (the log compression makes
    // an attack register as soon as it enters the window), minus one block.
    constexpr int64_t kBlock = 64;
    const auto energy = [&](int64_t at) noexcept {
        double e = 0.0;
        for (int64_t i = at; i < at + kBlock; ++i) e += static_cast<double>(mono(i)) * static_cast<double>(mono(i));
        return e;
    };
    for (const int64_t j : picks) {
        const int64_t centre = j * hop;
        const int64_t a = std::max<int64_t>(0, centre - fft_n / 2), b = std::min(frames, centre + fft_n / 2 + hop);
        double emax = 0.0;
        for (int64_t at = a; at < b; at += kBlock) emax = std::max(emax, energy(at));
        if (emax <= 0.0) continue;
        int64_t best = centre;
        double best_rise = 0.0, before = energy(a - kBlock);
        for (int64_t at = a; at < b; at += kBlock) {
            const double e = energy(at);
            const double rise = (e + 1e-9) / (before + 1e-9);
            if (e >= 0.1 * emax && rise > best_rise) {
                best_rise = rise;
                best = at;
            }
            before = e;
        }
        const int64_t at = std::max<int64_t>(0, best - kBlock);
        if (!out.empty() && at - out.back().frame < std::llround(0.02 * sample_rate)) continue;
        out.push_back(se_onset{at, odf[static_cast<std::size_t>(j)], 0, {0, 0, 0}});
    }

    // Percussive: what follows the attack (the last 40 % of the stretch to
    // the next onset, looking at most 300 ms ahead) is at least 12 dB under
    // the attack; voted over the two neighbours on each side.
    const auto rms = [&](int64_t a, int64_t b) noexcept {
        double e = 0.0;
        for (int64_t i = a; i < b; ++i) e += static_cast<double>(mono(i)) * static_cast<double>(mono(i));
        return b > a ? std::sqrt(e / static_cast<double>(b - a)) : 0.0;
    };
    const int64_t head_max = std::llround(0.03 * sample_rate), look = std::llround(0.3 * sample_rate);
    std::vector<uint8_t> raw(out.size(), 0);
    for (std::size_t i = 0; i < out.size(); ++i) {
        const int64_t a = out[i].frame, b = i + 1 < out.size() ? out[i + 1].frame : frames;
        const int64_t len = std::min(b - a, look);
        const double head = rms(a, a + std::min(head_max, std::max<int64_t>(1, len / 3)));
        const double tail = rms(a + len * 6 / 10, a + len);
        raw[i] = tail < 0.25 * head ? 1 : 0;
    }
    for (std::size_t i = 0; i < out.size(); ++i) {
        int votes = 0, total = 0;
        for (std::size_t k = i >= 2 ? i - 2 : 0; k <= std::min(out.size() - 1, i + 2); ++k, ++total) votes += raw[k];
        out[i].percussive = 2 * votes > total ? 1 : 0;
    }
}

} // namespace saine
