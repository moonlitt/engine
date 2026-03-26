//! In-memory IBStream implementation for VST3 setState/getState.

use std::cell::Cell;
use std::ffi::c_void;
use vst3::Steinberg::{
    int32, int64, kResultFalse, kResultOk, tresult, uint32, FUnknown, FUnknownVtbl, IBStream,
    IBStreamVtbl, TUID,
};

/// A minimal in-memory read-only IBStream.
/// Layout: `{ vtbl: *const IBStreamVtbl, ... }` matches IBStream COM layout.
#[repr(C)]
pub struct MemoryStream {
    vtbl: *const IBStreamVtbl,
    ref_count: Cell<u32>,
    data: Vec<u8>,
    position: Cell<usize>,
}

static VTBL: IBStreamVtbl = IBStreamVtbl {
    base: FUnknownVtbl {
        queryInterface: ms_query_interface,
        addRef: ms_add_ref,
        release: ms_release,
    },
    read: ms_read,
    write: ms_write,
    seek: ms_seek,
    tell: ms_tell,
};

impl MemoryStream {
    pub fn from_data(data: Vec<u8>) -> Box<Self> {
        Box::new(Self {
            vtbl: &VTBL,
            ref_count: Cell::new(1),
            data,
            position: Cell::new(0),
        })
    }

    /// Get a raw pointer suitable for passing to VST3 setState.
    pub fn as_ibstream_ptr(self: &mut Box<Self>) -> *mut IBStream {
        &mut **self as *mut MemoryStream as *mut IBStream
    }
}

#[inline]
unsafe fn get_self(this: *mut IBStream) -> &'static MemoryStream {
    &*(this as *const MemoryStream)
}

unsafe extern "system" fn ms_query_interface(
    _this: *mut FUnknown,
    _iid: *const TUID,
    obj: *mut *mut c_void,
) -> tresult {
    if !obj.is_null() {
        *obj = std::ptr::null_mut();
    }
    kResultFalse
}

unsafe extern "system" fn ms_add_ref(this: *mut FUnknown) -> uint32 {
    let s = &*(this as *const MemoryStream);
    let c = s.ref_count.get() + 1;
    s.ref_count.set(c);
    c
}

unsafe extern "system" fn ms_release(this: *mut FUnknown) -> uint32 {
    let s = &*(this as *const MemoryStream);
    let c = s.ref_count.get().saturating_sub(1);
    s.ref_count.set(c);
    c
}

unsafe extern "system" fn ms_read(
    this: *mut IBStream,
    buffer: *mut c_void,
    num_bytes: int32,
    bytes_read: *mut int32,
) -> tresult {
    let s = get_self(this);
    let pos = s.position.get();
    let available = s.data.len().saturating_sub(pos);
    let to_read = (num_bytes as usize).min(available);

    if to_read > 0 && !buffer.is_null() {
        std::ptr::copy_nonoverlapping(s.data[pos..].as_ptr(), buffer as *mut u8, to_read);
    }
    s.position.set(pos + to_read);
    if !bytes_read.is_null() {
        *bytes_read = to_read as int32;
    }
    kResultOk
}

unsafe extern "system" fn ms_write(
    _this: *mut IBStream,
    _buffer: *mut c_void,
    _num_bytes: int32,
    _bytes_written: *mut int32,
) -> tresult {
    kResultFalse // read-only
}

unsafe extern "system" fn ms_seek(
    this: *mut IBStream,
    pos: int64,
    mode: int32,
    result: *mut int64,
) -> tresult {
    let s = get_self(this);
    let new_pos = match mode {
        0 => pos as usize,                              // kIBSeekSet
        1 => (s.position.get() as i64 + pos) as usize,  // kIBSeekCur
        2 => (s.data.len() as i64 + pos) as usize,      // kIBSeekEnd
        _ => return kResultFalse,
    };
    s.position.set(new_pos.min(s.data.len()));
    if !result.is_null() {
        *result = s.position.get() as int64;
    }
    kResultOk
}

unsafe extern "system" fn ms_tell(this: *mut IBStream, pos: *mut int64) -> tresult {
    let s = get_self(this);
    if !pos.is_null() {
        *pos = s.position.get() as int64;
    }
    kResultOk
}

// === sfizz state builder ===

/// Build a binary state blob for sfizz VST3 (state version 5).
/// Uses Steinberg IBStreamer little-endian format.
pub fn build_sfizz_state(sfz_path: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);

    // version: u64 = 5
    buf.extend_from_slice(&5u64.to_le_bytes());
    // sfzFile: IBStreamer::writeStr8 (i32 len + bytes)
    write_str8(&mut buf, sfz_path);
    // volume: f32 = 0.0
    buf.extend_from_slice(&0.0f32.to_le_bytes());
    // numVoices: i32 = 64
    buf.extend_from_slice(&64i32.to_le_bytes());
    // oversamplingLog2: i32 = 0
    buf.extend_from_slice(&0i32.to_le_bytes());
    // preloadSize: i32 = 8192
    buf.extend_from_slice(&8192i32.to_le_bytes());
    // scalaFile: string (v>=1) — empty string needs length 1 with just \0
    write_str8(&mut buf, "");
    // scalaRootKey: i32 = 60 (v>=1)
    buf.extend_from_slice(&60i32.to_le_bytes());
    // tuningFrequency: f32 = 440.0 (v>=1)
    buf.extend_from_slice(&440.0f32.to_le_bytes());
    // stretchedTuning: f32 = 0.0 (v>=1)
    buf.extend_from_slice(&0.0f32.to_le_bytes());
    // sampleQuality: i32 = 10 (Sinc 72) (v>=3)
    buf.extend_from_slice(&10i32.to_le_bytes());
    // oscillatorQuality: i32 = 3 (v>=3)
    buf.extend_from_slice(&3i32.to_le_bytes());
    // freewheelingSampleQuality: i32 = 10 (v>=5)
    buf.extend_from_slice(&10i32.to_le_bytes());
    // freewheelingOscillatorQuality: i32 = 3 (v>=5)
    buf.extend_from_slice(&3i32.to_le_bytes());
    // sustainCancelsRelease: bool as int16 = 0 (v>=5)
    buf.extend_from_slice(&0i16.to_le_bytes());
    // lastKeyswitch: i32 = -1 (v>=4)
    buf.extend_from_slice(&(-1i32).to_le_bytes());
    // controller count: u32 = 0 (v>=2)
    buf.extend_from_slice(&0u32.to_le_bytes());

    buf
}

/// IBStreamer::writeStr8 format: i32 length (strlen+1 including \0) + bytes + \0
/// For empty string "": strlen=0, length=1, writes \0
/// For null pointer: length=0, no bytes (but we don't have null in Rust)
fn write_str8(buf: &mut Vec<u8>, s: &str) {
    let len = s.len() + 1; // always includes null terminator (strlen + 1)
    buf.extend_from_slice(&(len as i32).to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
    buf.push(0); // null terminator
}
