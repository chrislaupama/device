// Out-of-line parts of the shared device DSP helpers (dsp.hpp).
#include "dsp.hpp"

#include "../plugins/plugin_instance.hpp"

namespace saine::dsp {

void LfoClock::begin(float hz, bool synced, float quarters, const RtTransport* tr, uint32_t offset,
                     double sample_rate) noexcept {
    const double bpm = tr != nullptr && tr->tempo_bpm > 0.0 && std::isfinite(tr->tempo_bpm) ? tr->tempo_bpm : 120.0;
    const double q = std::max(1e-3, static_cast<double>(quarters));
    double f = synced ? bpm / 60.0 / q : static_cast<double>(hz);
    if (!std::isfinite(f) || f < 0.0) f = 0.0;
    f = std::min(f, 0.25 * sample_rate);
    inc = f / sample_rate;
    if (synced && tr != nullptr && tr->playing && tr->sample_rate > 0.0) {
        const double beats = static_cast<double>(tr->position + static_cast<int64_t>(offset)) / tr->sample_rate * bpm / 60.0;
        phase = wrap01(beats / q);
    }
    if (!std::isfinite(phase)) phase = 0.0;
}

namespace {
double bessel_i0(double x) {
    double sum = 1.0, term = 1.0;
    for (int k = 1; k < 40; ++k) {
        term *= (x / (2.0 * k)) * (x / (2.0 * k));
        sum += term;
        if (term < 1e-12 * sum) break;
    }
    return sum;
}
} // namespace

Oversampler2x::Oversampler2x() {
    constexpr int c = (kTaps - 1) / 2;
    constexpr double beta = 7.0;
    const double norm = bessel_i0(beta);
    double sum = 0.0;
    double h[kTaps];
    for (int j = 0; j < kTaps; ++j) {
        const double x = 0.5 * (j - c);
        const double sinc = x == 0.0 ? 1.0 : std::sin(3.14159265358979323846 * x) / (3.14159265358979323846 * x);
        const double r = static_cast<double>(j - c) / c;
        const double w = bessel_i0(beta * std::sqrt(std::max(0.0, 1.0 - r * r))) / norm;
        h[j] = (j - c) % 2 == 0 && j != c ? 0.0 : 0.5 * sinc * w;
        sum += h[j];
    }
    for (int j = 0; j < kTaps; ++j) h_[j] = static_cast<float>(h[j] / sum);
    reset();
}

void Oversampler2x::reset() noexcept {
    std::fill(std::begin(up_hist_), std::end(up_hist_), 0.0f);
    std::fill(std::begin(dn_hist_), std::end(dn_hist_), 0.0f);
    up_pos_ = dn_pos_ = 0;
}

// up_hist_ holds base-rate inputs, newest at up_pos_ (circular over kTaps).
// Even 2x output: 2 * sum over even j of h[j] x[n - j/2]; odd output: the
// centre tap only, x[n - (c - 1) / 2] (half-band: other odd taps are zero).
void Oversampler2x::up(float x, float& y0, float& y1) noexcept {
    constexpr int c = (kTaps - 1) / 2;
    constexpr int n = (kTaps + 1) / 2; // base-rate history needed
    up_pos_ = (up_pos_ + 1) % n;
    up_hist_[up_pos_] = x;
    float acc = 0.0f;
    for (int j = 0, k = up_pos_; j < kTaps; j += 2) {
        acc += h_[j] * up_hist_[k];
        k = k == 0 ? n - 1 : k - 1;
    }
    y0 = 2.0f * acc;
    y1 = up_hist_[(up_pos_ - (c - 1) / 2 + n) % n] * (2.0f * h_[c]);
}

// dn_hist_: [0, n) even-index 2x samples, [n, 2n) odd-index samples.
// z[n] = sum over even j of h[j] v[2n - j] + h[c] v[2n - c].
float Oversampler2x::down(float y0, float y1) noexcept {
    constexpr int c = (kTaps - 1) / 2;
    constexpr int n = (kTaps + 1) / 2;
    dn_pos_ = (dn_pos_ + 1) % n;
    dn_hist_[dn_pos_] = y0;
    float acc = 0.0f;
    for (int j = 0, k = dn_pos_; j < kTaps; j += 2) {
        acc += h_[j] * dn_hist_[k];
        k = k == 0 ? n - 1 : k - 1;
    }
    // v[2n - c] is odd-indexed: the odd sample (c + 1) / 2 base steps back.
    acc += h_[c] * dn_hist_[n + (dn_pos_ - (c + 1) / 2 + n) % n];
    dn_hist_[n + dn_pos_] = y1;
    return acc;
}

} // namespace saine::dsp
