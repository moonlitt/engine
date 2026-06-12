//! Integration tests for the moonlitt C FFI layer.
//!
//! These call the `extern "C"` functions directly — the same entry points
//! that a C/C#/Node consumer would use through the shared library.
//!
//! Note: the FFI functions are declared as safe `extern "C" fn` (not `unsafe`),
//! because they handle NULL checks internally. From Rust they're called like
//! normal functions; from C they go through the shared library's symbol table.

use moonlitt::*;
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
    assert_eq!(
        result, MOONLITT_ERR_IO,
        "nonexistent file is an I/O failure"
    );

    let err = moonlitt_last_error_message();
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
fn test_presets_without_backend_is_null() {
    let e = moonlitt_engine_create(44100, 256);
    assert!(
        moonlitt_engine_get_presets(e).is_null(),
        "no backend → NULL + NOT_LOADED, not a fake empty list"
    );
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
    assert_eq!(
        moonlitt_engine_note_on(null, 0, 60, 100),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_engine_note_off(null, 0, 60),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_engine_cc(null, 0, 64, 127),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_engine_pitch_bend(null, 0, 0),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_engine_program_change(null, 0, 0),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_engine_all_notes_off(null),
        MOONLITT_ERR_INVALID_ARG
    );
    moonlitt_engine_unload(null);
    assert_eq!(
        moonlitt_engine_set_volume(null, 1.0),
        MOONLITT_ERR_INVALID_ARG
    );
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
    assert_eq!(moonlitt_runtime_start_audio(null), MOONLITT_ERR_INVALID_ARG);
    assert_eq!(moonlitt_runtime_stop_audio(null), MOONLITT_ERR_INVALID_ARG);
    assert_eq!(
        moonlitt_runtime_note_on(null, 0, 60, 100),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_runtime_note_off(null, 0, 60),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_runtime_cc(null, 0, 64, 127),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_runtime_pitch_bend(null, 0, 0),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_runtime_all_notes_off(null),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_runtime_set_volume(null, 1.0),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(moonlitt_runtime_play(null), MOONLITT_ERR_INVALID_ARG);
    assert_eq!(moonlitt_runtime_pause(null), MOONLITT_ERR_INVALID_ARG);
    assert_eq!(moonlitt_runtime_stop(null), MOONLITT_ERR_INVALID_ARG);
    // Out-of-range ids are rejected, never truncated.
    assert_eq!(
        moonlitt_runtime_set_track_volume(null, 300, 1.0),
        MOONLITT_ERR_INVALID_ARG
    );
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

// ---------------------------------------------------------------------------
// Error model: ABI version, thread-local last error, panic guard
// ---------------------------------------------------------------------------

#[test]
fn test_abi_version_packed() {
    let v = moonlitt_abi_version();
    let (major, minor, patch) = (v >> 16, (v >> 8) & 0xFF, v & 0xFF);
    assert_eq!(
        (major, minor, patch),
        (0, 9, 0),
        "ABI draft starts at 0.9.0"
    );
}

#[test]
fn test_last_error_null_on_fresh_thread() {
    std::thread::spawn(|| {
        assert!(moonlitt_last_error_message().is_null());
    })
    .join()
    .unwrap();
}

#[test]
fn test_panic_guard_returns_status_and_sets_message() {
    // Silence the default panic hook while we trigger a deliberate panic.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let status = moonlitt_debug_trigger_panic();
    std::panic::set_hook(prev);

    assert_eq!(status, MOONLITT_ERR_PANIC);
    let msg = moonlitt_last_error_message();
    assert!(!msg.is_null(), "panic must leave a last-error message");
    let text = unsafe { CStr::from_ptr(msg) }.to_string_lossy().to_string();
    assert!(
        text.contains("panic"),
        "message should mention panic: {text}"
    );
}

// ---------------------------------------------------------------------------
// Status-code conventions (ABI draft 0.9): arguments are validated before
// backend presence; no silent failures
// ---------------------------------------------------------------------------

#[test]
fn test_engine_arg_validation_precedes_backend_check() {
    let e = moonlitt_engine_create(44100, 256); // nothing loaded
    assert_eq!(
        moonlitt_engine_note_on(e, 16, 60, 100),
        MOONLITT_ERR_INVALID_ARG,
        "channel > 15"
    );
    assert_eq!(
        moonlitt_engine_note_on(e, 0, 128, 100),
        MOONLITT_ERR_INVALID_ARG,
        "note > 127"
    );
    assert_eq!(
        moonlitt_engine_note_on(e, 0, 60, 128),
        MOONLITT_ERR_INVALID_ARG,
        "velocity > 127"
    );
    assert_eq!(
        moonlitt_engine_pitch_bend(e, 0, 9000),
        MOONLITT_ERR_INVALID_ARG,
        "bend > 8191"
    );
    // Valid args but no backend loaded → NOT_LOADED
    assert_eq!(
        moonlitt_engine_note_on(e, 0, 60, 100),
        MOONLITT_ERR_NOT_LOADED
    );
    let msg = moonlitt_last_error_message();
    assert!(!msg.is_null());
    moonlitt_engine_destroy(e);
}

#[test]
fn test_engine_render_reports_not_loaded_but_still_silences() {
    let e = moonlitt_engine_create(44100, 256);
    let mut left = vec![1.0f32; 64];
    let mut right = vec![1.0f32; 64];
    let st = moonlitt_engine_render(e, left.as_mut_ptr(), right.as_mut_ptr(), 64);
    assert_eq!(
        st, MOONLITT_ERR_NOT_LOADED,
        "detectable, even though buffers are silenced"
    );
    assert!(left.iter().chain(right.iter()).all(|&s| s == 0.0));
    assert_eq!(
        moonlitt_engine_render(e, std::ptr::null_mut(), right.as_mut_ptr(), 64),
        MOONLITT_ERR_INVALID_ARG
    );
    moonlitt_engine_destroy(e);
}

#[test]
fn test_engine_param_validation() {
    let e = moonlitt_engine_create(44100, 256);
    assert_eq!(
        moonlitt_engine_set_param(e, -1, 0.5),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_engine_set_param(e, 0, f64::NAN),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_engine_set_param(e, 0, 0.5),
        MOONLITT_ERR_NOT_LOADED
    );
    assert!(
        moonlitt_engine_get_param(e, 0).is_nan(),
        "documented NaN sentinel"
    );
    assert!(
        moonlitt_engine_get_presets(e).is_null(),
        "no backend → NULL, not a fake empty list"
    );
    moonlitt_engine_destroy(e);
}

#[test]
fn test_engine_create_rejects_invalid_config() {
    assert!(moonlitt_engine_create(0, 256).is_null());
    assert!(moonlitt_engine_create(44100, -1).is_null());
    let msg = moonlitt_last_error_message();
    assert!(!msg.is_null());
}

// ---------------------------------------------------------------------------
// Engine state API (single-patch workflow: capture once, replay headless)
// ---------------------------------------------------------------------------

#[test]
fn test_engine_state_api_validation() {
    let e = moonlitt_engine_create(44100, 256);
    let mut data: *mut u8 = std::ptr::null_mut();
    let mut len: usize = 0;

    // Null out-params → INVALID_ARG; valid args but no backend → NOT_LOADED.
    assert_eq!(
        moonlitt_engine_save_state(e, std::ptr::null_mut(), &mut len),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_engine_save_state(e, &mut data, std::ptr::null_mut()),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_engine_save_state(e, &mut data, &mut len),
        MOONLITT_ERR_NOT_LOADED
    );
    assert_eq!(
        moonlitt_engine_load_state(e, std::ptr::null(), 4),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(moonlitt_engine_warm_up(e, -1), MOONLITT_ERR_INVALID_ARG);
    assert_eq!(moonlitt_engine_warm_up(e, 16), MOONLITT_ERR_NOT_LOADED);

    // Capability/advisory queries are 0 on empty/null handles.
    assert_eq!(moonlitt_engine_supports_state(e), 0);
    assert_eq!(moonlitt_engine_supports_state(std::ptr::null_mut()), 0);
    assert_eq!(moonlitt_engine_recommended_warmup_blocks(e), 0);

    // free_buffer is NULL-safe.
    moonlitt_free_buffer(std::ptr::null_mut(), 0);

    moonlitt_engine_destroy(e);
}

#[test]
fn test_engine_state_unsupported_on_sf2() {
    let sf2 = "/Users/wangyan/Desktop/stardew valley mods/soundfonts/GeneralUser_GS.sf2";
    if !std::path::Path::new(sf2).exists() {
        eprintln!("SF2 not present — skipping");
        return;
    }
    let e = moonlitt_engine_create(44100, 256);
    let path = CString::new(sf2).unwrap();
    assert_eq!(moonlitt_engine_load(e, path.as_ptr()), MOONLITT_OK);

    assert_eq!(
        moonlitt_engine_supports_state(e),
        0,
        "SF2 backends expose no state"
    );
    let mut data: *mut u8 = std::ptr::null_mut();
    let mut len: usize = 0;
    assert_eq!(
        moonlitt_engine_save_state(e, &mut data, &mut len),
        MOONLITT_ERR_UNSUPPORTED
    );
    assert_eq!(
        moonlitt_engine_load_state(e, [0u8; 4].as_ptr(), 4),
        MOONLITT_ERR_UNSUPPORTED
    );
    // Warm-up is always safe to call (no-op for non-streamers).
    assert_eq!(moonlitt_engine_warm_up(e, 4), MOONLITT_OK);
    moonlitt_engine_destroy(e);
}

fn find_pianoteq() -> Option<std::path::PathBuf> {
    std::fs::read_dir("/Library/Audio/Plug-Ins/VST3")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().to_lowercase().contains("pianoteq"))
        })
}

#[test]
fn test_pianoteq_state_roundtrip_through_c_api() {
    let Some(plugin) = find_pianoteq() else {
        eprintln!("Pianoteq not installed — skipping");
        return;
    };
    println!("using {}", plugin.display());
    let e = moonlitt_engine_create(44100, 256);
    let path = CString::new(plugin.to_str().unwrap()).unwrap();
    assert_eq!(moonlitt_engine_load(e, path.as_ptr()), MOONLITT_OK);
    assert_eq!(moonlitt_engine_supports_state(e), 1);

    let mut data: *mut u8 = std::ptr::null_mut();
    let mut len: usize = 0;
    assert_eq!(
        moonlitt_engine_save_state(e, &mut data, &mut len),
        MOONLITT_OK
    );
    assert!(!data.is_null());
    assert!(len > 0, "state blob should be non-empty");

    let blob = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
    assert_eq!(
        moonlitt_engine_load_state(e, blob.as_ptr(), blob.len()),
        MOONLITT_OK
    );

    moonlitt_free_buffer(data, len);
    moonlitt_engine_destroy(e);
}

// ---------------------------------------------------------------------------
// Runtime queries (is_running, master meters)
// ---------------------------------------------------------------------------

#[test]
fn test_runtime_queries_null_safe() {
    let null = std::ptr::null_mut();
    assert_eq!(moonlitt_runtime_is_running(null), 0);
    let mut l = 0.0f32;
    let mut r = 0.0f32;
    assert_eq!(
        moonlitt_runtime_master_peak(null, &mut l, &mut r),
        MOONLITT_ERR_INVALID_ARG
    );
    assert_eq!(
        moonlitt_runtime_master_rms(null, &mut l, &mut r),
        MOONLITT_ERR_INVALID_ARG
    );
}

// ---------------------------------------------------------------------------
// Deep session validation: referenced files must exist, so a session that
// validates cannot later fail to load on a missing path
// ---------------------------------------------------------------------------

#[test]
fn test_session_validate_checks_referenced_paths() {
    use moonlitt_session::persistence::{
        MasterState, Session, SourceState, TrackState, TransportSnapshot,
    };

    let session = Session {
        version: 2,
        sample_rate: 44100,
        master: MasterState {
            volume: 1.0,
            limiter_threshold: -0.1,
        },
        tracks: vec![TrackState {
            id: 0,
            channel_mask: 0xFFFF,
            volume: 1.0,
            trim_db: 0.0,
            pan: 0.5,
            mute: false,
            solo: false,
            send_levels: vec![],
            source: SourceState {
                path: Some("/no/such/plugin.vst3".into()),
                state: None,
                warm_up_blocks: 0,
            },
            inserts: vec![],
            color: None,
        }],
        send_buses: vec![],
        transport: TransportSnapshot::default(),
        sequencer_source: None,
    };

    let dir = std::env::temp_dir().join("moonlitt-capi-validate-test");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("missing-plugin.mlsession");
    session
        .save_to_file(p.to_str().unwrap())
        .expect("write session");

    let cpath = CString::new(p.to_str().unwrap()).unwrap();
    assert_eq!(
        moonlitt_session_validate_file(cpath.as_ptr()),
        MOONLITT_ERR_STATE,
        "session referencing a missing plugin must fail deep validation"
    );
    let msg = unsafe { CStr::from_ptr(moonlitt_last_error_message()) }
        .to_string_lossy()
        .to_string();
    assert!(
        msg.contains("/no/such/plugin.vst3"),
        "message should name the missing file: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Session save from a LIVE runtime (control-side shadow + shared-handle
// state capture) — the first real save-from-runtime path in the project
// ---------------------------------------------------------------------------

#[test]
fn test_runtime_save_session_roundtrip_sf2() {
    let sf2 = "/Users/wangyan/Desktop/stardew valley mods/soundfonts/GeneralUser_GS.sf2";
    if !std::path::Path::new(sf2).exists() {
        eprintln!("SF2 not present — skipping");
        return;
    }
    let e = moonlitt_engine_create(44100, 256);
    let cpath = CString::new(sf2).unwrap();
    assert_eq!(moonlitt_engine_load(e, cpath.as_ptr()), MOONLITT_OK);

    let rt = moonlitt_runtime_create(e);
    if rt.is_null() {
        eprintln!("no audio device — skipping");
        moonlitt_engine_destroy(e);
        return;
    }

    // Mutate mixer state so the saved session must reflect the shadow.
    assert_eq!(moonlitt_runtime_set_track_volume(rt, 0, 0.75), MOONLITT_OK);
    assert_eq!(moonlitt_runtime_set_track_pan(rt, 0, 0.25), MOONLITT_OK);
    assert_eq!(moonlitt_runtime_set_master_volume(rt, 0.5), MOONLITT_OK);

    let dir = std::env::temp_dir().join("moonlitt-capi-session-save");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("roundtrip.mlsession");
    let cfile = CString::new(file.to_str().unwrap()).unwrap();

    assert_eq!(
        moonlitt_runtime_save_session(rt, cfile.as_ptr()),
        MOONLITT_OK,
        "save from a live runtime must succeed"
    );
    assert_eq!(
        moonlitt_session_validate_file(cfile.as_ptr()),
        MOONLITT_OK,
        "the saved session must deep-validate"
    );

    let json_ptr = moonlitt_session_read_json(cfile.as_ptr());
    assert!(!json_ptr.is_null());
    let json = unsafe { CStr::from_ptr(json_ptr) }
        .to_string_lossy()
        .to_string();
    moonlitt_free_string(json_ptr);
    assert!(
        json.contains("GeneralUser_GS.sf2"),
        "session must reference the SF2"
    );
    assert!(json.contains("0.75"), "track volume must be captured");
    assert!(json.contains("0.25"), "track pan must be captured");
    assert!(json.contains("0.5"), "master volume must be captured");

    // And it restores into a working runtime.
    let rt2 = moonlitt_session_load_from_file(cfile.as_ptr(), 256);
    assert!(
        !rt2.is_null(),
        "saved session must load back into a runtime"
    );

    moonlitt_runtime_destroy(rt2);
    moonlitt_runtime_destroy(rt);
    moonlitt_engine_destroy(e);
}

#[test]
fn test_pianoteq_session_save_captures_plugin_state() {
    let Some(plugin) = find_pianoteq() else {
        eprintln!("Pianoteq not installed — skipping");
        return;
    };
    let e = moonlitt_engine_create(44100, 256);
    let cplugin = CString::new(plugin.to_str().unwrap()).unwrap();
    assert_eq!(moonlitt_engine_load(e, cplugin.as_ptr()), MOONLITT_OK);

    let rt = moonlitt_runtime_create(e);
    if rt.is_null() {
        eprintln!("no audio device — skipping");
        moonlitt_engine_destroy(e);
        return;
    }

    let dir = std::env::temp_dir().join("moonlitt-capi-session-save");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("pianoteq.mlsession");
    let cfile = CString::new(file.to_str().unwrap()).unwrap();
    assert_eq!(
        moonlitt_runtime_save_session(rt, cfile.as_ptr()),
        MOONLITT_OK
    );

    // The VST3 state blob must be captured (base64 "state" field non-null).
    let json_ptr = moonlitt_session_read_json(cfile.as_ptr());
    let json = unsafe { CStr::from_ptr(json_ptr) }
        .to_string_lossy()
        .to_string();
    moonlitt_free_string(json_ptr);
    let state_is_null = json.contains(r#""state":null"#) || json.contains(r#""state": null"#);
    assert!(
        json.contains("state") && !state_is_null,
        "live plugin state must be captured into the session (state is null)"
    );

    moonlitt_runtime_destroy(rt);
    moonlitt_engine_destroy(e);
}

// ---------------------------------------------------------------------------
// Keyscape headless replay — THE game-mod workflow, end to end through
// the C ABI: load plugin → restore captured patch → warm up → audible.
// ---------------------------------------------------------------------------

#[test]
fn test_keyscape_headless_replay_through_c_api() {
    let plugin = "/Library/Audio/Plug-Ins/VST3/Keyscape.vst3";
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../moonlitt-vst3/tests/fixtures/keyscape-default.mlstate");
    if !std::path::Path::new(plugin).exists() {
        eprintln!("Keyscape not installed — skipping");
        return;
    }
    if !fixture.exists() {
        eprintln!("Keyscape state fixture missing — skipping");
        return;
    }

    let e = moonlitt_engine_create(44100, 256);
    let cplugin = CString::new(plugin).unwrap();
    assert_eq!(moonlitt_engine_load(e, cplugin.as_ptr()), MOONLITT_OK);
    assert_eq!(moonlitt_engine_supports_state(e), 1);

    // Restore the captured patch.
    let blob = std::fs::read(&fixture).unwrap();
    assert_eq!(
        moonlitt_engine_load_state(e, blob.as_ptr(), blob.len()),
        MOONLITT_OK
    );

    // Sample streamer must advertise warm-up, and warming must succeed.
    let warm = moonlitt_engine_recommended_warmup_blocks(e);
    assert!(warm > 0, "Spectrasonics streamer must advertise warm-up");
    assert_eq!(moonlitt_engine_warm_up(e, warm), MOONLITT_OK);

    // Play a chord and verify audible output through the C render path.
    moonlitt_engine_note_on(e, 0, 60, 100);
    moonlitt_engine_note_on(e, 0, 64, 100);
    moonlitt_engine_note_on(e, 0, 67, 100);

    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];
    let mut peak = 0.0f32;
    for _ in 0..128 {
        assert_eq!(
            moonlitt_engine_render(e, left.as_mut_ptr(), right.as_mut_ptr(), 256),
            MOONLITT_OK
        );
        for s in left.iter().chain(right.iter()) {
            peak = peak.max(s.abs());
        }
    }
    assert!(
        peak > 1e-3,
        "headless Keyscape replay must be audible (peak={peak})"
    );
    println!("✅ Keyscape headless replay via C ABI: peak={peak:.4}");

    moonlitt_engine_destroy(e);
}
