//! macOS app menu. Slint's winit backend builds the app menu itself (About, Services, Hide,
//! Quit) and builds it again whenever the menu bar changes, with no way to add items to it.
//! The first menu of our MenuBar ("Device Studio") holds the items that belong there
//! (Settings…). After each rebuild, a copy of each goes into the app menu below About,
//! followed by a separator, and the holder is hidden. A
//! copy targets the original item, so choosing it runs the original's Slint callback; the
//! originals stay in the holder because the next rebuild removes them from there.

#[cfg(target_os = "macos")]
const HOLDER: &str = "Device Studio";

/// Polls cheaply: nothing happens unless the menu bar was rebuilt since the last call.
#[cfg(target_os = "macos")]
pub fn adopt_items() {
    use objc2::runtime::{AnyClass, AnyObject, Sel};
    use objc2::{class, msg_send};
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let bar: *mut AnyObject = msg_send![app, mainMenu];
        if bar.is_null() {
            return;
        }
        let count: isize = msg_send![bar, numberOfItems];
        if count < 2 {
            return;
        }
        let holder: *mut AnyObject = msg_send![bar, itemAtIndex: 1isize];
        let hidden: bool = msg_send![holder, isHidden];
        let title: *mut AnyObject = msg_send![holder, title];
        let title: *const std::ffi::c_char = msg_send![title, UTF8String];
        if hidden || title.is_null() || std::ffi::CStr::from_ptr(title).to_bytes() != HOLDER.as_bytes() {
            return;
        }
        let app_item: *mut AnyObject = msg_send![bar, itemAtIndex: 0isize];
        let app_menu: *mut AnyObject = msg_send![app_item, submenu];
        let items: *mut AnyObject = msg_send![holder, submenu];
        if app_menu.is_null() || items.is_null() {
            return;
        }
        let item_class: &AnyClass = class!(NSMenuItem);
        let n: isize = msg_send![items, numberOfItems];
        let mut at: isize = 2;
        for i in 0..n {
            let original: *mut AnyObject = msg_send![items, itemAtIndex: i];
            let title: *mut AnyObject = msg_send![original, title];
            let action: Option<Sel> = msg_send![original, action];
            let key: *mut AnyObject = msg_send![original, keyEquivalent];
            let copy: *mut AnyObject = msg_send![item_class, alloc];
            let copy: *mut AnyObject = msg_send![copy, initWithTitle: title, action: action, keyEquivalent: key];
            let mask: usize = msg_send![original, keyEquivalentModifierMask];
            let enabled: bool = msg_send![original, isEnabled];
            let _: () = msg_send![copy, setKeyEquivalentModifierMask: mask];
            let _: () = msg_send![copy, setTarget: original];
            let _: () = msg_send![copy, setEnabled: enabled];
            let _: () = msg_send![app_menu, insertItem: copy, atIndex: at];
            let _: () = msg_send![copy, release];
            at += 1;
        }
        let separator: *mut AnyObject = msg_send![item_class, separatorItem];
        let _: () = msg_send![app_menu, insertItem: separator, atIndex: at];
        let _: () = msg_send![holder, setHidden: true];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn adopt_items() {}
