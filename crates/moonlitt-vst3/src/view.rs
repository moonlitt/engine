//! VST3 plug-in editor view.
//!
//! Wraps `IEditController::createView("editor")` and the `IPlugView`
//! attach/detach lifecycle. The host gives the plugin a native parent
//! (NSView on macOS, HWND on Windows, X11 window id on Linux); the
//! plugin draws into it and handles its own input.
//!
//! Pure Rust, no GUI runtime dependency — the *caller* (CLI / app) is
//! responsible for opening the OS window. This module just hands the
//! plugin's view to whatever native parent the caller provides.

use std::ffi::{c_void, CStr};

use vst3::ComPtr;
use vst3::Steinberg::Vst::IEditControllerTrait;
use vst3::Steinberg::{kResultOk, IPlugView, IPlugViewTrait, ViewRect};

use crate::{Error, Result, Vst3Plugin};

/// Platform-type strings recognised by `IPlugView::attached`/`isPlatformTypeSupported`.
pub mod platform {
    pub const NS_VIEW: &str = "NSView"; // macOS Cocoa
    pub const HWND: &str = "HWND"; // Windows
    pub const X11_EMBED_WINDOW_ID: &str = "X11EmbedWindowID"; // Linux X11
}

/// Wrapper around an `IPlugView` returned by `IEditController::createView`.
pub struct Vst3PluginView {
    view: ComPtr<IPlugView>,
}

impl Vst3PluginView {
    /// Returns true if the plugin can attach to a parent of the given platform type.
    pub fn is_platform_supported(&self, platform_type: &str) -> bool {
        with_cstr(platform_type, |c| unsafe {
            self.view.isPlatformTypeSupported(c.as_ptr() as _) == kResultOk
        })
    }

    /// Initial size hint, in points. Some plugins don't implement this and
    /// return non-OK; in that case we fall back to (640, 480).
    pub fn get_size(&self) -> (i32, i32) {
        let mut rect = ViewRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let r = unsafe { self.view.getSize(&mut rect) };
        if r != kResultOk || rect.right <= rect.left || rect.bottom <= rect.top {
            return (640, 480);
        }
        (rect.right - rect.left, rect.bottom - rect.top)
    }

    /// Embed the plugin's editor inside `parent`. The parent must be of the
    /// appropriate platform type (`platform::NS_VIEW` on macOS, etc.) — pass
    /// the OS handle (NSView*, HWND, X11 window id) cast to `*mut c_void`.
    pub fn attach(&self, parent: *mut c_void, platform_type: &str) -> Result<()> {
        with_cstr(platform_type, |c| {
            let r = unsafe { self.view.attached(parent, c.as_ptr() as _) };
            if r != kResultOk {
                return Err(Error::PluginError(r));
            }
            Ok(())
        })
    }

    /// Detach from the parent. Should be called before the parent is destroyed.
    pub fn detach(&self) -> Result<()> {
        let r = unsafe { self.view.removed() };
        if r != kResultOk {
            return Err(Error::PluginError(r));
        }
        Ok(())
    }

    /// Tell the plugin its frame was resized to `(width, height)` points.
    pub fn on_size(&self, width: i32, height: i32) -> Result<()> {
        let mut rect = ViewRect {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        let r = unsafe { self.view.onSize(&mut rect) };
        if r != kResultOk {
            return Err(Error::PluginError(r));
        }
        Ok(())
    }
}

impl Vst3Plugin {
    /// Create the plugin's editor view, if it exposes one. Returns `None` for
    /// plug-ins that ship without a GUI (rare for instrument VST3s) or whose
    /// `IEditController::createView` returns null.
    pub fn create_view(&self) -> Option<Vst3PluginView> {
        let ctrl = self.inner.controller.as_ref()?;
        let raw = with_cstr("editor", |c| unsafe { ctrl.createView(c.as_ptr() as _) });
        if raw.is_null() {
            return None;
        }
        let view = unsafe { ComPtr::<IPlugView>::from_raw(raw) }?;
        Some(Vst3PluginView { view })
    }
}

/// Helper to safely build a NUL-terminated C string from `&str` for the
/// VST3 API, without allocating heap memory in the hot path.
fn with_cstr<R>(s: &str, f: impl FnOnce(&CStr) -> R) -> R {
    use std::ffi::CString;
    let c = CString::new(s).expect("platform type contains nul byte");
    f(&c)
}
