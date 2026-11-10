//! Interface zoom (View › Zoom In / Zoom Out / Reset Zoom in the reference).
//!
//! The geometry in `ui/*.slint` and the Rust-side layout constants were measured
//! from reference captures taken at the reference's **85 %** interface zoom (found
//! on 2026-11-09 by comparing a live capture at its default zoom with our render:
//! the best whole-UI scale was 1.175 ≈ 1 / 0.85, and three 5 % Zoom Out steps from
//! the default reproduce the old captures). So our design units are "85 % units",
//! and 100 % — the reference's default — draws them `1 / 0.85` larger.
//!
//! Zoom is applied as a Slint scale factor: `native display scale × level / 0.85`.
//! Everything (Slint layout, input mapping and the wgpu timeline/piano-roll
//! textures, which all read `window().scale_factor()`) follows it, and the OS
//! window keeps its size, so content reflows exactly like the reference's zoom.

use slint::platform::WindowEvent;

/// Interface zoom our design units were measured at.
pub const DESIGN_LEVEL: f32 = 0.85;
/// Default level (the reference's Reset Zoom).
pub const DEFAULT_PERCENT: u32 = 100;
/// One Zoom In / Zoom Out step (the reference's step, measured from track-row heights
/// 88 → 83.5 → 79 pt).
pub const STEP_PERCENT: u32 = 5;
/// Our bounds (the reference's limits are unmeasured).
pub const MIN_PERCENT: u32 = 50;
pub const MAX_PERCENT: u32 = 200;

/// Interface zoom state for one window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zoom {
    /// Current level in percent.
    pub percent: u32,
    /// The display's own scale factor (2.0 on a Retina display). Re-learned whenever
    /// the OS reports a different scale (the window moved to another display).
    native: f32,
    /// The factor we last dispatched; anything else seen on the window came from the OS.
    applied: f32,
    /// A deferred `sync` is scheduled (see `needs_sync`).
    pub pending: bool,
}

impl Zoom {
    pub fn new(percent: u32) -> Zoom {
        Zoom { percent: clamp(percent), native: 0.0, applied: 0.0, pending: false }
    }

    /// Slint scale factor for `percent` on a display with `native` scale.
    pub fn factor(native: f32, percent: u32) -> f32 {
        native * percent as f32 / 100.0 / DESIGN_LEVEL
    }

    pub fn step(&mut self, delta: i32) {
        self.percent = clamp((self.percent as i32 + delta * STEP_PERCENT as i32).max(0) as u32);
    }

    pub fn reset(&mut self) {
        self.percent = DEFAULT_PERCENT;
    }

    /// True when the window's scale factor is not the one we applied: the window was just
    /// created, or the backend dispatched a new native scale (the window moved to another
    /// display). One float compare, checked before every frame; the fix-up (`sync`) is
    /// deferred out of the render callback.
    pub fn needs_sync(&self, current: f32) -> bool {
        !self.pending && (self.applied == 0.0 || (current - self.applied).abs() > 1e-4)
    }

    /// Bring the window's scale factor in line with the zoom level, keeping the OS
    /// window's size. Returns true when it dispatched a new factor.
    pub fn sync(&mut self, window: &slint::Window) -> bool {
        self.pending = false;
        let current = window.scale_factor();
        if self.native == 0.0 || (current - self.applied).abs() > 1e-4 {
            // First call, or the OS replaced our factor: what we see is the native scale.
            self.native = current;
        }
        let want = Zoom::factor(self.native, self.percent);
        if (current - want).abs() <= 1e-4 {
            self.applied = current;
            return false;
        }
        self.applied = want;
        // Keep the OS window's size: only the content scale changes (the backend would
        // otherwise keep the logical size and grow the window).
        let physical = window.size();
        window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: want });
        window.set_size(physical);
        // The OS size did not change, so no resize event follows: tell Slint the new
        // logical size so the layout reflows now.
        window.dispatch_event(WindowEvent::Resized { size: physical.to_logical(want) });
        true
    }
}

fn clamp(p: u32) -> u32 {
    p.clamp(MIN_PERCENT, MAX_PERCENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_level_scales_design_units_up() {
        let f = Zoom::factor(2.0, DEFAULT_PERCENT);
        assert!((f - 2.0 / 0.85).abs() < 1e-6);
        // 85 % reproduces the design units exactly.
        assert!((Zoom::factor(2.0, 85) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn steps_are_clamped() {
        let mut z = Zoom::new(100);
        z.step(1);
        assert_eq!(z.percent, 105);
        z.step(-3);
        assert_eq!(z.percent, 90);
        for _ in 0..100 {
            z.step(-1);
        }
        assert_eq!(z.percent, MIN_PERCENT);
        for _ in 0..100 {
            z.step(1);
        }
        assert_eq!(z.percent, MAX_PERCENT);
        z.reset();
        assert_eq!(z.percent, 100);
    }
}
