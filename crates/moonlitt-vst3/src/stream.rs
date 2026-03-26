//! In-memory IBStream implementation for VST3 setState/getState.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use vst3::Steinberg::{
    kResultOk, kResultFalse, tresult,
    IBStream,
};

/// A minimal in-memory IBStream for passing state data to VST3 plugins.
#[repr(C)]
pub struct MemoryStream {
    vtable: *const IBStreamVTable,
    ref_count: AtomicU32,
    data: Vec<u8>,
    position: std::cell::Cell<usize>,
}

// Manual VTable for IBStream COM interface
#[repr(C)]
struct IBStreamVTable {
    // FUnknown
    query_interface: unsafe extern "system" fn(*mut c_void, *const c_void, *mut *mut c_void) -> tresult,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IBStream
    read: unsafe extern "system" fn(*mut c_void, *mut c_void, i32, *mut i32) -> tresult,
    write: unsafe extern "system" fn(*mut c_void, *const c_void, i32, *mut i32) -> tresult,
    seek: unsafe extern "system" fn(*mut c_void, i64, i32, *mut i64) -> tresult,
    tell: unsafe extern "system" fn(*mut c_void, *mut i64) -> tresult,
}

static MEMORY_STREAM_VTABLE: IBStreamVTable = IBStreamVTable {
    query_interface: ms_query_interface,
    add_ref: ms_add_ref,
    release: ms_release,
    read: ms_read,
    write: ms_write,
    seek: ms_seek,
    tell: ms_tell,
};

impl MemoryStream {
    pub fn from_data(data: Vec<u8>) -> Box<Self> {
        Box::new(Self {
            vtable: &MEMORY_STREAM_VTABLE,
            ref_count: AtomicU32::new(1),
            data,
            position: std::cell::Cell::new(0),
        })
    }

    /// Get a raw pointer suitable for passing to VST3 setState.
    pub fn as_ibstream_ptr(self: &mut Box<Self>) -> *mut IBStream {
        &mut **self as *mut MemoryStream as *mut IBStream
    }

    fn this(ptr: *mut c_void) -> &'static MemoryStream {
        unsafe { &*(ptr as *const MemoryStream) }
    }
}

unsafe extern "system" fn ms_query_interface(_this: *mut c_void, _iid: *const c_void, obj: *mut *mut c_void) -> tresult {
    *obj = std::ptr::null_mut();
    kResultFalse
}

unsafe extern "system" fn ms_add_ref(this: *mut c_void) -> u32 {
    let s = MemoryStream::this(this);
    s.ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn ms_release(this: *mut c_void) -> u32 {
    let s = MemoryStream::this(this);
    let prev = s.ref_count.fetch_sub(1, Ordering::Relaxed);
    if prev == 1 {
        // Last reference — but we manage lifetime via Box, don't drop here
    }
    prev - 1
}

unsafe extern "system" fn ms_read(this: *mut c_void, buffer: *mut c_void, num_bytes: i32, bytes_read: *mut i32) -> tresult {
    let s = MemoryStream::this(this);
    let pos = s.position.get();
    let available = s.data.len().saturating_sub(pos);
    let to_read = (num_bytes as usize).min(available);

    if to_read > 0 {
        std::ptr::copy_nonoverlapping(
            s.data[pos..].as_ptr(),
            buffer as *mut u8,
            to_read,
        );
    }
    s.position.set(pos + to_read);
    if !bytes_read.is_null() {
        *bytes_read = to_read as i32;
    }
    kResultOk
}

unsafe extern "system" fn ms_write(_this: *mut c_void, _buffer: *const c_void, _num_bytes: i32, _bytes_written: *mut i32) -> tresult {
    kResultFalse // read-only stream
}

unsafe extern "system" fn ms_seek(this: *mut c_void, pos: i64, mode: i32, result: *mut i64) -> tresult {
    let s = MemoryStream::this(this);
    let new_pos = match mode {
        0 => pos as usize,                           // kIBSeekSet
        1 => (s.position.get() as i64 + pos) as usize, // kIBSeekCur
        2 => (s.data.len() as i64 + pos) as usize,    // kIBSeekEnd
        _ => return kResultFalse,
    };
    s.position.set(new_pos.min(s.data.len()));
    if !result.is_null() {
        *result = s.position.get() as i64;
    }
    kResultOk
}

unsafe extern "system" fn ms_tell(this: *mut c_void, pos: *mut i64) -> tresult {
    let s = MemoryStream::this(this);
    if !pos.is_null() {
        *pos = s.position.get() as i64;
    }
    kResultOk
}

// === sfizz state builder ===

/// Build a binary state blob for sfizz VST3 (state version 5).
pub fn build_sfizz_state(sfz_path: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);

    // version: u64 = 5
    buf.extend_from_slice(&5u64.to_le_bytes());
    // sfzFile: length-prefixed string (IBStreamer format)
    write_string(&mut buf, sfz_path);
    // volume: f32 = 0.0 (0dB)
    buf.extend_from_slice(&0.0f32.to_le_bytes());
    // numVoices: i32 = 64
    buf.extend_from_slice(&64i32.to_le_bytes());
    // oversamplingLog2: i32 = 0
    buf.extend_from_slice(&0i32.to_le_bytes());
    // preloadSize: i32 = 8192
    buf.extend_from_slice(&8192i32.to_le_bytes());
    // scalaFile: string = "" (v>=1)
    write_string(&mut buf, "");
    // scalaRootKey: i32 = 60 (v>=1)
    buf.extend_from_slice(&60i32.to_le_bytes());
    // tuningFrequency: f32 = 440.0 (v>=1)
    buf.extend_from_slice(&440.0f32.to_le_bytes());
    // stretchedTuning: f32 = 0.0 (v>=1)
    buf.extend_from_slice(&0.0f32.to_le_bytes());
    // sampleQuality: i32 = 10 (max, Sinc 72) (v>=3)
    buf.extend_from_slice(&10i32.to_le_bytes());
    // oscillatorQuality: i32 = 3 (v>=3)
    buf.extend_from_slice(&3i32.to_le_bytes());
    // freewheelingSampleQuality: i32 = 10 (v>=5)
    buf.extend_from_slice(&10i32.to_le_bytes());
    // freewheelingOscillatorQuality: i32 = 3 (v>=5)
    buf.extend_from_slice(&3i32.to_le_bytes());
    // sustainCancelsRelease: bool = false (v>=5)
    buf.push(0u8);
    // lastKeyswitch: i32 = -1 (v>=4)
    buf.extend_from_slice(&(-1i32).to_le_bytes());
    // controller count: u32 = 0 (v>=2)
    buf.extend_from_slice(&0u32.to_le_bytes());

    buf
}

/// Write a string in Steinberg IBStreamer format (i32 length + UTF-8 bytes).
fn write_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    buf.extend_from_slice(&(bytes.len() as i32).to_le_bytes());
    buf.extend_from_slice(bytes);
}
