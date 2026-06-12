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

/// Free a binary buffer previously returned by `moonlitt_engine_save_state`
/// (pass the exact pointer AND length you received). Safe to call with NULL.
#[no_mangle]
pub extern "C" fn moonlitt_free_buffer(data: *mut u8, len: usize) {
    crate::error::ffi_guard!((), {
        if !data.is_null() {
            unsafe {
                drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(data, len)));
            }
        }
    })
}
