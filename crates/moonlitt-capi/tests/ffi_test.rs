//! Integration tests for the moonlitt C FFI layer.
//!
//! These call the `extern "C"` functions directly — the same entry points
//! that a C/C#/Node consumer would use through the shared library.
//!
//! Note: the FFI functions are declared as safe `extern "C" fn` (not `unsafe`),
//! because they handle NULL checks internally. From Rust they're called like
//! normal functions; from C they go through the shared library's symbol table.

use moonlitt_capi::*;
use std::ffi::{CStr, CString};

// ---------------------------------------------------------------------------
// Engine lifecycle
// ---------------------------------------------------------------------------

#[test]
fn test_engine_lifecycle() {
    let e = moonlitt_engine_create(44100, 256);
    assert!(!e.is_null());
    assert_eq!(moonlitt_engine_is_loaded(e), 0);
    moonlitt_engine_destroy(e);
}

#[test]
fn test_engine_destroy_null() {
    // Must not crash.
    moonlitt_engine_destroy(std::ptr::null_mut());
}

// ---------------------------------------------------------------------------
// Render silence when no backend
// ---------------------------------------------------------------------------

#[test]
fn test_engine_render_silence() {
    let e = moonlitt_engine_create(44100, 256);
    let mut left = vec![1.0f32; 256];
    let mut right = vec![1.0f32; 256];
    moonlitt_engine_render(e, left.as_mut_ptr(), right.as_mut_ptr(), 256);
    assert!(left.iter().all(|&s| s == 0.0), "left should be zeroed");
    assert!(right.iter().all(|&s| s == 0.0), "right should be zeroed");
    moonlitt_engine_destroy(e);
}

// ---------------------------------------------------------------------------
// Render with null / zero frames — should not crash
// ---------------------------------------------------------------------------

#[test]
fn test_engine_render_null_buffers() {
    let e = moonlitt_engine_create(44100, 256);
    moonlitt_engine_render(e, std::ptr::null_mut(), std::ptr::null_mut(), 256);
    moonlitt_engine_render(e, std::ptr::null_mut(), std::ptr::null_mut(), 0);
    moonlitt_engine_destroy(e);
}

// ---------------------------------------------------------------------------
// MIDI on unloaded engine — should not crash
// ---------------------------------------------------------------------------

#[test]
fn test_engine_midi_without_backend() {
    let e = moonlitt_engine_create(44100, 256);
    moonlitt_engine_note_on(e, 0, 60, 100);
    moonlitt_engine_note_off(e, 0, 60);
    moonlitt_engine_cc(e, 0, 64, 127);
    moonlitt_engine_pitch_bend(e, 0, 0);
    moonlitt_engine_program_change(e, 0, 0);
    moonlitt_engine_all_notes_off(e);
    moonlitt_engine_destroy(e);
}

// ---------------------------------------------------------------------------
// Load invalid path — should return error
// ---------------------------------------------------------------------------

#[test]
fn test_engine_load_invalid_path() {
    let e = moonlitt_engine_create(44100, 256);
    let path = CString::new("/nonexistent/file.sf2").unwrap();
    let result = moonlitt_engine_load(e, path.as_ptr());
    assert_ne!(result, 0, "loading nonexistent file should fail");

    let err = moonlitt_engine_get_error(e);
    assert!(!err.is_null(), "error message should be set");
    let err_str = unsafe { CStr::from_ptr(err).to_str().unwrap() };
    assert!(!err_str.is_empty(), "error message should not be empty");
    println!("Expected error: {err_str}");

    moonlitt_engine_destroy(e);
}

// ---------------------------------------------------------------------------
// Load null path
// ---------------------------------------------------------------------------

#[test]
fn test_engine_load_null_path() {
    let e = moonlitt_engine_create(44100, 256);
    let result = moonlitt_engine_load(e, std::ptr::null());
    assert_ne!(result, 0);
    moonlitt_engine_destroy(e);
}

// ---------------------------------------------------------------------------
// SF2 load (conditional — skipped if file doesn't exist)
// ---------------------------------------------------------------------------

#[test]
fn test_engine_load_sf2() {
    let e = moonlitt_engine_create(44100, 256);
    let sf2_path = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
    if std::path::Path::new(sf2_path).exists() {
        let path = CString::new(sf2_path).unwrap();
        let result = moonlitt_engine_load(e, path.as_ptr());
        assert_eq!(result, 0, "loading real SF2 should succeed");
        assert_eq!(moonlitt_engine_is_loaded(e), 1);

        // Unload
        moonlitt_engine_unload(e);
        assert_eq!(moonlitt_engine_is_loaded(e), 0);
    }
    moonlitt_engine_destroy(e);
}

// ---------------------------------------------------------------------------
// Plugin scanning
// ---------------------------------------------------------------------------

#[test]
fn test_scan_plugins() {
    let e = moonlitt_engine_create(44100, 256);
    let json = moonlitt_engine_scan_plugins(e);
    assert!(!json.is_null());
    let s = unsafe { CStr::from_ptr(json).to_str().unwrap() };
    assert!(s.starts_with('['), "should return JSON array");
    assert!(s.ends_with(']'), "should return JSON array");
    println!("Plugins: {s}");
    moonlitt_free_string(json);
    moonlitt_engine_destroy(e);
}

// ---------------------------------------------------------------------------
// Presets (empty when no backend)
// ---------------------------------------------------------------------------

#[test]
fn test_presets_empty() {
    let e = moonlitt_engine_create(44100, 256);
    let json = moonlitt_engine_get_presets(e);
    assert!(!json.is_null());
    let s = unsafe { CStr::from_ptr(json).to_str().unwrap() };
    assert_eq!(s, "[]");
    moonlitt_free_string(json);
    moonlitt_engine_destroy(e);
}

// ---------------------------------------------------------------------------
// Volume
// ---------------------------------------------------------------------------

#[test]
fn test_engine_set_volume() {
    let e = moonlitt_engine_create(44100, 256);
    moonlitt_engine_set_volume(e, 0.5);
    // No crash = pass.
    moonlitt_engine_destroy(e);
}

// ---------------------------------------------------------------------------
// Free null string — should not crash
// ---------------------------------------------------------------------------

#[test]
fn test_free_null_string() {
    moonlitt_free_string(std::ptr::null_mut());
}

// ---------------------------------------------------------------------------
// Null engine operations — should not crash
// ---------------------------------------------------------------------------

#[test]
fn test_null_engine_operations() {
    let null = std::ptr::null_mut();
    assert_eq!(moonlitt_engine_is_loaded(null), 0);
    moonlitt_engine_note_on(null, 0, 60, 100);
    moonlitt_engine_note_off(null, 0, 60);
    moonlitt_engine_cc(null, 0, 64, 127);
    moonlitt_engine_pitch_bend(null, 0, 0);
    moonlitt_engine_program_change(null, 0, 0);
    moonlitt_engine_all_notes_off(null);
    moonlitt_engine_unload(null);
    moonlitt_engine_set_volume(null, 1.0);
    let err = moonlitt_engine_get_error(null);
    assert!(err.is_null());
}

// ---------------------------------------------------------------------------
// Runtime null operations — should not crash
// ---------------------------------------------------------------------------

#[test]
fn test_runtime_destroy_null() {
    moonlitt_runtime_destroy(std::ptr::null_mut());
}

#[test]
fn test_runtime_null_operations() {
    let null = std::ptr::null_mut();
    assert_eq!(moonlitt_runtime_start(null), 1);
    assert_eq!(moonlitt_runtime_stop(null), 1);
    moonlitt_runtime_note_on(null, 0, 60, 100);
    moonlitt_runtime_note_off(null, 0, 60);
    moonlitt_runtime_cc(null, 0, 64, 127);
    moonlitt_runtime_pitch_bend(null, 0, 0);
    moonlitt_runtime_all_notes_off(null);
    moonlitt_runtime_set_volume(null, 1.0);
    moonlitt_runtime_play(null);
    moonlitt_runtime_pause(null);
    moonlitt_runtime_stop_playback(null);
}

// ---------------------------------------------------------------------------
// MIDI device listing
// ---------------------------------------------------------------------------

#[test]
fn test_list_midi_inputs() {
    let json = moonlitt_runtime_list_midi_inputs();
    assert!(!json.is_null());
    let s = unsafe { CStr::from_ptr(json).to_str().unwrap() };
    assert!(s.starts_with('['));
    println!("MIDI inputs: {s}");
    moonlitt_free_string(json);
}

// ---------------------------------------------------------------------------
// Runtime create from null engine
// ---------------------------------------------------------------------------

#[test]
fn test_runtime_create_null_engine() {
    let rt = moonlitt_runtime_create(std::ptr::null_mut());
    assert!(rt.is_null());
}
