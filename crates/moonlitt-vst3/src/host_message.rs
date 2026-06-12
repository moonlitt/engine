//! Host-side IMessage / IAttributeList implementations.
//!
//! VST3 plug-ins send notifications between their component and
//! controller halves by constructing an `IMessage`, populating its
//! `IAttributeList` payload, and calling `IConnectionPoint::notify` on
//! the peer they are connected to. The catch is that **the host has to
//! provide the IMessage and IAttributeList objects** — plug-ins
//! delegate construction to `IHostApplication::createInstance(IMessage)`.
//!
//! Without this factory, every IConnectionPoint-using plug-in is
//! effectively unable to talk to itself across the component/controller
//! split (Steinberg's own SDK uses these exact host helpers in its
//! reference hosts). Implementing them is small but unblocks a class
//! of plug-ins entirely.
//!
//! Both objects are thread-safe (any thread may call into them) and use
//! interior mutability behind a single Mutex; the spec doesn't require
//! lock-free access here and these are not on the audio hot path.

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::Mutex;

use vst3::Steinberg::Vst::{
    IAttributeList, IAttributeListTrait, IAttributeList_::AttrID, IMessage, IMessageTrait, TChar,
};
use vst3::Steinberg::{
    int64, kInvalidArgument, kResultFalse, kResultOk, tresult, uint32, FIDString,
};
use vst3::{Class, ComWrapper};

#[derive(Clone, Debug)]
enum AttrValue {
    Int(i64),
    Float(f64),
    /// Stored without a trailing NUL — UTF-16 from the plug-in.
    String(Vec<u16>),
    Binary(Vec<u8>),
}

/// Thread-safe attribute store keyed by C string, exposed via
/// `IAttributeList`. Per spec, `getBinary` returns a pointer that must
/// stay valid until the list is dropped or the value is overwritten;
/// we satisfy that by owning the bytes inside the HashMap and returning
/// a borrow.
pub(crate) struct HostAttributeList {
    /// HashMap key is CString (owned, NUL-terminated). We do not allow
    /// concurrent reads on `getBinary` to interleave with `setBinary` on
    /// the same key, so a Mutex is sufficient.
    map: Mutex<HashMap<CString, AttrValue>>,
}

impl Class for HostAttributeList {
    type Interfaces = (IAttributeList,);
}

fn attr_id_to_cstring(id: AttrID) -> Option<CString> {
    if id.is_null() {
        return None;
    }
    let cstr = unsafe { CStr::from_ptr(id) };
    CString::new(cstr.to_bytes()).ok()
}

impl IAttributeListTrait for HostAttributeList {
    unsafe fn setInt(&self, id: AttrID, value: int64) -> tresult {
        let Some(key) = attr_id_to_cstring(id) else {
            return kInvalidArgument;
        };
        if let Ok(mut m) = self.map.lock() {
            m.insert(key, AttrValue::Int(value));
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn getInt(&self, id: AttrID, value: *mut int64) -> tresult {
        if value.is_null() {
            return kInvalidArgument;
        }
        let Some(key) = attr_id_to_cstring(id) else {
            return kInvalidArgument;
        };
        let Ok(m) = self.map.lock() else {
            return kResultFalse;
        };
        match m.get(&key) {
            Some(AttrValue::Int(v)) => {
                std::ptr::write(value, *v);
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setFloat(&self, id: AttrID, value: f64) -> tresult {
        let Some(key) = attr_id_to_cstring(id) else {
            return kInvalidArgument;
        };
        if let Ok(mut m) = self.map.lock() {
            m.insert(key, AttrValue::Float(value));
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn getFloat(&self, id: AttrID, value: *mut f64) -> tresult {
        if value.is_null() {
            return kInvalidArgument;
        }
        let Some(key) = attr_id_to_cstring(id) else {
            return kInvalidArgument;
        };
        let Ok(m) = self.map.lock() else {
            return kResultFalse;
        };
        match m.get(&key) {
            Some(AttrValue::Float(v)) => {
                std::ptr::write(value, *v);
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setString(&self, id: AttrID, string: *const TChar) -> tresult {
        let Some(key) = attr_id_to_cstring(id) else {
            return kInvalidArgument;
        };
        if string.is_null() {
            return kInvalidArgument;
        }
        // Copy until terminating zero. TChar is u16.
        let mut len = 0usize;
        while *string.add(len) != 0 {
            len += 1;
        }
        let buf: Vec<u16> = std::slice::from_raw_parts(string, len).to_vec();
        if let Ok(mut m) = self.map.lock() {
            m.insert(key, AttrValue::String(buf));
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn getString(&self, id: AttrID, string: *mut TChar, size_in_bytes: uint32) -> tresult {
        if string.is_null() {
            return kInvalidArgument;
        }
        let Some(key) = attr_id_to_cstring(id) else {
            return kInvalidArgument;
        };
        let Ok(m) = self.map.lock() else {
            return kResultFalse;
        };
        let Some(AttrValue::String(buf)) = m.get(&key) else {
            return kResultFalse;
        };

        // size_in_bytes is bytes, but TChar is 2 bytes. Reserve one slot
        // for the trailing zero.
        let cap_chars = (size_in_bytes as usize) / std::mem::size_of::<u16>();
        if cap_chars == 0 {
            return kInvalidArgument;
        }
        let copy_len = buf.len().min(cap_chars - 1);
        let dst = std::slice::from_raw_parts_mut(string, cap_chars);
        dst[..copy_len].copy_from_slice(&buf[..copy_len]);
        dst[copy_len] = 0;
        kResultOk
    }

    unsafe fn setBinary(&self, id: AttrID, data: *const c_void, size_in_bytes: uint32) -> tresult {
        let Some(key) = attr_id_to_cstring(id) else {
            return kInvalidArgument;
        };
        if data.is_null() && size_in_bytes != 0 {
            return kInvalidArgument;
        }
        let bytes: Vec<u8> = if size_in_bytes == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(data as *const u8, size_in_bytes as usize).to_vec()
        };
        if let Ok(mut m) = self.map.lock() {
            m.insert(key, AttrValue::Binary(bytes));
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn getBinary(
        &self,
        id: AttrID,
        data: *mut *const c_void,
        size_in_bytes: *mut uint32,
    ) -> tresult {
        if data.is_null() || size_in_bytes.is_null() {
            return kInvalidArgument;
        }
        let Some(key) = attr_id_to_cstring(id) else {
            return kInvalidArgument;
        };
        let Ok(m) = self.map.lock() else {
            return kResultFalse;
        };
        let Some(AttrValue::Binary(bytes)) = m.get(&key) else {
            return kResultFalse;
        };

        // Per spec: pointer remains valid until the list is destroyed or
        // the value is overwritten. The HashMap entry owns the Vec, so
        // its as_ptr() satisfies that contract for as long as no one
        // calls setBinary on the same key.
        std::ptr::write(data, bytes.as_ptr() as *const c_void);
        std::ptr::write(size_in_bytes, bytes.len() as uint32);
        kResultOk
    }
}

/// Construct an empty IAttributeList COM wrapper.
pub(crate) fn new_attribute_list() -> ComWrapper<HostAttributeList> {
    ComWrapper::new(HostAttributeList {
        map: Mutex::new(HashMap::new()),
    })
}

/// IMessage implementation. Stores its message ID (an FIDString — owned
/// CString here for lifetime safety) and a paired IAttributeList created
/// alongside it. Plug-ins call setMessageID to identify what kind of
/// message they're sending; both halves of the connection use that ID
/// to dispatch in their `notify` handler.
pub(crate) struct HostMessage {
    message_id: Mutex<CString>,
    attributes: ComWrapper<HostAttributeList>,
}

impl Class for HostMessage {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for HostMessage {
    unsafe fn getMessageID(&self) -> FIDString {
        let guard = match self.message_id.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        // The pointer remains valid as long as the CString isn't
        // replaced. Plug-ins are expected to copy the string if they
        // need it past the next setMessageID call (matches reference
        // host behavior).
        guard.as_ptr() as FIDString
    }

    unsafe fn setMessageID(&self, id: FIDString) {
        let new_id = if id.is_null() {
            CString::new("").unwrap_or_default()
        } else {
            CStr::from_ptr(id as *const c_char).to_owned()
        };
        if let Ok(mut g) = self.message_id.lock() {
            *g = new_id;
        }
    }

    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        // Borrow the IAttributeList through to the plug-in. We hand back
        // a non-owning pointer; the IMessage owns the IAttributeList for
        // its lifetime, so the pointer is valid for that span.
        match self.attributes.as_com_ref::<IAttributeList>() {
            Some(r) => r.as_ptr(),
            None => std::ptr::null_mut(),
        }
    }
}

/// Construct an empty IMessage COM wrapper with a fresh IAttributeList
/// payload. Caller is responsible for keeping the wrapper (or a ComPtr
/// derived from it) alive until the plug-in releases the message.
pub(crate) fn new_host_message() -> ComWrapper<HostMessage> {
    ComWrapper::new(HostMessage {
        message_id: Mutex::new(CString::new("").unwrap_or_default()),
        attributes: new_attribute_list(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::ptr;

    fn cstr(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    fn utf16(s: &str) -> Vec<u16> {
        let mut v: Vec<u16> = s.encode_utf16().collect();
        v.push(0);
        v
    }

    #[test]
    fn attribute_list_int_roundtrip() {
        let list = new_attribute_list();
        let api = list.to_com_ptr::<IAttributeList>().unwrap();
        let key = cstr("answer");

        unsafe {
            assert_eq!(api.setInt(key.as_ptr(), 42), kResultOk);
            let mut out: int64 = 0;
            assert_eq!(api.getInt(key.as_ptr(), &mut out), kResultOk);
            assert_eq!(out, 42);
        }
    }

    #[test]
    fn attribute_list_float_roundtrip() {
        let list = new_attribute_list();
        let api = list.to_com_ptr::<IAttributeList>().unwrap();
        let key = cstr("ratio");
        unsafe {
            assert_eq!(api.setFloat(key.as_ptr(), 1.61803), kResultOk);
            let mut out: f64 = 0.0;
            assert_eq!(api.getFloat(key.as_ptr(), &mut out), kResultOk);
            assert!((out - 1.61803).abs() < 1e-9);
        }
    }

    #[test]
    fn attribute_list_string_roundtrip() {
        let list = new_attribute_list();
        let api = list.to_com_ptr::<IAttributeList>().unwrap();
        let key = cstr("name");
        let payload = utf16("Moonlitt");

        unsafe {
            assert_eq!(api.setString(key.as_ptr(), payload.as_ptr()), kResultOk);
            let mut buf = vec![0u16; 32];
            let cap_bytes = (buf.len() * 2) as uint32;
            assert_eq!(
                api.getString(key.as_ptr(), buf.as_mut_ptr(), cap_bytes),
                kResultOk
            );
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let recovered = String::from_utf16_lossy(&buf[..end]);
            assert_eq!(recovered, "Moonlitt");
        }
    }

    #[test]
    fn attribute_list_binary_pointer_stable() {
        let list = new_attribute_list();
        let api = list.to_com_ptr::<IAttributeList>().unwrap();
        let key = cstr("blob");
        let payload: [u8; 5] = [0xDE, 0xAD, 0xBE, 0xEF, 0x42];

        unsafe {
            assert_eq!(
                api.setBinary(
                    key.as_ptr(),
                    payload.as_ptr() as *const c_void,
                    payload.len() as uint32
                ),
                kResultOk
            );
            let mut data: *const c_void = ptr::null();
            let mut size: uint32 = 0;
            assert_eq!(api.getBinary(key.as_ptr(), &mut data, &mut size), kResultOk);
            assert_eq!(size as usize, payload.len());
            let slice = std::slice::from_raw_parts(data as *const u8, size as usize);
            assert_eq!(slice, &payload[..]);
        }
    }

    #[test]
    fn attribute_list_missing_key_returns_false() {
        let list = new_attribute_list();
        let api = list.to_com_ptr::<IAttributeList>().unwrap();
        let key = cstr("absent");
        unsafe {
            let mut v: int64 = 0;
            assert_eq!(api.getInt(key.as_ptr(), &mut v), kResultFalse);
        }
    }

    #[test]
    fn message_id_roundtrip() {
        let msg = new_host_message();
        let api = msg.to_com_ptr::<IMessage>().unwrap();
        let id = cstr("moonlitt.test");
        unsafe {
            api.setMessageID(id.as_ptr() as FIDString);
            let got = api.getMessageID();
            assert!(!got.is_null());
            let recovered = CStr::from_ptr(got).to_string_lossy().into_owned();
            assert_eq!(recovered, "moonlitt.test");
        }
    }

    #[test]
    fn message_attributes_are_addressable() {
        let msg = new_host_message();
        let api = msg.to_com_ptr::<IMessage>().unwrap();
        unsafe {
            let attrs_raw = api.getAttributes();
            assert!(!attrs_raw.is_null());
            // Re-attach as a ComRef so we can use the trait.
            let attrs = vst3::ComRef::from_raw(attrs_raw).unwrap();
            let key = cstr("tempo");
            assert_eq!(attrs.setFloat(key.as_ptr(), 128.0), kResultOk);
            let mut out: f64 = 0.0;
            assert_eq!(attrs.getFloat(key.as_ptr(), &mut out), kResultOk);
            assert_eq!(out, 128.0);
        }
    }
}
