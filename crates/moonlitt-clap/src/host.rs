//! clap_host implementation.
//!
//! Provides the host context that CLAP plugins receive during creation.
//! CLAP's host interface is a simple C struct with function pointers —
//! no COM, no reference counting.

use clap_sys::host::clap_host;
use clap_sys::version::CLAP_VERSION;
use std::ffi::c_void;
use std::pin::Pin;

/// Host context passed to CLAP plugins.
///
/// Pinned because the plugin holds a pointer to the `clap_host` inside.
pub(crate) struct HostContext {
    pub host: clap_host,
    // Name string must outlive the host struct
    _name: Pin<Box<std::ffi::CString>>,
}

// The host context is just data + function pointers. Safe to send.
unsafe impl Send for HostContext {}

impl HostContext {
    /// Create a new host context.
    pub fn new() -> Pin<Box<Self>> {
        let name = Box::pin(
            std::ffi::CString::new("Moonlitt").expect("CString::new failed"),
        );

        let name_ptr = name.as_ptr();

        let host = clap_host {
            clap_version: CLAP_VERSION,
            host_data: std::ptr::null_mut(),
            name: name_ptr,
            vendor: c"moonlitt".as_ptr(),
            url: c"https://github.com/moonlitt/engine".as_ptr(),
            version: c"0.1.0".as_ptr(),
            get_extension: Some(host_get_extension),
            request_restart: Some(host_request_restart),
            request_process: Some(host_request_process),
            request_callback: Some(host_request_callback),
        };

        Box::pin(Self { host, _name: name })
    }

    /// Get a raw pointer to the clap_host (for passing to plugin creation).
    pub fn as_ptr(&self) -> *const clap_host {
        &self.host as *const clap_host
    }
}

// ---------------------------------------------------------------------------
// Host callback implementations (minimal — just enough for hosting)
// ---------------------------------------------------------------------------

unsafe extern "C" fn host_get_extension(
    _host: *const clap_host,
    _extension_id: *const std::ffi::c_char,
) -> *const c_void {
    // We don't implement any host extensions yet.
    // Plugins will gracefully degrade without them.
    std::ptr::null()
}

unsafe extern "C" fn host_request_restart(_host: *const clap_host) {
    // In a DAW, this would schedule a deactivate/activate cycle.
    // For our headless host, we can ignore this.
}

unsafe extern "C" fn host_request_process(_host: *const clap_host) {
    // In a DAW, this would wake the audio thread.
    // Our host renders on demand, so nothing to do.
}

unsafe extern "C" fn host_request_callback(_host: *const clap_host) {
    // In a DAW, this would schedule a main-thread callback.
    // Not needed for headless hosting.
}
