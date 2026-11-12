//! The main window's close, minimise and zoom buttons (macOS).
//!
//! The window has a transparent, full-size titlebar, so AppKit draws its standard
//! buttons over the header. Left alone, AppKit puts them 9 pt from the window's
//! top-left corner, tight against the edge and above the header's title row.
//! Device Studio centres them on the title row instead: the close button's centre
//! sits 24 pt in from the left and top edges at 100 % interface zoom, and the
//! inset follows the zoom like the rest of the header.
//!
//! AppKit lays the titlebar out again when the window resizes, changes display or
//! focus, or leaves full screen, so `place` runs again on those window events. It
//! only reads frames when the buttons are already where they belong.

/// Centre of the close button from the window's left and top edges, in points at
/// 100 % interface zoom (the header's title row is centred 23.5 pt down).
pub const CLOSE_CENTER_PT: f64 = 24.0;

/// Centre of the close button (x from the left edge, y from the top edge), in
/// points, for an interface zoom `percent`.
pub fn close_center(percent: u32) -> (f64, f64) {
    let c = CLOSE_CENTER_PT * percent as f64 / 100.0;
    (c, c)
}

/// Top-left corner of a `w` x `h` button centred on `center`, on whole points.
pub fn button_origin(center: (f64, f64), w: f64, h: f64) -> (f64, f64) {
    ((center.0 - w / 2.0).round(), (center.1 - h / 2.0).round())
}

/// Moves the window buttons to their place for the interface zoom `percent`.
/// Does nothing in full screen (AppKit shows the buttons in the menu-bar overlay).
#[cfg(target_os = "macos")]
pub fn place(window: &slint::Window, percent: u32) {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    window.with_winit_window(|w| {
        let Ok(handle) = w.window_handle() else { return };
        if let RawWindowHandle::AppKit(h) = handle.as_raw() {
            // SAFETY: winit hands out the window's live NSView; everything below runs
            // on the main thread (the event loop) and only messages AppKit objects
            // reached from that view.
            unsafe { mac::place(h.ns_view.as_ptr().cast(), close_center(percent)) };
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn place(_window: &slint::Window, _percent: u32) {}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSPoint, NSRect};

    /// NSWindowStyleMaskFullScreen.
    const FULL_SCREEN: usize = 1 << 14;
    /// NSWindowCloseButton, NSWindowMiniaturizeButton, NSWindowZoomButton.
    const BUTTONS: [usize; 3] = [0, 1, 2];

    /// Moves differences under half a point are left alone (no relayout churn).
    fn differs(a: f64, b: f64) -> bool {
        (a - b).abs() > 0.5
    }

    pub(super) unsafe fn place(view: *mut AnyObject, center: (f64, f64)) {
        unsafe {
            let window: *mut AnyObject = msg_send![view, window];
            if window.is_null() {
                return;
            }
            let mask: usize = msg_send![window, styleMask];
            if mask & FULL_SCREEN != 0 {
                return;
            }
            let buttons = BUTTONS.map(|kind| -> *mut AnyObject { msg_send![window, standardWindowButton: kind] });
            if buttons.iter().any(|b| b.is_null()) {
                return;
            }
            let close: NSRect = msg_send![buttons[0], frame];
            let mini: NSRect = msg_send![buttons[1], frame];
            // Keep AppKit's own spacing between the buttons (it differs between macOS versions).
            let step = mini.origin.x - close.origin.x;
            let (w, h) = (close.size.width, close.size.height);
            let (left, top) = super::button_origin(center, w, h);

            // The titlebar (NSTitlebarContainerView, the buttons' grandparent) must be tall
            // enough to hold the buttons, or clicks below its edge miss them. Give it the
            // same margin under the buttons as above.
            let bar: *mut AnyObject = msg_send![buttons[0], superview];
            if bar.is_null() {
                return;
            }
            let container: *mut AnyObject = msg_send![bar, superview];
            if container.is_null() {
                return;
            }
            let win_frame: NSRect = msg_send![window, frame];
            let height = top * 2.0 + h;
            let mut c: NSRect = msg_send![container, frame];
            if differs(c.size.height, height) || differs(c.origin.y, win_frame.size.height - height) {
                c.size.height = height;
                c.origin.y = win_frame.size.height - height;
                let _: () = msg_send![container, setFrame: c];
            }

            // Button rects in window coordinates (origin at the bottom-left corner),
            // converted into the titlebar view's own (possibly flipped) coordinates.
            for (i, button) in buttons.iter().enumerate() {
                let x = left + step * i as f64;
                let want = NSRect::new(NSPoint::new(x, win_frame.size.height - top - h), close.size);
                let want: NSRect = msg_send![bar, convertRect: want, fromView: std::ptr::null_mut::<AnyObject>()];
                let now: NSRect = msg_send![*button, frame];
                if differs(now.origin.x, want.origin.x) || differs(now.origin.y, want.origin.y) {
                    let _: () = msg_send![*button, setFrameOrigin: want.origin];
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_button_sits_17_pt_in_at_100_percent() {
        // A 14 pt button centred 24 pt from the corner: 17 pt clear of both edges.
        assert_eq!(button_origin(close_center(100), 14.0, 14.0), (17.0, 17.0));
    }

    #[test]
    fn inset_follows_interface_zoom() {
        assert_eq!(close_center(200), (48.0, 48.0));
        assert_eq!(close_center(50), (12.0, 12.0));
        // Rounded to whole points at in-between levels.
        assert_eq!(button_origin(close_center(85), 14.0, 14.0), (13.0, 13.0));
    }
}
