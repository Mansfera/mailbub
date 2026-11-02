//! Right-clicks inside an email frame. WebKit never runs the page's listeners in the
//! script-less frame and shows its own context menu there, so the core empties that menu
//! while the page says the pointer is over an email, and asks the page for the message menu.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set by the page while the pointer is over an email frame.
static OVER_MAIL_FRAME: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub fn set_mail_frame_hover(over: bool) {
    OVER_MAIL_FRAME.store(over, Ordering::Relaxed);
}

#[cfg(target_os = "macos")]
pub use macos::install;

#[cfg(not(target_os = "macos"))]
pub fn install(_window: &tauri::WebviewWindow) {}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::OnceLock;
    use std::sync::atomic::Ordering;

    use objc2::runtime::{AnyObject, Imp, Sel};
    use objc2::{msg_send, sel};
    use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

    use super::OVER_MAIL_FRAME;

    static APP: OnceLock<AppHandle> = OnceLock::new();
    /// The inherited `willOpenMenu:withEvent:` (NSView's or WKWebView's).
    static INHERITED: OnceLock<Imp> = OnceLock::new();

    type WillOpenMenu =
        unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject);

    unsafe extern "C-unwind" fn will_open_menu(
        this: &AnyObject,
        cmd: Sel,
        menu: *mut AnyObject,
        event: *mut AnyObject,
    ) {
        if let Some(imp) = INHERITED.get() {
            // SAFETY: `imp` is this selector's inherited implementation, same signature.
            let inherited: WillOpenMenu = unsafe { std::mem::transmute(*imp) };
            unsafe { inherited(this, cmd, menu, event) };
        }
        if !OVER_MAIL_FRAME.load(Ordering::Relaxed) || menu.is_null() {
            return;
        }
        // An empty NSMenu is not shown.
        let _: () = unsafe { msg_send![menu, removeAllItems] };
        if let Some(app) = APP.get() {
            let _ = app.emit("mail-frame-menu", ());
        }
    }

    /// Adds the override to wry's WKWebView subclass (shared by every window).
    pub fn install(window: &WebviewWindow) {
        if APP.set(window.app_handle().clone()).is_err() {
            return;
        }
        let _ = window.with_webview(|webview| {
            let view = webview.inner() as *const AnyObject;
            if view.is_null() {
                return;
            }
            let sel = sel!(willOpenMenu:withEvent:);
            // SAFETY: `view` is the live WKWebView; adding a method to its class is
            // what Objective-C subclasses do, and the signature matches the selector.
            unsafe {
                let class = (*view).class();
                if let Some(method) = class.instance_method(sel) {
                    let _ = INHERITED.set(method.implementation());
                }
                let imp: Imp = std::mem::transmute(will_open_menu as WillOpenMenu);
                objc2::ffi::class_addMethod(
                    class as *const _ as *mut _,
                    sel,
                    imp,
                    c"v@:@@".as_ptr(),
                );
            }
        });
    }
}
