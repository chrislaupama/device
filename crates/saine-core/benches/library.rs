//! Pattern library costs on the click path: one Create builds a set of six
//! patterns (and the sidebar rebuilds the shown set's previews from it).
//! Prints medians; no thresholds.
//!
//!   cargo bench -p saine-core --bench library

use saine_core::library::{self, PartType};
use saine_core::PPQ;
use std::hint::black_box;
use std::time::Instant;

fn median_us(runs: usize, mut f: impl FnMut()) -> (f64, f64) {
    let mut t: Vec<f64> = (0..runs)
        .map(|_| {
            let s = Instant::now();
            f();
            s.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    t.sort_by(f64::total_cmp);
    (t[runs / 2], t[runs * 99 / 100])
}

fn main() {
    println!("{:<30} {:>10} {:>10}", "set of six", "median us", "p99 us");
    for part in PartType::ALL {
        for bars in [1, 4] {
            let (m, p) = median_us(2000, || {
                black_box(library::create_set(part, 3, None, 1, bars * 4 * PPQ, 4 * PPQ));
            });
            println!("{:<30} {m:>10.1} {p:>10.1}", format!("{} ({bars} bar{})", part.label(), if bars > 1 { "s" } else { "" }));
        }
    }
    let (m, p) = median_us(2000, || {
        black_box(library::create_drum_set(10, 1));
    });
    println!("{:<30} {m:>10.1} {p:>10.1}", "Drums (16 steps)");
}
