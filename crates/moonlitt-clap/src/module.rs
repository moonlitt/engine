//! Platform-specific .clap bundle loading.
//!
//! Loads .clap shared libraries and extracts the `clap_entry` symbol.
//! Uses `clap_plugin_factory` to enumerate and create plugin instances.

use crate::{Error, Result};
use clap_sys::entry::clap_plugin_entry;
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::plugin::clap_plugin_descriptor;
use std::ffi::{c_void, CStr, CString};
use std::path::Path;

#[cfg(target_os = "windows")]
use std::ffi::OsStr;
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;

/// Parsed plugin descriptor (owned strings).
pub(crate) struct PluginDescriptor {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub description: String,
}

/// A loaded .clap shared library with access to its factory.
pub(crate) struct ClapModule {
    _handle: *mut c_void,
    factory: *const clap_plugin_factory,
    entry: *const clap_plugin_entry,
}

// The module handle is just a dlopen handle — safe to move across threads.
// The factory pointer is only used behind &self methods.
unsafe impl Send for ClapModule {}

impl ClapModule {
    /// Load a .clap bundle from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let binary_path = resolve_binary_path(path)?;
        let (handle, entry_sym) = load_shared_library(&binary_path)?;

        let plugin_path = CString::new(path.to_string_lossy().as_ref())
            .map_err(|e| Error::LoadFailed(e.to_string()))?;

        unsafe {
            let entry = entry_sym as *const clap_plugin_entry;

            // Call entry.init(plugin_path)
            let init_fn = (*entry)
                .init
                .ok_or(Error::LoadFailed("clap_entry.init is null".into()))?;
            if !init_fn(plugin_path.as_ptr()) {
                return Err(Error::LoadFailed(
                    "clap_entry.init() returned false".into(),
                ));
            }

            // Get the plugin factory
            let get_factory = (*entry).get_factory.ok_or(Error::LoadFailed(
                "clap_entry.get_factory is null".into(),
            ))?;
            let factory_raw = get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr());
            if factory_raw.is_null() {
                if let Some(deinit_fn) = (*entry).deinit {
                    deinit_fn();
                }
                return Err(Error::LoadFailed(
                    "get_factory returned null for plugin factory".into(),
                ));
            }

            let factory = factory_raw as *const clap_plugin_factory;

            Ok(Self {
                _handle: handle,
                factory,
                entry,
            })
        }
    }

    /// Number of plugins in this bundle.
    pub fn plugin_count(&self) -> u32 {
        unsafe {
            let count_fn = (*self.factory).get_plugin_count.expect(
                "get_plugin_count is null",
            );
            count_fn(self.factory)
        }
    }

    /// Get descriptor for plugin at index.
    pub fn plugin_descriptor(&self, index: u32) -> Option<PluginDescriptor> {
        unsafe {
            let get_desc = (*self.factory).get_plugin_descriptor?;
            let desc_ptr: *const clap_plugin_descriptor = get_desc(self.factory, index);
            if desc_ptr.is_null() {
                return None;
            }

            Some(parse_descriptor(&*desc_ptr))
        }
    }

    /// Get the raw factory pointer (for creating plugin instances).
    pub(crate) fn factory(&self) -> *const clap_plugin_factory {
        self.factory
    }
}

impl Drop for ClapModule {
    fn drop(&mut self) {
        unsafe {
            // Call entry.deinit()
            if let Some(deinit_fn) = (*self.entry).deinit {
                deinit_fn();
            }
            // We intentionally do NOT dlclose the handle — some plugins
            // have static destructors that reference the library. Leaking
            // the handle is the standard practice for audio plugin hosts.
        }
    }
}

/// Parse a clap_plugin_descriptor into owned Rust strings.
fn parse_descriptor(desc: &clap_plugin_descriptor) -> PluginDescriptor {
    PluginDescriptor {
        id: cstr_to_string(desc.id),
        name: cstr_to_string(desc.name),
        vendor: cstr_to_string(desc.vendor),
        description: cstr_to_string(desc.description),
    }
}

/// Safely convert a C string pointer to an owned Rust String.
fn cstr_to_string(ptr: *const std::ffi::c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

/// Load the shared library and find the `clap_entry` symbol.
/// Returns (handle, symbol pointer).
#[cfg(not(target_os = "windows"))]
fn load_shared_library(binary_path: &str) -> Result<(*mut c_void, *mut c_void)> {
    let c_path =
        CString::new(binary_path).map_err(|e| Error::LoadFailed(e.to_string()))?;

    unsafe {
        let handle = libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if handle.is_null() {
            let err = libc::dlerror();
            let msg = if err.is_null() {
                "dlopen failed".to_string()
            } else {
                CStr::from_ptr(err).to_string_lossy().into()
            };
            return Err(Error::LoadFailed(msg));
        }

        let sym = libc::dlsym(handle, c"clap_entry".as_ptr());
        if sym.is_null() {
            libc::dlclose(handle);
            return Err(Error::LoadFailed("clap_entry symbol not found".into()));
        }

        Ok((handle, sym))
    }
}

/// Load the shared library and find the `clap_entry` symbol (Windows).
/// Returns (handle, symbol pointer).
#[cfg(target_os = "windows")]
fn load_shared_library(binary_path: &str) -> Result<(*mut c_void, *mut c_void)> {
    let wide: Vec<u16> = OsStr::new(binary_path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let handle = LoadLibraryW(wide.as_ptr());
        if handle.is_null() {
            return Err(Error::LoadFailed(format!(
                "LoadLibraryW failed for {binary_path}"
            )));
        }

        let sym = GetProcAddress(handle, c"clap_entry".as_ptr() as *const u8);
        if sym.is_null() {
            return Err(Error::LoadFailed("clap_entry symbol not found".into()));
        }

        Ok((handle as *mut c_void, sym as *mut c_void))
    }
}

#[cfg(target_os = "windows")]
extern "system" {
    fn LoadLibraryW(lpFileName: *const u16) -> *mut c_void;
    fn GetProcAddress(hModule: *mut c_void, lpProcName: *const u8) -> *const ();
}

/// Resolve the actual binary path inside a .clap bundle.
///
/// On macOS, .clap files are bundles (like .app):
///   Foo.clap/Contents/MacOS/Foo
///
/// On Linux and Windows, .clap files are plain shared libraries.
fn resolve_binary_path(path: &Path) -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::LoadFailed("invalid path".into()))?;

        let binary = format!("{}/Contents/MacOS/{}", path.display(), stem);

        // Some CLAP plugins on macOS are plain dylibs, not bundles.
        // Check if the bundle-style path exists; if not, try the path directly.
        if std::path::Path::new(&binary).exists() {
            return Ok(binary);
        }

        // Fall back to treating it as a plain shared library
        Ok(path.to_string_lossy().into_owned())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(path.to_string_lossy().into_owned())
    }
}
