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
use std::sync::Mutex;

use vst3::Steinberg::Vst::IEditControllerTrait;
use vst3::Steinberg::{
    kInvalidArgument, kResultOk, kResultTrue, IPlugFrame, IPlugFrameTrait, IPlugView,
    IPlugViewContentScaleSupport, IPlugViewContentScaleSupportTrait, IPlugViewTrait, ViewRect,
};
use vst3::{Class, ComPtr, ComRef, ComWrapper};

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
    /// Host-side `IPlugFrame` handed to the plug-in via `setFrame`.
    /// Kept alive here for the view's lifetime; cleared on detach.
    frame: Mutex<Option<ComWrapper<PlugFrame>>>,
}

/// Host-side `IPlugFrame`: receives the plug-in's resize requests.
///
/// Per the VST3 workflow the host resizes its window in `resizeView`
/// and then confirms the final size back through `IPlugView::onSize`.
/// Plug-ins that report a too-small pre-attach size (Spectrasonics)
/// depend on this to reach their real editor size.
struct PlugFrame {
    on_resize: Box<dyn Fn(i32, i32) + Send + Sync>,
}

impl Class for PlugFrame {
    type Interfaces = (IPlugFrame,);
}

impl IPlugFrameTrait for PlugFrame {
    unsafe fn resizeView(&self, view: *mut IPlugView, new_size: *mut ViewRect) -> vst3::Steinberg::tresult {
        if new_size.is_null() {
            return kInvalidArgument;
        }
        let rect = *new_size;
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        crate::trace::emit(&format!("PlugFrame::resizeView {w}x{h}"));
        (self.on_resize)(w, h);
        // Confirm the final size back to the plug-in.
        if let Some(view) = ComRef::from_raw(view) {
            let mut confirmed = rect;
            view.onSize(&mut confirmed);
        }
        kResultOk
    }
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
        // Clear the frame first so the plug-in can't call back into a
        // host window that is going away.
        unsafe { self.view.setFrame(std::ptr::null_mut()) };
        *self.frame.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let r = unsafe { self.view.removed() };
        if r != kResultOk {
            return Err(Error::PluginError(r));
        }
        Ok(())
    }

    /// Does the plug-in support live resizing of its editor?
    pub fn can_resize(&self) -> bool {
        unsafe { self.view.canResize() == kResultTrue }
    }

    /// Ask the plug-in to adjust a prospective size to one it accepts.
    /// Returns the input unchanged when the plug-in has no opinion.
    pub fn check_size_constraint(&self, width: i32, height: i32) -> (i32, i32) {
        let mut rect = ViewRect {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        let r = unsafe { self.view.checkSizeConstraint(&mut rect) };
        if r != kResultOk || rect.right <= rect.left || rect.bottom <= rect.top {
            return (width, height);
        }
        (rect.right - rect.left, rect.bottom - rect.top)
    }

    /// Install a host frame so the plug-in can request editor resizes
    /// (`IPlugFrame::resizeView`). `on_resize(width, height)` must
    /// resize the native window that hosts the view; the final size is
    /// confirmed back to the plug-in automatically. Call before
    /// [`Self::attach`] so resize requests during attach are honoured.
    pub fn set_frame(&self, on_resize: impl Fn(i32, i32) + Send + Sync + 'static) -> Result<()> {
        let wrapper = ComWrapper::new(PlugFrame {
            on_resize: Box::new(on_resize),
        });
        let ptr = wrapper
            .to_com_ptr::<IPlugFrame>()
            .ok_or(Error::InterfaceNotFound("IPlugFrame"))?;
        let r = unsafe { self.view.setFrame(ptr.as_ptr()) };
        if r != kResultOk {
            return Err(Error::PluginError(r));
        }
        *self.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some(wrapper);
        Ok(())
    }

    /// Tell the plug-in the host window's backing scale (2.0 on Retina)
    /// so it renders crisply. Best-effort: plug-ins without
    /// `IPlugViewContentScaleSupport` ignore this.
    pub fn set_content_scale(&self, factor: f32) {
        if let Some(scale) = self.view.cast::<IPlugViewContentScaleSupport>() {
            unsafe { scale.setContentScaleFactor(factor) };
        }
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
        Some(Vst3PluginView {
            view,
            frame: Mutex::new(None),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The PlugFrame COM object must translate ViewRect into a
    /// (width, height) callback — including non-zero left/top origins.
    #[test]
    fn plug_frame_reports_dimensions_not_corners() {
        let captured = std::sync::Arc::new(Mutex::new(None));
        let c2 = captured.clone();
        let wrapper = ComWrapper::new(PlugFrame {
            on_resize: Box::new(move |w, h| {
                *c2.lock().unwrap() = Some((w, h));
            }),
        });
        let frame = wrapper.to_com_ptr::<IPlugFrame>().expect("IPlugFrame");
        let mut rect = ViewRect {
            left: 10,
            top: 20,
            right: 1103,
            bottom: 738,
        };
        let r = unsafe { frame.resizeView(std::ptr::null_mut(), &mut rect) };
        assert_eq!(r, kResultOk);
        assert_eq!(*captured.lock().unwrap(), Some((1093, 718)));
    }

    #[test]
    fn plug_frame_rejects_null_rect() {
        let wrapper = ComWrapper::new(PlugFrame {
            on_resize: Box::new(|_, _| {}),
        });
        let frame = wrapper.to_com_ptr::<IPlugFrame>().expect("IPlugFrame");
        let r = unsafe { frame.resizeView(std::ptr::null_mut(), std::ptr::null_mut()) };
        assert_eq!(r, kInvalidArgument);
    }
}

/// Helper to safely build a NUL-terminated C string from `&str` for the
/// VST3 API, without allocating heap memory in the hot path.
fn with_cstr<R>(s: &str, f: impl FnOnce(&CStr) -> R) -> R {
    use std::ffi::CString;
    let c = CString::new(s).expect("platform type contains nul byte");
    f(&c)
}
