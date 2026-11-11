//! Transient Shaper gain scope history: the engine
//! publishes 750 columns a second (output peak, pre-clip peak, applied gain
//! dB), the plot spans 3000 of them (4 s), one bar per point of plot width.
//!
//! Frames arrive with the device's running column count, so overlapping
//! frames merge without repeats or gaps. The read head trails the newest
//! column by a fixed lead and runs at 750 columns/s on the wall clock, nudged
//! (at most 6 %) towards that lead, so the plot scrolls evenly whatever the
//! audio block size and poll rate are.

/// Columns per second.
pub const RATE: f64 = 750.0;
/// Columns across the plot (4 s).
pub const SPAN: f64 = 3000.0;
/// Columns kept (5 s).
const CAP: i64 = 3750;
/// Columns the head trails the newest column (about 150 ms).
const LEAD: f64 = 113.0;
/// Largest head rate correction, and the lead error that reaches it.
const TRIM: f64 = 0.06;
const TRIM_SPAN: f64 = 375.0;
/// Time constant (ms) of the smoothed lead.
const LEAD_MS: f64 = 600.0;
/// Without a frame for this long (ms), the head runs free into silence.
const STALE_MS: f64 = 250.0;
/// A column is audible above -48 dB.
const AUDIBLE: f32 = 0.003_981_072;
/// Columns of quiet before the scope rests (stopped) or reads as empty.
const QUIET: f64 = 3113.0;
/// Bar decay time constant (ms) from one plot point to the next.
const DECAY_MS: f64 = 3.0;

/// One plot point's values, oldest first, and the sub-point scroll offset.
#[derive(Debug, Clone, Default)]
pub struct Pixels {
    pub out: Vec<f32>,
    pub pre: Vec<f32>,
    pub gain: Vec<f32>,
    pub offset: f32,
}

#[derive(Debug, Clone)]
pub struct ScopeHistory {
    cols: Vec<[f32; 3]>,
    /// One past the newest column written (history index).
    end: i64,
    /// History index minus the device's column count.
    offset: i64,
    /// Device column count after the last frame merged (None: not following).
    src_end: Option<i64>,
    head: f64,
    clock_ms: Option<f64>,
    last_arrival_ms: f64,
    last_audible: f64,
    last_loud: f64,
    lead: Option<f64>,
    pub seq: u64,
}

impl Default for ScopeHistory {
    fn default() -> Self {
        Self {
            cols: vec![[0.0; 3]; CAP as usize],
            end: 0,
            offset: 0,
            src_end: None,
            head: 0.0,
            clock_ms: None,
            last_arrival_ms: f64::NEG_INFINITY,
            last_audible: f64::NEG_INFINITY,
            last_loud: f64::NEG_INFINITY,
            lead: None,
            seq: 0,
        }
    }
}

fn audible(c: &[f32]) -> bool {
    c[0].max(c[1]) > AUDIBLE
}

impl ScopeHistory {
    fn slot(i: i64) -> usize {
        i.rem_euclid(CAP) as usize
    }

    /// Zeroes the columns from `end` up to `to` (exclusive).
    fn silence_to(&mut self, to: i64) {
        if to <= self.end {
            return;
        }
        for i in self.end.max(to - CAP)..to {
            self.cols[Self::slot(i)] = [0.0; 3];
        }
        self.end = to;
    }

    /// Merges one analysis frame: [count >> 20, count & 0xFFFFF, then
    /// (out, pre, gain) per column, oldest first]. While stopped, silent
    /// columns are dropped once the scope rests.
    pub fn push(&mut self, frame: &[f32], now_ms: f64, playing: bool) {
        if frame.len() < 2 {
            self.src_end = None;
            return;
        }
        let total = ((frame[0] as i64) << 20) | frame[1] as i64;
        let data = &frame[2..];
        let n = (data.len() / 3) as i64;
        if self.rests(playing) && !data.chunks_exact(3).any(audible) {
            self.src_end = None;
            return;
        }
        let first = total - n;
        let mut skip = 0;
        match self.src_end {
            Some(src) if total >= src && first >= src - CAP => {
                if first < src {
                    skip = src - first;
                    if skip >= n {
                        self.last_arrival_ms = now_ms;
                        return;
                    }
                } else {
                    let at = first + self.offset;
                    if at < self.end || at > self.end + CAP {
                        self.offset = self.end - first;
                    } else {
                        self.silence_to(at);
                    }
                }
            }
            _ => self.offset = self.end.max(self.head.ceil() as i64) - first,
        }
        for k in skip..n {
            let at = first + k + self.offset;
            let c = &data[3 * k as usize..3 * k as usize + 3];
            let slot = Self::slot(at);
            if !playing && !audible(c) {
                self.cols[slot] = [0.0; 3];
                continue;
            }
            self.cols[slot] = [c[0], c[1], c[2]];
            if c[2].abs() > 0.02 || c[0].abs() > 1e-4 || c[1].abs() > 1e-4 {
                self.last_loud = at as f64;
            }
            if audible(c) {
                self.last_audible = at as f64;
            }
        }
        self.end = self.end.max(first + n + self.offset);
        self.src_end = Some(total);
        self.last_arrival_ms = now_ms;
    }

    /// Moves the read head to the wall clock `now_ms`.
    pub fn advance(&mut self, now_ms: f64) {
        let dt = self.clock_ms.map_or(0.0, |c| (now_ms - c).clamp(0.0, 1000.0));
        self.clock_ms = Some(now_ms);
        let step = dt * RATE / 1000.0;
        if now_ms - self.last_arrival_ms < STALE_MS {
            let lead = self.end as f64 - self.head;
            if (lead - LEAD).abs() > TRIM_SPAN {
                self.head = self.end as f64 - LEAD;
                self.lead = Some(LEAD);
                return;
            }
            let smoothed = self.lead.map_or(lead, |l| l + (lead - l) * (1.0 - (-dt / LEAD_MS).exp()));
            self.lead = Some(smoothed);
            let trim = ((smoothed - LEAD) / TRIM_SPAN).clamp(-TRIM, TRIM);
            self.head = (self.head + step * (1.0 + trim)).min(self.end as f64);
            return;
        }
        self.lead = None;
        self.head += step;
        self.silence_to(self.head.ceil() as i64 + 1);
    }

    /// Stops the clock (the next advance starts from zero elapsed time).
    pub fn park(&mut self) {
        self.clock_ms = None;
    }

    /// Nothing but quiet across the plot.
    pub fn empty(&self) -> bool {
        self.head - self.last_loud > QUIET
    }

    /// Stopped and quiet: the head holds still.
    pub fn rests(&self, playing: bool) -> bool {
        !playing && self.head - self.last_audible > QUIET
    }

    /// Plot values for `width` points (one bar each, plus one entering on
    /// the right), snapped so columns keep their bars while scrolling;
    /// `scale` = device pixels per point, for the sub-point offset.
    pub fn pixels(&self, width: f32, scale: f32) -> Pixels {
        let n = (width.floor() as usize).max(1);
        let r = n + 1;
        let per = SPAN / n as f64;
        let right = (self.head / per).ceil() * per;
        let offset = (((right - self.head) / per * scale as f64).round() / scale as f64) as f32;
        let oldest = self.end - CAP;
        let decay = (-(per * 1000.0 / RATE) / DECAY_MS).exp() as f32;
        let mut px = Pixels { out: Vec::with_capacity(r), pre: Vec::with_capacity(r), gain: Vec::with_capacity(r), offset };
        let (mut out, mut pre) = (0.0f32, 0.0f32);
        for e in 0..r {
            let a = (right - (r - e) as f64 * per).floor() as i64;
            let b = (a + 1).max((right - (r - e - 1) as f64 * per).floor() as i64);
            let (mut o, mut p, mut g) = (0.0f32, 0.0f32, 0.0f32);
            for i in a.max(oldest)..b.min(self.end) {
                let c = self.cols[Self::slot(i)];
                o = o.max(c[0]);
                p = p.max(c[1]);
                if c[2].abs() > g.abs() {
                    g = c[2];
                }
            }
            out = o.max(out * decay);
            pre = p.max(pre * decay);
            px.out.push(out);
            px.pre.push(pre);
            px.gain.push(g);
        }
        px
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(total: i64, cols: &[[f32; 3]]) -> Vec<f32> {
        let mut f = vec![(total >> 20) as f32, (total & 0xFFFFF) as f32];
        f.extend(cols.iter().flatten());
        f
    }

    #[test]
    fn overlapping_frames_merge_by_count() {
        let mut h = ScopeHistory::default();
        let a: Vec<[f32; 3]> = (0..100).map(|i| [i as f32 / 100.0, 0.0, 0.0]).collect();
        h.push(&frame(100, &a), 0.0, true);
        // The next frame repeats 60 columns and adds 40.
        let b: Vec<[f32; 3]> = (60..140).map(|i| [i as f32 / 200.0, 0.0, 0.0]).collect();
        h.push(&frame(140, &b), 50.0, true);
        assert_eq!(h.end - h.offset, 140);
        assert_eq!(h.cols[ScopeHistory::slot(h.offset + 99)][0], 0.99);
        assert_eq!(h.cols[ScopeHistory::slot(h.offset + 139)][0], 139.0 / 200.0);
    }

    #[test]
    fn head_runs_at_750_columns_a_second_behind_the_lead() {
        let mut h = ScopeHistory::default();
        let mut total = 0;
        for k in 0..200 {
            let now = k as f64 * 30.0;
            total += 22 + (k % 2) as i64;
            let cols = vec![[0.5, 0.5, -3.0]; 256.min(total as usize)];
            h.push(&frame(total, &cols), now, true);
            h.advance(now);
        }
        let lead = h.end as f64 - h.head;
        assert!((lead - LEAD).abs() < 30.0, "lead {lead}");
        let before = h.head;
        h.advance(199.0 * 30.0 + 100.0);
        assert!((h.head - before - 75.0).abs() < 5.0, "moved {}", h.head - before);
    }

    #[test]
    fn pixels_hold_peaks_and_the_largest_gain() {
        let mut h = ScopeHistory::default();
        let mut cols = vec![[0.1f32, 0.1, 1.0]; 256];
        cols[200] = [0.9, 1.2, -6.0];
        h.push(&frame(256, &cols), 0.0, true);
        h.head = 256.0;
        let px = h.pixels(300.0, 2.0);
        assert_eq!(px.out.len(), 301);
        assert!(px.out.iter().any(|&o| o == 0.9));
        assert!(px.pre.iter().any(|&p| p == 1.2));
        assert!(px.gain.iter().any(|&g| g == -6.0));
        assert_eq!(px.out[0], 0.0);
    }

    #[test]
    fn rests_when_stopped_and_quiet() {
        let mut h = ScopeHistory::default();
        let quiet = vec![[0.0f32, 0.0, 0.0]; 256];
        h.push(&frame(256, &quiet), 0.0, false);
        assert!(h.rests(false));
        assert!(h.src_end.is_none());
        assert!(h.empty());
    }
}
