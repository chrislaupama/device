#include "sampler_engine.hpp"

#include <cstring>
#include <limits>

namespace dstudio::sampler {

namespace {

double bessel_i0(double x) {
    double sum = 1.0, term = 1.0;
    for (int k = 1; k < 32; ++k) {
        term *= (x / (2.0 * k)) * (x / (2.0 * k));
        sum += term;
        if (term < 1e-12 * sum) break;
    }
    return sum;
}

SincTables build_tables() {
    SincTables t{};
    constexpr double kBeta = 7.5;
    const double i0b = bessel_i0(kBeta);
    // Table 0 (rates up to 1) is the full-band interpolator; the others are
    // anti-aliasing low-passes for faster reads.
    const double cut[kCutoffs] = {1.0, 0.80, 0.67, 0.56, 0.47, 0.40, 0.33, 0.28, 0.24, 0.20};
    for (int k = 0; k < kCutoffs; ++k) {
        t.cutoff[k] = static_cast<float>(cut[k]);
        for (int p = 0; p <= kPhases; ++p) {
            const double frac = static_cast<double>(p) / kPhases;
            double row[kTaps];
            double sum = 0.0;
            for (int j = 0; j < kTaps; ++j) {
                // Tap j reads sample floor(pos) - (kHalf - 1) + j.
                const double x = static_cast<double>(j - (kHalf - 1)) - frac;
                const double c = cut[k];
                const double s = std::fabs(x) < 1e-12 ? c : std::sin(3.14159265358979323846 * c * x) / (3.14159265358979323846 * x);
                const double w = x / kHalf;
                const double win = std::fabs(w) >= 1.0 ? 0.0 : bessel_i0(kBeta * std::sqrt(1.0 - w * w)) / i0b;
                row[j] = s * win;
                sum += row[j];
            }
            for (int j = 0; j < kTaps; ++j) t.coef[k][p][j] = static_cast<float>(row[j] / sum); // unity DC gain
        }
    }
    return t;
}

constexpr uint32_t kControl = 16; // frames per control update (pitch, filter, pan)

} // namespace

const SincTables& sinc_tables() noexcept {
    static const SincTables t = build_tables();
    return t;
}

// ---------------------------------------------------------------- Voice

void Voice::prepare(double sr) noexcept {
    sr_ = sr;
    tab_ = &sinc_tables();
    // About 43 ms grains: long enough for low notes, short enough to keep
    // transients.
    grain_len_ = std::clamp(static_cast<int>(std::lround(sr * 0.0427)) & ~1, 256, 16384);
    hop_ = grain_len_ / 2;
    active_ = false;
}

float Voice::position() const noexcept {
    if (!active_ || sample_frames_ == 0) return -1.0f;
    const double src = stretch_ ? src_of(off_) : src_of(off_);
    return static_cast<float>(std::clamp(src / static_cast<double>(sample_frames_), 0.0, 1.0));
}

void Voice::start(const VoiceSettings& vs, const BuiltinSample& s, const Region& r, uint8_t note, uint8_t velocity,
                  float from_pitch, float lfo_phase, uint64_t age) noexcept {
    region_ = r;
    // Whole-frame region edges: unpitched playback then reads the samples as they are.
    region_.start = std::floor(std::max(0.0, r.start));
    region_.end = std::floor(std::min(static_cast<double>(s.frames), r.end));
    region_.loop_start = std::floor(r.loop_start);
    note_ = note;
    age_ = age;
    token_ = sample_token(s);
    sample_frames_ = s.frames;
    active_ = true;
    held_ = true;
    killing_ = 0;
    finished_ = false;
    const float v = static_cast<float>(velocity) / 127.0f;
    vel_gain_ = 1.0f - std::clamp(vs.velocity_sens, 0.0f, 1.0f) * (1.0f - v * v);
    target_pitch_ = (vs.key_tracks ? static_cast<float>(note - vs.root) : 0.0f) + r.pitch;
    glide_pitch_ = std::isnan(from_pitch) || vs.glide_ms <= 0.0f ? target_pitch_ : from_pitch;
    set_envelopes(vs);
    amp_.gate_on(true);
    flt_env_.gate_on(true);
    filter_.reset();
    lfo_phase_ = lfo_phase - std::floor(lfo_phase);
    elapsed_ = 0.0;

    const Region& g = region_;
    len_ = std::max(0.0, g.end - g.start);
    looping_ = g.loop;
    if (g.reverse) {
        play_end_ = looping_ ? g.end - std::clamp(g.loop_start, g.start, g.end) : len_;
        loop_back_ = 0.0;
    } else {
        play_end_ = len_;
        loop_back_ = std::clamp(g.loop_start, g.start, g.end) - g.start;
    }
    if (looping_ && play_end_ - loop_back_ < 2.0 * kLoopXfade) looping_ = false;
    if (!looping_) play_end_ = len_;
    // Fades are fractions of the region; a 0.5 ms declick always applies.
    const double declick = 0.0005 * sr_;
    fade_in_ = std::max(declick, static_cast<double>(clampf(vs.fade_in, 0.0f, 1.0f)) * len_);
    fade_out_ = std::max(declick, static_cast<double>(clampf(vs.fade_out, 0.0f, 1.0f)) * len_);
    if (fade_in_ + fade_out_ > len_ && len_ > 0.0) {
        const double k = len_ / (fade_in_ + fade_out_);
        fade_in_ *= k;
        fade_out_ *= k;
    }
    off_ = 0.0;
    stretch_ = vs.warp_frames > 0.0 && len_ > 0.0;
    stretch_rate_ = stretch_ ? len_ / vs.warp_frames : 1.0;
    grains_[0] = grains_[1] = Grain{};
    since_hop_ = hop_; // the first frame starts a grain
    if (len_ <= 0.0) active_ = false;
}

void Voice::retarget(const VoiceSettings& vs, uint8_t note) noexcept {
    note_ = note;
    held_ = true;
    target_pitch_ = (vs.key_tracks ? static_cast<float>(note - vs.root) : 0.0f) + region_.pitch;
    if (vs.glide_ms <= 0.0f) glide_pitch_ = target_pitch_;
    if (amp_.stage() == Adsr::Release) amp_.gate_on(false);
}

// Also per block: envelope edits reach sounding notes (their running stage).
void Voice::set_envelopes(const VoiceSettings& vs) noexcept {
    if (vs.use_amp_env)
        amp_.set(vs.amp_a, vs.amp_d, vs.amp_s, vs.amp_r, sr_);
    else
        amp_.set(0.5f, 0.0f, 1.0f, 30.0f, sr_);
    flt_env_.set(vs.flt_a, vs.flt_d, vs.flt_s, vs.flt_r, sr_);
}

void Voice::release() noexcept {
    held_ = false;
    if (region_.one_shot) return;
    force_release();
}

void Voice::force_release() noexcept {
    held_ = false;
    amp_.gate_off();
    flt_env_.gate_off();
}

void Voice::kill() noexcept {
    if (active_ && killing_ == 0) killing_ = kKillFrames;
}

double Voice::wrap(double off) const noexcept {
    if (!looping_ || off < play_end_) return off;
    const double l = play_end_ - loop_back_;
    return loop_back_ + std::fmod(off - loop_back_, l);
}

void Voice::read_off(const BuiltinSample& s, double off, int k, float& l, float& r) const noexcept {
    if (oct_ < 0) {
        read_frame(*tab_, k, s.ch[0], s.ch[1], static_cast<int64_t>(s.frames), src_of(off), l, r);
        return;
    }
    const double pos = std::ldexp(src_of(off), -(oct_ + 1));
    read_frame(*tab_, k, s.oct[oct_][0], s.oct[oct_][1], static_cast<int64_t>(s.oct_frames[oct_]), pos, l, r);
}

// Near the loop end, crossfade into the material just before the loop start.
void Voice::read_seam(const BuiltinSample& s, double off, int k, float& l, float& r) const noexcept {
    read_off(s, off, k, l, r);
    if (!looping_) return;
    const double into = off - (play_end_ - kLoopXfade);
    if (into <= 0.0) return;
    float l2, r2;
    read_off(s, off - (play_end_ - loop_back_), k, l2, r2);
    const float w = static_cast<float>(std::min(1.0, into / kLoopXfade));
    l += w * (l2 - l);
    r += w * (r2 - r);
}

float Voice::fades(double off) const noexcept {
    double g = 1.0;
    if (off < fade_in_) g = off / fade_in_;
    if (!looping_) {
        const double left = len_ - off;
        if (left < fade_out_) g = std::min(g, std::max(0.0, left / fade_out_));
    }
    return static_cast<float>(g);
}

// Normalised cross-correlation search: where should a new grain start so it
// continues the running grain's waveform (WSOLA)? want and the result are
// offsets; the running grain continues from its own position.
double Voice::aligned_start(const BuiltinSample& s, double want, double rate) const noexcept {
    const Grain* g = grains_[0].age >= 0 ? &grains_[0] : grains_[1].age >= 0 ? &grains_[1] : nullptr;
    if (g == nullptr) return want;
    const auto frames = static_cast<int64_t>(s.frames);
    const float* x = s.ch[0];
    const auto at = [&](double off) -> float {
        const auto i = static_cast<int64_t>(src_of(wrap(off)));
        return i >= 0 && i < frames ? x[i] : 0.0f;
    };
    constexpr int kPoints = 96;
    const double step = 3.0 * std::max(0.25, rate);
    float ref[kPoints];
    for (int n = 0; n < kPoints; ++n) ref[n] = at(g->src + n * step);
    const int span = grain_len_ / 4;
    const auto score = [&](double cand) {
        float c = 0.0f, e = 1e-9f;
        for (int n = 0; n < kPoints; ++n) {
            const float v = at(cand + n * step);
            c += v * ref[n];
            e += v * v;
        }
        return c / std::sqrt(e);
    };
    double best = want;
    float best_score = -std::numeric_limits<float>::infinity();
    for (int d = -span; d <= span; d += 8) {
        const float sc = score(want + d);
        if (sc > best_score) {
            best_score = sc;
            best = want + d;
        }
    }
    const double coarse = best;
    for (int d = -7; d <= 7; ++d) {
        if (d == 0) continue;
        const float sc = score(coarse + d);
        if (sc > best_score) {
            best_score = sc;
            best = coarse + d;
        }
    }
    return std::max(0.0, best);
}

void Voice::stretch_frame(const BuiltinSample& s, double rate, int k, float& l, float& r) noexcept {
    if (since_hop_ >= hop_ && !finished_) {
        since_hop_ = 0;
        const int free = grains_[0].age < 0 ? 0 : grains_[1].age < 0 ? 1 : (grains_[0].age > grains_[1].age ? 0 : 1);
        grains_[free].src = aligned_start(s, off_, rate);
        grains_[free].age = 0;
    }
    ++since_hop_;
    l = r = 0.0f;
    for (Grain& g : grains_) {
        if (g.age < 0) continue;
        const float ph = static_cast<float>(g.age) / static_cast<float>(grain_len_);
        const float sn = std::sin(kPi * ph);
        const float w = sn * sn;
        float gl, gr;
        const double o = wrap(g.src);
        if (looping_ || o < len_) {
            read_off(s, o, k, gl, gr);
            l += w * gl;
            r += w * gr;
        }
        g.src += rate;
        if (++g.age >= grain_len_) g.age = -1;
    }
}

void Voice::render(const VoiceSettings& vs, const BuiltinSample& s, float bend, float* out_l, float* out_r,
                   uint32_t frames) noexcept {
    if (!active_) return;
    if (s.frames == 0 || sample_token(s) != token_) {
        active_ = false;
        return;
    }
    set_envelopes(vs);
    const double sr = sr_;
    for (uint32_t i0 = 0; i0 < frames && active_; i0 += kControl) {
        const uint32_t n = std::min(kControl, frames - i0);
        // ---- control rate: glide, LFO, pitch, filter, pan
        if (vs.glide_ms > 0.0f) {
            // Glide time = the time to (nearly) arrive: fall constant 5, like the envelopes.
            const float k = std::exp(-5.0f * static_cast<float>(n) * 1000.0f / (vs.glide_ms * static_cast<float>(sr)));
            glide_pitch_ = target_pitch_ + (glide_pitch_ - target_pitch_) * k;
        } else {
            glide_pitch_ = target_pitch_;
        }
        float lfo = 0.0f;
        if (vs.lfo_amount > 0.0f) {
            const float fade = vs.lfo_fade_ms > 0.0f ? static_cast<float>(std::min(1.0, elapsed_ * 1000.0 / static_cast<double>(vs.lfo_fade_ms))) : 1.0f;
            float ph = lfo_phase_ + vs.lfo_phase;
            ph -= std::floor(ph);
            lfo = lfo_shape(vs.lfo_shape, ph) * vs.lfo_amount * fade;
        }
        lfo_phase_ += static_cast<float>(static_cast<double>(vs.lfo_rate) * n / sr);
        lfo_phase_ -= std::floor(lfo_phase_);
        elapsed_ += n / sr;
        float pitch = glide_pitch_ + vs.transpose + bend;
        if (vs.lfo_target == 0) pitch += lfo * 12.0f * vs.lfo_amount; // depth 12 st x amount^2: fine near zero
        pitch = std::clamp(pitch, -60.0f, 60.0f);
        const double ratio = std::exp2(static_cast<double>(pitch) / 12.0);
        // Re-pitch: the read head moves at the pitch ratio. Warp: the head
        // moves at the stretch rate and grains read at the pitch ratio.
        const double head_rate = stretch_ ? stretch_rate_ : ratio;
        // Far up, read an octave-down copy so the interpolator's rate stays
        // at most 2 (its 16 taps cannot low-pass much further).
        double read_rate = ratio;
        oct_ = -1;
        while (read_rate > 2.0 && oct_ + 1 < Asset::kOctaves && s.oct[oct_ + 1][0] != nullptr) {
            read_rate *= 0.5;
            ++oct_;
        }
        const int k = cutoff_index(*tab_, read_rate);
        if (vs.filter_on) {
            float st = vs.env_depth * 48.0f * flt_env_.level() + vs.key_track * static_cast<float>(note_ - 60);
            if (vs.lfo_target == 1) st += lfo * 48.0f;
            filter_.set(vs.filter_type, vs.cutoff * std::exp2(st / 12.0f), vs.resonance, vs.drive, sr);
        }
        float pan = vs.pan + region_.pan + (vs.lfo_target == 2 ? lfo : 0.0f);
        pan = std::clamp(pan, -1.0f, 1.0f);
        const float pl = pan > 0.0f ? std::cos(pan * kPi * 0.5f) : 1.0f;
        const float pr = pan < 0.0f ? std::cos(-pan * kPi * 0.5f) : 1.0f;
        float gain = vs.gain * vel_gain_ * region_.gain;
        if (vs.lfo_target == 3) gain *= 1.0f - 0.5f * vs.lfo_amount + 0.5f * lfo;
        // ---- audio rate
        for (uint32_t i = i0; i < i0 + n; ++i) {
            float l = 0.0f, r = 0.0f;
            float fade = 1.0f;
            if (stretch_) {
                stretch_frame(s, ratio, k, l, r);
                fade = fades(std::min(off_, len_));
            } else if (!finished_) {
                read_seam(s, off_, k, l, r);
                fade = fades(off_);
            }
            const float env = amp_.next();
            flt_env_.next();
            float g = gain * env * fade;
            if (killing_ > 0) {
                g *= static_cast<float>(killing_) / kKillFrames;
                if (--killing_ == 0) active_ = false;
            }
            if (vs.filter_on) {
                l = filter_.process(l, 0);
                r = filter_.process(r, 1);
            }
            out_l[i] += g * pl * l;
            out_r[i] += g * pr * r;
            off_ += head_rate;
            if (off_ >= play_end_) {
                if (looping_) {
                    off_ = wrap(off_);
                } else {
                    finished_ = true;
                    off_ = play_end_;
                }
            }
            if (!amp_.active() || (finished_ && (!stretch_ || (grains_[0].age < 0 && grains_[1].age < 0)))) {
                active_ = false;
                break;
            }
        }
    }
}

// ---------------------------------------------------------------- VoicePool

void VoicePool::prepare(double sr) noexcept {
    for (Voice& v : voices_) v.prepare(sr);
}

void VoicePool::reset() noexcept {
    for (Voice& v : voices_) v.stop();
}

int VoicePool::sounding() const noexcept {
    int n = 0;
    for (const Voice& v : voices_) n += v.active() && !v.killing();
    return n;
}

Voice* VoicePool::allocate(int limit) noexcept {
    limit = std::clamp(limit, 1, kMaxVoices - 8);
    while (sounding() >= limit) {
        // Steal: the oldest releasing voice, else the oldest.
        Voice* victim = nullptr;
        for (Voice& v : voices_) {
            if (!v.active() || v.killing()) continue;
            const bool better = victim == nullptr || (v.releasing() && !victim->releasing()) ||
                                (v.releasing() == victim->releasing() && v.age() < victim->age());
            if (better) victim = &v;
        }
        if (victim == nullptr) break;
        victim->kill();
    }
    for (Voice& v : voices_)
        if (!v.active()) return &v;
    Voice* oldest = &voices_[0]; // every slot busy declicking: reuse the oldest
    for (Voice& v : voices_)
        if (v.age() < oldest->age()) oldest = &v;
    oldest->stop();
    return oldest;
}

void VoicePool::release_note(uint8_t note) noexcept {
    for (Voice& v : voices_)
        if (v.active() && v.held() && v.note() == note) v.release();
}

void VoicePool::all_off(bool hard) noexcept {
    for (Voice& v : voices_) {
        if (!v.active()) continue;
        if (hard)
            v.kill();
        else
            v.force_release();
    }
}

void VoicePool::drop_stale(const BuiltinSample* samples, uint32_t count) noexcept {
    for (Voice& v : voices_) {
        if (!v.active()) continue;
        if (v.slot() >= count || sample_token(samples[v.slot()]) != v.token()) v.stop();
    }
}

void VoicePool::render(const VoiceSettings& vs, const BuiltinSample* samples, uint32_t count, float bend, float* l,
                       float* r, uint32_t frames) noexcept {
    for (Voice& v : voices_) {
        if (!v.active()) continue;
        if (v.slot() >= count) {
            v.stop();
            continue;
        }
        v.render(vs, samples[v.slot()], bend, l, r, frames);
    }
}

} // namespace dstudio::sampler
