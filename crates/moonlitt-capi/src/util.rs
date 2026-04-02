//! FFI utility helpers — C string conversions, JSON serialization, string cleanup.

use std::ffi::{c_char, CStr, CString};

/// Convert a C string pointer to `&str`.
/// Returns `None` if the pointer is null or contains invalid UTF-8.
pub(crate) unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok()
}

/// Allocate a heap C string from a Rust `&str`.
/// Caller must free with `moonlitt_free_string`.
/// Returns null on allocation failure.
pub(crate) fn to_c_string(s: &str) -> *mut c_char {
    CString::new(s)
        .map(|cs| cs.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// Free a string previously returned by any `moonlitt_*` function that
/// documents "caller must free with `moonlitt_free_string`".
#[no_mangle]
pub extern "C" fn moonlitt_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            drop(CString::from_raw(s));
        }
    }
}

/// Log a warning in debug builds when a MIDI parameter is outside its valid range.
/// In release builds this is a no-op (zero cost).
#[inline]
pub(crate) fn debug_warn_midi_range(
    _func: &str,
    _param: &str,
    _value: std::ffi::c_int,
    _min: std::ffi::c_int,
    _max: std::ffi::c_int,
) {
    #[cfg(debug_assertions)]
    if _value < _min || _value > _max {
        eprintln!(
            "[moonlitt] warning: {}.{} = {} out of range [{}..{}], clamped",
            _func, _param, _value, _min, _max
        );
    }
}

/// Escape a string for safe embedding in JSON.
pub(crate) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\x20' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}
