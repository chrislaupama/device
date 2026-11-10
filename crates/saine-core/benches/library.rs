//! Pattern library costs on the click path: one Create builds a set of six
//! patterns (and the sidebar rebuilds the shown set's previews from it), and
//! each Shape edit (a slider drag runs one per move) reshapes the playing pattern.
//! Prints medians; no thresholds.
//!
//!   cargo bench -p saine-core --bench library

use saine_core::library::{self, PartType};
use saine_core::shape::{self, Curve, Shape, ShapeKey};
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

    println!("{:<30} {:>10} {:>10}", "Shape (4 bars)", "median us", "p99 us");
    let clip = 16 * PPQ;
    let curve = |k: ShapeKey| Curve { key: k, points: (0..shape::curve_len(clip)).map(|i| ((i % 4) as f32 - 1.5) / 4.0).collect() };
    for (name, part, f) in [
        ("Chords: octave, density, length", PartType::Chords, (|s: Shape| Shape { octave: 1, density: 0.6, length: 0.3, ..s }) as fn(Shape) -> Shape),
        ("Chords: repeat 16", PartType::Chords, |s| Shape { repeat: 3, ..s }),
        ("Chords: arp 1/16", PartType::Chords, |s| Shape { arp: 3, ..s }),
        ("Chords: air, sub, tension, spread", PartType::Chords, |s| Shape { air: 0.67, sub: 0.34, tension: 0.4, spread: 0.5, ..s }),
        ("Chords: density, vary", PartType::Chords, |s| Shape { density: 0.5, vary: 0.5, ..s }),
        ("Lead: 3 voices, nudge", PartType::Lead, |s| Shape { voices: 3, nudge: 0.6, ..s }),
    ] {
        let notes = library::create_set(part, 3, None, 1, clip, 4 * PPQ).remove(0).notes;
        let s = f(Shape::neutral(Some(part)));
        let (m, p) = median_us(2000, || {
            black_box(shape::apply(&notes, &s, Some(part), None, clip));
        });
        println!("{:<30} {m:>10.1} {p:>10.1}", format!("{name} ({} notes)", notes.len()));
    }
    let notes = library::create_set(PartType::Chords, 3, None, 1, clip, 4 * PPQ).remove(0).notes;
    let curves = Shape {
        curves: [ShapeKey::Velocity, ShapeKey::Density, ShapeKey::Length, ShapeKey::Nudge, ShapeKey::Air, ShapeKey::Tension].map(curve).to_vec(),
        ..Shape::neutral(Some(PartType::Chords))
    };
    for (name, s) in [("Chords: six drawn curves", curves.clone()), ("Chords: six curves, arp 1/16", Shape { arp: 3, ..curves })] {
        let (m, p) = median_us(2000, || {
            black_box(shape::apply(&notes, &s, Some(PartType::Chords), None, clip));
        });
        println!("{:<30} {m:>10.1} {p:>10.1}", format!("{name} ({} notes)", notes.len()));
    }
}
