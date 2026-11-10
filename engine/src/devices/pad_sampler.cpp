// Pad Sampler and Drum Rack (reference: the reference app's built-in drum rack and the
// sampler inside each of its pads). A pad holds up to 8 sounds (layers) that
// alternate hit by hit (round robin) and shares one set of settings: gain,
// pan, transpose, fades, start / end, reverse, gate, warp. The Drum Rack is 16
// pads on MIDI notes 36..51 summed to its output; the Pad Sampler is one pad
// answering every note. Voices come from the shared sampler engine.
#include "builtin.hpp"
#include "sampler_engine.hpp"

#include <array>
#include <cmath>
#include <cstring>

namespace dstudio {
namespace {

using namespace sampler;

constexpr uint32_t kLayers = 8;
constexpr uint32_t kPads = 16;
constexpr uint8_t kBaseNote = 36;
constexpr int kVoicesPerPad = 4;

// Pad settings in Pad Sampler order (the Drum Rack prefixes each pad's block
// with its on switch).
enum PadParam : uint32_t { kGain, kPan, kTranspose, kFadeIn, kFadeOut, kStart, kEnd, kReverse, kGate, kWarp, kPadParams };

int ival(float v, int lo, int hi) noexcept {
    return std::isfinite(v) ? std::clamp(static_cast<int>(std::lround(v)), lo, hi) : lo;
}

class Pad {
public:
    void prepare(double sr) noexcept {
        sr_ = sr;
        vs_.sr = sr;
        pool_.prepare(sr);
    }
    void reset() noexcept {
        pool_.reset();
        rr_ = 0;
    }

    // p: this pad's block (kPadParams values).
    void settings(const float* p, double tempo) noexcept {
        VoiceSettings& s = vs_;
        s.gain = p[kGain] <= -59.95f ? 0.0f : dsp::db_to_gain(clampf(p[kGain], -60.0f, 12.0f));
        s.pan = clampf(p[kPan], -1.0f, 1.0f);
        s.transpose = static_cast<float>(ival(p[kTranspose], -24, 24));
        s.key_tracks = false;
        s.use_amp_env = false;
        s.velocity_sens = 1.0f;
        s.fade_in = clampf(p[kFadeIn], 0.0f, 100.0f) / 100.0f;
        s.fade_out = clampf(p[kFadeOut], 0.0f, 100.0f) / 100.0f;
        start_ = clampf(p[kStart], 0.0f, 1.0f);
        end_ = clampf(p[kEnd], 0.0f, 1.0f);
        if (end_ < start_) std::swap(start_, end_);
        reverse_ = ival(p[kReverse], 0, 1) == 1;
        gate_ = ival(p[kGate], 0, 1) == 1;
        s.warp_frames = warp_quarters(ival(p[kWarp], 0, 4)) * 60.0 / std::clamp(tempo, 20.0, 999.0) * sr_;
    }

    // layers: this pad's 8 sample slots.
    void hit(uint8_t note, uint8_t vel, const BuiltinSample* layers) noexcept {
        if (layers == nullptr) return;
        uint32_t filled[kLayers];
        uint32_t n = 0;
        for (uint32_t i = 0; i < kLayers; ++i)
            if (layers[i].frames > 0) filled[n++] = i;
        if (n == 0) return;
        const uint32_t layer = filled[rr_++ % n];
        const BuiltinSample& s = layers[layer];
        Region rg;
        rg.slot = layer;
        rg.start = static_cast<double>(start_) * static_cast<double>(s.frames);
        rg.end = static_cast<double>(end_) * static_cast<double>(s.frames);
        rg.reverse = reverse_;
        rg.one_shot = !gate_;
        if (rg.end - rg.start < 1.0) return;
        Voice* v = pool_.allocate(kVoicesPerPad);
        v->start(vs_, s, rg, note, vel, std::nanf(""), 0.0f, pool_.next_age());
    }
    void release(uint8_t note) noexcept { pool_.release_note(note); }
    void all_off(bool hard) noexcept { pool_.all_off(hard); }

    void render(const BuiltinSample* layers, float* l, float* r, uint32_t frames) noexcept {
        if (layers == nullptr) return;
        pool_.drop_stale(layers, kLayers);
        pool_.render(vs_, layers, kLayers, 0.0f, l, r, frames);
    }

    // Activity (0..1), playhead of the newest voice (-1 = none), its layer (-1).
    void readouts(float& level, float& pos, float& layer) const noexcept {
        level = 0.0f;
        pos = layer = -1.0f;
        const Voice* newest = nullptr;
        for (int i = 0; i < VoicePool::kMaxVoices; ++i) {
            const Voice& v = pool_.voices()[i];
            if (!v.active() || v.killing()) continue;
            level = std::max(level, v.level());
            if (newest == nullptr || v.age() > newest->age()) newest = &v;
        }
        if (newest != nullptr) {
            pos = newest->position();
            layer = static_cast<float>(newest->slot());
        }
    }

private:
    double sr_ = 48000.0;
    VoicePool pool_;
    VoiceSettings vs_;
    uint32_t rr_ = 0;
    float start_ = 0.0f, end_ = 1.0f;
    bool reverse_ = false, gate_ = false;
};

double tempo_of(const BuiltinContext& ctx) noexcept {
    return ctx.transport != nullptr && ctx.transport->tempo_bpm > 0.0 ? ctx.transport->tempo_bpm : 120.0;
}

// Sample-accurate event dispatch: render(from, to) between events, then on(event).
template <class Render, class On>
void dispatch(const BuiltinContext& ctx, uint32_t frames, Render&& render, On&& on) noexcept {
    uint32_t pos = 0;
    for (uint32_t e = 0; e < ctx.event_count; ++e) {
        const uint32_t at = std::min(ctx.events[e].offset, frames);
        if (at > pos) render(pos, at);
        pos = std::max(pos, at);
        on(ctx.events[e]);
    }
    if (pos < frames) render(pos, frames);
}

// ---------------------------------------------------------------- Pad Sampler

constexpr const char* kPadIds[] = {"gain", "pan", "transpose", "fadeIn", "fadeOut", "start", "end", "reverse", "gate", "warp"};
static_assert(sizeof(kPadIds) / sizeof(kPadIds[0]) == kPadParams);
constexpr float kPadDefaults[kPadParams] = {0, 0, 0, 0, 0, 0, 1, 0, 0, 0};
constexpr const char* kPadReadouts[] = {"level", "pos", "layer"};
constexpr float kPadIdle[] = {0.0f, -1.0f, -1.0f};

class PadSampler final : public BuiltinProcessor {
public:
    void prepare(double sr, uint32_t) override { pad_.prepare(sr); }
    void reset() noexcept override { pad_.reset(); }
    void process(float* l, float* r, uint32_t frames, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        std::memset(l, 0, sizeof(float) * frames);
        std::memset(r, 0, sizeof(float) * frames);
        pad_.settings(p, tempo_of(ctx));
        const BuiltinSample* layers = ctx.sample_count >= kLayers ? ctx.samples : nullptr;
        dispatch(
            ctx, frames, [&](uint32_t a, uint32_t b) { pad_.render(layers, l + a, r + a, b - a); },
            [&](const MidiEvent& m) {
                const uint8_t type = m.status & 0xf0;
                if (type == 0x90 && m.data2 > 0) pad_.hit(m.data1 & 0x7f, m.data2, layers);
                else if (type == 0x80 || type == 0x90) pad_.release(m.data1 & 0x7f);
                else if (type == 0xb0 && (m.data1 == 120 || m.data1 == 123)) pad_.all_off(m.data1 == 120);
            });
        pad_.readouts(ro[0], ro[1], ro[2]);
    }

private:
    Pad pad_;
};

std::unique_ptr<BuiltinProcessor> create_pad_sampler() { return std::make_unique<PadSampler>(); }

// ---------------------------------------------------------------- Drum Rack

constexpr uint32_t kRackBlock = kPadParams + 1; // on + pad settings

#define DSTUDIO_PAD_IDS(n) "p" #n "_on", "p" #n "_gain", "p" #n "_pan", "p" #n "_transpose", "p" #n "_fadeIn", \
    "p" #n "_fadeOut", "p" #n "_start", "p" #n "_end", "p" #n "_reverse", "p" #n "_gate", "p" #n "_warp"
constexpr const char* kRackIds[] = {DSTUDIO_PAD_IDS(1),  DSTUDIO_PAD_IDS(2),  DSTUDIO_PAD_IDS(3),  DSTUDIO_PAD_IDS(4),
                                    DSTUDIO_PAD_IDS(5),  DSTUDIO_PAD_IDS(6),  DSTUDIO_PAD_IDS(7),  DSTUDIO_PAD_IDS(8),
                                    DSTUDIO_PAD_IDS(9),  DSTUDIO_PAD_IDS(10), DSTUDIO_PAD_IDS(11), DSTUDIO_PAD_IDS(12),
                                    DSTUDIO_PAD_IDS(13), DSTUDIO_PAD_IDS(14), DSTUDIO_PAD_IDS(15), DSTUDIO_PAD_IDS(16)};
#undef DSTUDIO_PAD_IDS
static_assert(sizeof(kRackIds) / sizeof(kRackIds[0]) == kPads * kRackBlock);
constexpr auto kRackDefaults = [] {
    std::array<float, kPads * kRackBlock> v{};
    for (uint32_t p = 0; p < kPads; ++p) {
        v[p * kRackBlock] = 1.0f;
        for (uint32_t i = 0; i < kPadParams; ++i) v[p * kRackBlock + 1 + i] = kPadDefaults[i];
    }
    return v;
}();

#define DSTUDIO_PAD_RO(p) "lvl" #p, "pos" #p, "layer" #p
constexpr const char* kRackReadouts[] = {DSTUDIO_PAD_RO(0),  DSTUDIO_PAD_RO(1),  DSTUDIO_PAD_RO(2),  DSTUDIO_PAD_RO(3),
                                         DSTUDIO_PAD_RO(4),  DSTUDIO_PAD_RO(5),  DSTUDIO_PAD_RO(6),  DSTUDIO_PAD_RO(7),
                                         DSTUDIO_PAD_RO(8),  DSTUDIO_PAD_RO(9),  DSTUDIO_PAD_RO(10), DSTUDIO_PAD_RO(11),
                                         DSTUDIO_PAD_RO(12), DSTUDIO_PAD_RO(13), DSTUDIO_PAD_RO(14), DSTUDIO_PAD_RO(15)};
#undef DSTUDIO_PAD_RO
constexpr float kRackIdle[] = {0, -1, -1, 0, -1, -1, 0, -1, -1, 0, -1, -1, 0, -1, -1, 0, -1, -1, 0, -1, -1, 0, -1, -1,
                               0, -1, -1, 0, -1, -1, 0, -1, -1, 0, -1, -1, 0, -1, -1, 0, -1, -1, 0, -1, -1, 0, -1, -1};
static_assert(sizeof(kRackReadouts) / sizeof(kRackReadouts[0]) == 3 * kPads);
static_assert(sizeof(kRackIdle) / sizeof(kRackIdle[0]) == 3 * kPads);

class DrumRack final : public BuiltinProcessor {
public:
    void prepare(double sr, uint32_t) override {
        for (Pad& p : pads_) p.prepare(sr);
    }
    void reset() noexcept override {
        for (Pad& p : pads_) p.reset();
    }
    void process(float* l, float* r, uint32_t frames, const float* p, const BuiltinContext& ctx, float* ro) noexcept override {
        std::memset(l, 0, sizeof(float) * frames);
        std::memset(r, 0, sizeof(float) * frames);
        const double tempo = tempo_of(ctx);
        bool on[kPads];
        for (uint32_t i = 0; i < kPads; ++i) {
            const float* blk = p + i * kRackBlock;
            on[i] = ival(blk[0], 0, 1) == 1;
            pads_[i].settings(blk + 1, tempo);
        }
        const bool have = ctx.samples != nullptr && ctx.sample_count >= kPads * kLayers;
        const auto layers = [&](uint32_t i) { return have ? ctx.samples + i * kLayers : nullptr; };
        dispatch(
            ctx, frames,
            [&](uint32_t a, uint32_t b) {
                for (uint32_t i = 0; i < kPads; ++i) pads_[i].render(layers(i), l + a, r + a, b - a);
            },
            [&](const MidiEvent& m) {
                const uint8_t type = m.status & 0xf0;
                const int pad = static_cast<int>(m.data1 & 0x7f) - kBaseNote;
                if (type == 0xb0 && (m.data1 == 120 || m.data1 == 123)) {
                    for (Pad& pd : pads_) pd.all_off(m.data1 == 120);
                    return;
                }
                if (pad < 0 || pad >= static_cast<int>(kPads)) return;
                if (type == 0x90 && m.data2 > 0) {
                    if (on[pad]) pads_[pad].hit(m.data1 & 0x7f, m.data2, layers(static_cast<uint32_t>(pad)));
                } else if (type == 0x80 || type == 0x90) {
                    pads_[pad].release(m.data1 & 0x7f);
                }
            });
        for (uint32_t i = 0; i < kPads; ++i) {
            if (!on[i]) pads_[i].all_off(false); // a pad switched off fades out
            pads_[i].readouts(ro[3 * i], ro[3 * i + 1], ro[3 * i + 2]);
        }
    }

private:
    Pad pads_[kPads];
};

std::unique_ptr<BuiltinProcessor> create_drum_rack() { return std::make_unique<DrumRack>(); }

} // namespace

extern const BuiltinDescriptor kPadSamplerDescriptor;
extern const BuiltinDescriptor kDrumRackDescriptor;
const BuiltinDescriptor kPadSamplerDescriptor{"pad-sampler", kPadIds, kPadReadouts, &create_pad_sampler, kPadIdle, kPadDefaults, true, kLayers};
const BuiltinDescriptor kDrumRackDescriptor{"drum-rack", kRackIds, kRackReadouts, &create_drum_rack, kRackIdle, kRackDefaults, true, kPads * kLayers};

} // namespace dstudio
