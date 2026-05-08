//! IHostApplication implementation
//!
//! Provides the host context that VST3 plugins query during initialization.
//! Some plugins (e.g. Pianoteq) require a valid host context or they will
//! fail to initialize.

use std::ffi::c_void;

use vst3::Steinberg::Vst::{
    IAttributeList, IHostApplication, IHostApplicationTrait, IMessage, String128,
};
use vst3::Steinberg::{tresult, TUID};
use vst3::Steinberg::{kNotImplemented, kResultOk};
use vst3::{Class, ComWrapper, Interface};

use crate::host_message::{new_attribute_list, new_host_message};

/// Minimal IHostApplication that plugins can query during initialize().
pub(crate) struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        crate::trace::emit("HostApp::getName -> \"Moonlitt\"");
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
        cid: *mut TUID,
        iid: *mut TUID,
        obj: *mut *mut c_void,
    ) -> tresult {
        let (cid_bytes, iid_bytes) = (read_tuid(cid), read_tuid(iid));

        // Per Steinberg's reference host, both cid and iid identify the
        // **interface** to be instantiated when the request comes from a
        // plug-in (these aren't first-class classes registered with the
        // factory). We support the two interfaces the spec calls out:
        // IMessage and IAttributeList.
        let result = if tuid_matches::<IMessage>(&cid_bytes) || tuid_matches::<IMessage>(&iid_bytes) {
            // Hand the plug-in a fresh IMessage. We move ownership into a
            // ComPtr so the released-by-plugin path correctly drops it.
            let wrapper = new_host_message();
            match wrapper.to_com_ptr::<IMessage>() {
                Some(ptr) => {
                    let raw = ptr.into_raw() as *mut c_void;
                    if !obj.is_null() {
                        std::ptr::write(obj, raw);
                    }
                    kResultOk
                }
                None => kNotImplemented,
            }
        } else if tuid_matches::<IAttributeList>(&cid_bytes)
            || tuid_matches::<IAttributeList>(&iid_bytes)
        {
            let wrapper = new_attribute_list();
            match wrapper.to_com_ptr::<IAttributeList>() {
                Some(ptr) => {
                    let raw = ptr.into_raw() as *mut c_void;
                    if !obj.is_null() {
                        std::ptr::write(obj, raw);
                    }
                    kResultOk
                }
                None => kNotImplemented,
            }
        } else {
            kNotImplemented
        };

        crate::trace::emit(&format!(
            "HostApp::createInstance cid={} iid={} -> 0x{:08X}",
            crate::trace::iid_name(&cid_bytes),
            crate::trace::iid_name(&iid_bytes),
            result as u32
        ));
        result
    }
}

/// Read 16 raw bytes from a `*mut TUID`, returning zeros if null.
unsafe fn read_tuid(ptr: *mut TUID) -> [u8; 16] {
    if ptr.is_null() {
        return [0u8; 16];
    }
    let s = std::slice::from_raw_parts(ptr as *const u8, 16);
    let mut a = [0u8; 16];
    a.copy_from_slice(s);
    a
}

/// True if the given 16-byte buffer matches the IID of interface `I`.
fn tuid_matches<I: Interface>(bytes: &[u8; 16]) -> bool {
    // Interface::IID is a Guid (newtype around [u8; 16] with a stable
    // memory layout). Compare bytewise.
    let iid: &[u8; 16] = unsafe {
        &*(I::IID.as_ref() as *const _ as *const [u8; 16])
    };
    iid == bytes
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
