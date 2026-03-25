//! Platform-specific VST3 bundle loading
//!
//! Loads .vst3 bundles and extracts the GetPluginFactory entry point.

use crate::{Error, Result};
use std::ffi::c_void;
use std::path::Path;

/// Function pointer type for the VST3 module entry point
pub(crate) type GetFactoryFn = unsafe extern "system" fn() -> *mut c_void;

/// A loaded VST3 module. Holds the dlopen handle alongside the factory function.
///
/// We intentionally never call dlclose — audio plugins commonly have static
/// destructors that reference the library, so unloading would cause crashes.
/// This is standard practice across all major DAWs and plugin hosts.
pub(crate) struct Module {
    /// Kept alive to prevent the OS from unloading the shared library.
    /// Never dlclosed (intentional — see struct-level doc).
    _handle: *mut c_void,
    pub factory_fn: GetFactoryFn,
}

// The module handle is a dlopen handle — safe to move between threads.
// factory_fn is a plain function pointer.
unsafe impl Send for Module {}
unsafe impl Sync for Module {}

/// Load a .vst3 bundle and return a Module holding both the handle and factory function.
pub(crate) fn load_module(path: &Path) -> Result<Module> {
    #[cfg(target_os = "macos")]
    return load_module_macos(path);

    #[cfg(target_os = "windows")]
    return load_module_windows(path);

    #[cfg(target_os = "linux")]
    return load_module_linux(path);

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Err(Error::Other("unsupported platform".into()));
}

#[cfg(target_os = "macos")]
fn load_module_macos(path: &Path) -> Result<Module> {
    use std::ffi::CString;

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::LoadFailed("invalid path".into()))?;

    let binary = format!("{}/Contents/MacOS/{}", path.display(), stem);
    let c_path = CString::new(binary.as_str())
        .map_err(|e| Error::LoadFailed(e.to_string()))?;

    unsafe {
        let handle = libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if handle.is_null() {
            let err = libc::dlerror();
            let msg = if err.is_null() {
                "dlopen failed".to_string()
            } else {
                std::ffi::CStr::from_ptr(err).to_string_lossy().into()
            };
            return Err(Error::LoadFailed(msg));
        }

        let sym = libc::dlsym(handle, c"GetPluginFactory".as_ptr());
        if sym.is_null() {
            return Err(Error::LoadFailed("GetPluginFactory not found".into()));
        }

        Ok(Module {
            _handle: handle,
            factory_fn: std::mem::transmute::<*mut c_void, GetFactoryFn>(sym),
        })
    }
}

#[cfg(target_os = "windows")]
fn load_module_windows(path: &Path) -> Result<Module> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    // VST3 on Windows: either a DLL directly or a bundle directory
    // Bundle layout: path/Contents/x86_64-win/name.vst3
    let stem = path.file_name().unwrap_or_default();
    let bundle_path = path.join("Contents").join("x86_64-win").join(stem);
    let dll_path = if bundle_path.exists() {
        bundle_path
    } else {
        path.to_path_buf()
    };

    let wide: Vec<u16> = OsStr::new(&dll_path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let handle = LoadLibraryW(wide.as_ptr());
        if handle.is_null() {
            return Err(Error::LoadFailed(format!(
                "LoadLibraryW failed for {}",
                dll_path.display()
            )));
        }

        let sym = GetProcAddress(handle, c"GetPluginFactory".as_ptr() as *const u8);
        if sym.is_null() {
            return Err(Error::LoadFailed("GetPluginFactory not found".into()));
        }

        Ok(Module {
            _handle: handle as *mut c_void,
            factory_fn: std::mem::transmute::<*const (), GetFactoryFn>(sym as *const ()),
        })
    }
}

#[cfg(target_os = "windows")]
extern "system" {
    fn LoadLibraryW(lpFileName: *const u16) -> *mut c_void;
    fn GetProcAddress(hModule: *mut c_void, lpProcName: *const u8) -> *const ();
}

#[cfg(target_os = "linux")]
fn load_module_linux(path: &Path) -> Result<Module> {
    use std::ffi::CString;

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::LoadFailed("invalid path".into()))?;

    // Linux VST3 bundle: path/Contents/x86_64-linux/stem.so
    let so_path = path
        .join("Contents")
        .join("x86_64-linux")
        .join(format!("{stem}.so"));
    let load_path = if so_path.exists() {
        so_path
    } else {
        path.to_path_buf()
    };

    let c_path = CString::new(load_path.to_str().unwrap_or(""))
        .map_err(|e| Error::LoadFailed(e.to_string()))?;

    unsafe {
        let handle = libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if handle.is_null() {
            let err = libc::dlerror();
            let msg = if err.is_null() {
                "dlopen failed".to_string()
            } else {
                std::ffi::CStr::from_ptr(err).to_string_lossy().into()
            };
            return Err(Error::LoadFailed(msg));
        }

        let sym = libc::dlsym(handle, c"GetPluginFactory".as_ptr());
        if sym.is_null() {
            return Err(Error::LoadFailed("GetPluginFactory not found".into()));
        }

        Ok(Module {
            _handle: handle,
            factory_fn: std::mem::transmute::<*mut c_void, GetFactoryFn>(sym),
        })
    }
}
