// Sampler (reference: the reference app's built-in "Sampler", sampler-steam). Plays one
// sample over the keyboard: Classic (sustain loop, amp envelope), One Shot
// (the whole region per note) or Slice (one slice per key from the slice
// start key). Multimode filter with its own envelope, one LFO, glide, pitch
// bend, mono (legato, note priority: last) or poly voicing, warp. The voice
// engine is shared with the Pad Sampler and the Drum Rack (sampler_engine).
#include "builtin.hpp"
#include "sampler_engine.hpp"

#include <bitset>
#include <cmath>
#include <cstring>

namespace saine {
namespace {

using namespace sampler;

enum P : uint32_t {
    kMode, kPlayMode, kVoicing, kVoices, kWarp, kRoot, kVolume, kPan, kTranspose, kFine, kGlide, kBendRange,
    kVelocity, kLegato, kAmpA, kAmpD, kAmpS, kAmpR, kFilterOn, kFilterType, kCutoff, kResonance, kDrive, kKeyTrack,
    kEnvDepth, kFltA, kFltD, kFltS, kFltR, kLfoShape, kLfoRate, kLfoAmount, kLfoFade, kLfoPhase, kLfoTarget,
    kLfoRetrig, kStart, kEnd, kFadeIn, kFadeOut, kReverse, kLoop, kLoopStart, kSliceKey, kChop, kSensitivity,
    kDivision, kParamCount
};

constexpr const char* kParamIds[] = {
    "mode", "playMode", "voicing", "voices", "warp", "root", "volume", "pan", "transpose", "fineTune", "glide",
    "bendRange", "velocity", "legato", "ampAttack", "ampDecay", "ampSustain", "ampRelease", "filterOn", "filterType",
    "cutoff", "resonance", "drive", "keyTrack", "envDepth", "filterAttack", "filterDecay", "filterSustain",
    "filterRelease", "lfoShape", "lfoRate", "lfoAmount", "lfoFadeIn", "lfoPhase", "lfoTarget", "lfoKeyTrigger",
    "start", "end", "fadeIn", "fadeOut", "reverse", "loop", "loopStart", "sliceKey", "chop", "sensitivity",
    "division"};
static_assert(sizeof(kParamIds) / sizeof(kParamIds[0]) == kParamCount);

constexpr int kPlayheads = 8;
constexpr const char* kReadoutIds[] = {"pos0", "pos1", "pos2", "pos3", "pos4", "pos5", "pos6", "pos7"};
constexpr float kIdleReadouts[] = {-1, -1, -1, -1, -1, -1, -1, -1};

enum Mode { kClassic = 0, kOneShot = 1, kSlice = 2 };
enum PlayMode { kTrigger = 0, kGate = 1, kToggle = 2 };

int ival(const float* p, uint32_t i, int lo, int hi) noexcept {
    const float v = p[i];
    return std::isfinite(v) ? std::clamp(static_cast<int>(std::lround(v)), lo, hi) : lo;
}

class Sampler final : public BuiltinProcessor {
public:
    void prepare(double sample_rate, uint32_t) override {
        sr_ = sample_rate;
        pool_.prepare(sample_rate);
        vs_.sr = sample_rate;
    }

    void reset() noexcept override {
        pool_.reset();
        stack_n_ = 0;
        sustain_ = false;
        sustained_.reset();
        bend_ = 0.0f;
        last_pitch_ = std::nanf("");
        free_phase_ = 0.0f;
    }

    void process(float* l, float* r, uint32_t frames, const float* p, const BuiltinContext& ctx,
                 float* ro) noexcept override {
        std::memset(l, 0, sizeof(float) * frames);
        std::memset(r, 0, sizeof(float) * frames);
        settings(p, ctx);
        const BuiltinSample* samples = ctx.samples;
        const uint32_t count = ctx.sample_count;
        if (samples != nullptr) pool_.drop_stale(samples, count);
        uint32_t pos = 0;
        for (uint32_t e = 0; e < ctx.event_count; ++e) {
            const MidiEvent& m = ctx.events[e];
            const uint32_t at = std::min(m.offset, frames);
            if (at > pos && samples != nullptr) pool_.render(vs_, samples, count, bend_ * vs_.bend_range, l + pos, r + pos, at - pos);
            pos = std::max(pos, at);
            handle(m, p, samples, count);
        }
        if (pos < frames && samples != nullptr) pool_.render(vs_, samples, count, bend_ * vs_.bend_range, l + pos, r + pos, frames - pos);
        free_phase_ += static_cast<float>(static_cast<double>(vs_.lfo_rate) * frames / sr_);
        free_phase_ -= std::floor(free_phase_);
        // Playheads of the (up to 8) newest sounding voices.
        const Voice* v = pool_.voices();
        int idx[kPlayheads];
        int n = 0;
        for (int i = 0; i < VoicePool::kMaxVoices; ++i) {
            if (!v[i].active() || v[i].killing()) continue;
            if (n < kPlayheads) {
                idx[n++] = i;
                continue;
            }
            int oldest = 0;
            for (int k = 1; k < n; ++k)
                if (v[idx[k]].age() < v[idx[oldest]].age()) oldest = k;
            if (v[i].age() > v[idx[oldest]].age()) idx[oldest] = i;
        }
        for (int k = 0; k < kPlayheads; ++k) ro[k] = k < n ? v[idx[k]].position() : -1.0f;
    }

private:
    void settings(const float* p, const BuiltinContext& ctx) noexcept {
        VoiceSettings& s = vs_;
        mode_ = ival(p, kMode, 0, 2);
        play_mode_ = ival(p, kPlayMode, 0, 2);
        poly_ = ival(p, kVoicing, 0, 1) == 1;
        voices_ = ival(p, kVoices, 1, 32);
        legato_ = ival(p, kLegato, 0, 1) == 1;
        s.gain = dsp::db_to_gain(clampf(p[kVolume], -60.0f, 12.0f));
        if (p[kVolume] <= -59.95f) s.gain = 0.0f;
        s.pan = clampf(p[kPan], -1.0f, 1.0f);
        s.transpose = clampf(p[kTranspose], -24.0f, 24.0f) + clampf(p[kFine], -100.0f, 100.0f) / 100.0f;
        s.bend_range = clampf(p[kBendRange], 0.0f, 24.0f);
        s.velocity_sens = clampf(p[kVelocity], 0.0f, 1.0f);
        s.glide_ms = clampf(p[kGlide], 0.0f, 2000.0f);
        s.root = ival(p, kRoot, 0, 127);
        s.key_tracks = mode_ != kSlice;
        s.use_amp_env = true;
        s.amp_a = clampf(p[kAmpA], 0.0f, 2000.0f);
        s.amp_d = clampf(p[kAmpD], 0.0f, 2000.0f);
        s.amp_s = clampf(p[kAmpS], 0.0f, 1.0f);
        s.amp_r = clampf(p[kAmpR], 0.0f, 4000.0f);
        s.filter_on = ival(p, kFilterOn, 0, 1) == 1;
        s.filter_type = static_cast<FilterType>(ival(p, kFilterType, 0, 3));
        s.cutoff = clampf(p[kCutoff], 20.0f, 20000.0f);
        s.resonance = clampf(p[kResonance], 0.0f, 1.0f);
        s.drive = clampf(p[kDrive], 0.0f, 1.0f);
        s.key_track = clampf(p[kKeyTrack], 0.0f, 1.0f);
        s.env_depth = clampf(p[kEnvDepth], -1.0f, 1.0f);
        s.flt_a = clampf(p[kFltA], 0.0f, 2000.0f);
        s.flt_d = clampf(p[kFltD], 0.0f, 2000.0f);
        s.flt_s = clampf(p[kFltS], 0.0f, 1.0f);
        s.flt_r = clampf(p[kFltR], 0.0f, 4000.0f);
        s.lfo_shape = static_cast<LfoShape>(ival(p, kLfoShape, 0, 3));
        s.lfo_rate = clampf(p[kLfoRate], 0.05f, 20.0f);
        s.lfo_amount = clampf(p[kLfoAmount], 0.0f, 1.0f);
        s.lfo_fade_ms = clampf(p[kLfoFade], 0.0f, 4000.0f);
        s.lfo_phase = clampf(p[kLfoPhase], 0.0f, 1.0f);
        s.lfo_target = ival(p, kLfoTarget, 0, 3);
        retrig_ = ival(p, kLfoRetrig, 0, 1) == 1;
        s.fade_in = clampf(p[kFadeIn], 0.0f, 100.0f) / 100.0f;
        s.fade_out = clampf(p[kFadeOut], 0.0f, 100.0f) / 100.0f;
        const double tempo = ctx.transport != nullptr && ctx.transport->tempo_bpm > 0.0 ? ctx.transport->tempo_bpm : 120.0;
        s.warp_frames = warp_quarters(ival(p, kWarp, 0, 4)) * 60.0 / std::clamp(tempo, 20.0, 999.0) * sr_;
    }

    bool region_for(uint8_t note, const float* p, const BuiltinSample& s, Region& out) const noexcept {
        const auto frames = static_cast<double>(s.frames);
        float a = clampf(p[kStart], 0.0f, 1.0f), b = clampf(p[kEnd], 0.0f, 1.0f);
        if (b < a) std::swap(a, b);
        out = Region{};
        out.slot = 0;
        out.reverse = ival(p, kReverse, 0, 1) == 1;
        if (mode_ == kSlice) {
            const int key = ival(p, kSliceKey, 0, 127);
            const int i = static_cast<int>(note) - key;
            const uint32_t n = s.slice_count;
            if (n == 0) {
                if (i != 0) return false;
                out.start = static_cast<double>(a) * frames;
                out.end = static_cast<double>(b) * frames;
            } else {
                if (i < 0 || static_cast<uint32_t>(i) >= n) return false;
                const BuiltinSlice& sl = s.slices[i];
                out.start = static_cast<double>(clampf(sl.start, 0.0f, 1.0f)) * frames;
                out.end = static_cast<uint32_t>(i) + 1 < n ? static_cast<double>(clampf(s.slices[i + 1].start, 0.0f, 1.0f)) * frames : frames;
                out.gain = dsp::db_to_gain(clampf(sl.gain_db, -60.0f, 12.0f));
                out.pan = clampf(sl.pan, -1.0f, 1.0f);
                out.pitch = clampf(sl.pitch, -24.0f, 24.0f);
            }
            out.one_shot = play_mode_ != kGate;
        } else {
            out.start = static_cast<double>(a) * frames;
            out.end = static_cast<double>(b) * frames;
            if (mode_ == kClassic) {
                out.loop = ival(p, kLoop, 0, 1) == 1;
                out.loop_start = static_cast<double>(std::clamp(clampf(p[kLoopStart], 0.0f, 1.0f), a, b)) * frames;
            } else {
                out.one_shot = play_mode_ != kGate;
            }
        }
        return out.end - out.start >= 1.0;
    }

    void note_on(uint8_t note, uint8_t vel, const float* p, const BuiltinSample* samples, uint32_t count) noexcept {
        if (samples == nullptr || count == 0 || samples[0].frames == 0) return;
        const bool toggle = mode_ != kClassic && play_mode_ == kToggle;
        if (toggle) {
            // A second press stops the sounding note.
            bool stopped = false;
            for (int i = 0; i < VoicePool::kMaxVoices; ++i) {
                Voice& v = pool_.voices()[i];
                if (v.active() && !v.releasing() && v.note() == note) {
                    v.force_release();
                    stopped = true;
                }
            }
            if (stopped) return;
        }
        Region rg;
        if (!region_for(note, p, samples[0], rg)) return;
        const float phase = retrig_ ? 0.0f : free_phase_;
        if (!poly_) {
            push_held(note);
            Voice* cur = nullptr;
            for (int i = 0; i < VoicePool::kMaxVoices; ++i) {
                Voice& v = pool_.voices()[i];
                if (v.active() && !v.killing()) cur = &v;
            }
            if (cur != nullptr && legato_ && !cur->releasing() && mode_ != kSlice) {
                cur->retarget(vs_, note);
                last_pitch_ = static_cast<float>(note - vs_.root);
                return;
            }
            const float from = cur != nullptr ? cur->current_pitch() : last_pitch_;
            Voice* v = pool_.allocate(1);
            v->start(vs_, samples[0], rg, note, vel, from, phase, pool_.next_age());
        } else {
            Voice* v = pool_.allocate(voices_);
            v->start(vs_, samples[0], rg, note, vel, last_pitch_, phase, pool_.next_age());
        }
        last_pitch_ = (vs_.key_tracks ? static_cast<float>(note - vs_.root) : 0.0f) + rg.pitch;
    }

    void note_off(uint8_t note) noexcept {
        if (sustain_) {
            sustained_.set(note);
            remove_held(note);
            return;
        }
        if (!poly_) {
            const bool was_top = stack_n_ > 0 && held_[stack_n_ - 1] == note;
            remove_held(note);
            if (was_top && stack_n_ > 0 && mode_ != kSlice) {
                // Fall back to the previous held key (legato).
                for (int i = 0; i < VoicePool::kMaxVoices; ++i) {
                    Voice& v = pool_.voices()[i];
                    if (v.active() && !v.releasing() && v.note() == note) {
                        v.retarget(vs_, held_[stack_n_ - 1]);
                        return;
                    }
                }
            }
        }
        if (mode_ != kClassic && play_mode_ == kToggle) return;
        pool_.release_note(note);
    }

    void handle(const MidiEvent& m, const float* p, const BuiltinSample* samples, uint32_t count) noexcept {
        const uint8_t type = m.status & 0xf0;
        if (type == 0x90 && m.data2 > 0) {
            note_on(m.data1 & 0x7f, m.data2, p, samples, count);
        } else if (type == 0x80 || type == 0x90) {
            note_off(m.data1 & 0x7f);
        } else if (type == 0xe0) {
            bend_ = bend_value(m.data1, m.data2);
        } else if (type == 0xb0) {
            if (m.data1 == 64) {
                sustain_ = m.data2 >= 64;
                if (!sustain_) {
                    for (int n = 0; n < 128; ++n)
                        if (sustained_.test(static_cast<std::size_t>(n))) pool_.release_note(static_cast<uint8_t>(n));
                    sustained_.reset();
                }
            } else if (m.data1 == 120 || m.data1 == 123) {
                pool_.all_off(m.data1 == 120);
                stack_n_ = 0;
                sustained_.reset();
            }
        }
    }

    void push_held(uint8_t note) noexcept {
        remove_held(note);
        if (stack_n_ == kStack) {
            std::memmove(held_, held_ + 1, kStack - 1);
            --stack_n_;
        }
        held_[stack_n_++] = note;
    }
    void remove_held(uint8_t note) noexcept {
        int w = 0;
        for (int i = 0; i < stack_n_; ++i)
            if (held_[i] != note) held_[w++] = held_[i];
        stack_n_ = w;
    }

    static constexpr int kStack = 16;
    double sr_ = 48000.0;
    VoicePool pool_;
    VoiceSettings vs_;
    int mode_ = kOneShot, play_mode_ = kTrigger, voices_ = 8;
    bool poly_ = false, legato_ = false, retrig_ = true;
    uint8_t held_[kStack] = {};
    int stack_n_ = 0;
    bool sustain_ = false;
    std::bitset<128> sustained_;
    float bend_ = 0.0f;
    float last_pitch_ = std::nanf("");
    float free_phase_ = 0.0f;
};

std::unique_ptr<BuiltinProcessor> create_sampler() { return std::make_unique<Sampler>(); }

} // namespace

extern const BuiltinDescriptor kSamplerDescriptor;
const BuiltinDescriptor kSamplerDescriptor{"sampler-steam", kParamIds, kReadoutIds, &create_sampler, kIdleReadouts, true, 1};

} // namespace saine
