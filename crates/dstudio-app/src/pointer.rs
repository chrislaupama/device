//! Pointer hide and warp for minimap drags.
//!
//! While the user drags a minimap window the pointer is hidden, and on release it is put back
//! on the strip where the grabbed spot ended up. Slint has no pointer warp, so the controller
//! queues Ops and `apply` runs them against the winit window after each UI update. The pointer
//! is not pinned: Slint keeps reporting absolute positions while it is hidden, so the drag
//! maths needs no raw deltas. Hiding uses NSCursor on macOS and winit's cursor visibility
//! elsewhere; warping is winit's (Core Graphics on macOS, SetCursorPos on Windows, X11 warp;
//! Wayland refuses it and the pointer then simply reappears where the user left it).

use crate::AppWindow;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

/// One request to the OS, in Slint logical window coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    Hide,
    Show,
    Warp(f32, f32),
}

/// Pending requests and whether the pointer is currently hidden by us.
#[derive(Debug, Default)]
pub struct Queue {
    ops: Vec<Op>,
    hidden: bool,
}

impl Queue {
    pub fn hide(&mut self) {
        if !self.hidden {
            self.hidden = true;
            self.ops.push(Op::Hide);
        }
    }

    /// Show the pointer again, first moving it to `warp` when given. Does nothing unless it is hidden.
    pub fn release(&mut self, warp: Option<(f32, f32)>) {
        if !self.hidden {
            return;
        }
        self.hidden = false;
        if let Some((x, y)) = warp {
            self.ops.push(Op::Warp(x, y));
        }
        self.ops.push(Op::Show);
    }

    pub fn hidden(&self) -> bool {
        self.hidden
    }

    pub fn take(&mut self) -> Vec<Op> {
        std::mem::take(&mut self.ops)
    }
}

static SIMULATE: AtomicBool = AtomicBool::new(false);

thread_local! {
    static LOG: RefCell<Vec<Op>> = const { RefCell::new(Vec::new()) };
}

/// Self-test mode: ops are recorded (see `take_log`) instead of touching the real pointer.
pub fn simulate(on: bool) {
    SIMULATE.store(on, Ordering::Relaxed);
}

/// The ops `apply` recorded since the last call (self-test mode only).
pub fn take_log() -> Vec<Op> {
    LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

pub fn apply(app: &AppWindow, ops: Vec<Op>) {
    if ops.is_empty() {
        return;
    }
    if SIMULATE.load(Ordering::Relaxed) {
        LOG.with(|l| l.borrow_mut().extend(ops));
        return;
    }
    use slint::ComponentHandle;
    use slint::winit_030::WinitWindowAccessor;
    let scale = app.window().scale_factor() as f64;
    app.window().with_winit_window(|w| {
        for op in ops {
            match op {
                Op::Hide => set_visible(w, false),
                Op::Show => set_visible(w, true),
                Op::Warp(x, y) => {
                    let _ = w.set_cursor_position(slint::winit_030::winit::dpi::PhysicalPosition::new(x as f64 * scale, y as f64 * scale));
                }
            }
        }
    });
}

#[cfg(target_os = "macos")]
fn set_visible(_: &slint::winit_030::winit::window::Window, visible: bool) {
    use objc2::{class, msg_send};
    unsafe {
        if visible {
            let _: () = msg_send![class!(NSCursor), unhide];
        } else {
            let _: () = msg_send![class!(NSCursor), hide];
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn set_visible(w: &slint::winit_030::winit::window::Window, visible: bool) {
    w.set_cursor_visible(visible);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hide_and_release_are_balanced() {
        let mut q = Queue::default();
        q.hide();
        q.hide();
        assert!(q.hidden());
        q.release(Some((10.0, 20.0)));
        q.release(Some((99.0, 99.0)));
        assert!(!q.hidden());
        assert_eq!(q.take(), vec![Op::Hide, Op::Warp(10.0, 20.0), Op::Show]);
        q.release(None);
        assert!(q.take().is_empty());
    }

    #[test]
    fn release_without_a_warp_only_shows() {
        let mut q = Queue::default();
        q.hide();
        q.release(None);
        assert_eq!(q.take(), vec![Op::Hide, Op::Show]);
    }
}
