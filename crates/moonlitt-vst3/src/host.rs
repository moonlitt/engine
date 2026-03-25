//! IHostApplication implementation
//!
//! Provides the host context that VST3 plugins query during initialization.
//! Some plugins (e.g. Pianoteq) require a valid host context or they will
//! fail to initialize.

use std::ffi::c_void;

use vst3::Steinberg::Vst::{IHostApplication, IHostApplicationTrait, String128};
use vst3::Steinberg::{tresult, TUID};
use vst3::Steinberg::{kNotImplemented, kResultOk};
use vst3::{Class, ComWrapper};

/// Minimal IHostApplication that plugins can query during initialize().
pub(crate) struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        // Write "Moonlitt" as UTF-16 into the String128 buffer
        let host_name: &[u16] = &[
            'M' as u16,
            'o' as u16,
            'o' as u16,
            'n' as u16,
            'l' as u16,
            'i' as u16,
            't' as u16,
            't' as u16,
            0,
        ];
        let buf = &mut *name;
        buf.fill(0);
        buf[..host_name.len()].copy_from_slice(host_name);
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        _cid: *mut TUID,
        _iid: *mut TUID,
        _obj: *mut *mut c_void,
    ) -> tresult {
        kNotImplemented
    }
}

/// Create a new host application COM wrapper.
pub(crate) fn create_host() -> ComWrapper<HostApp> {
    ComWrapper::new(HostApp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vst3::Steinberg::Vst::IHostApplication;

    #[test]
    fn host_can_be_created_and_queried() {
        let host = create_host();
        // Should be able to get a ComPtr<IHostApplication>
        let ptr = host.to_com_ptr::<IHostApplication>();
        assert!(ptr.is_some());
    }
}
