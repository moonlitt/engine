//! moonlitt-desktop Tauri backend.
//!
//! Boots the audio engine, registers Tauri commands, spawns the 60 Hz
//! metering broadcaster.

mod commands;
mod engine;
mod midi_analyze;
mod patch_library;
mod plugin_state_cache;
#[cfg(target_os = "macos")]
mod plugin_window;
mod recent_files;
mod sentinel_scan;
mod state_metadata;

use std::time::Duration;

use tauri::{Emitter, Manager};

use crate::commands::AppState;
use crate::engine::Engine;

const METER_INTERVAL: Duration = Duration::from_millis(16);
/// How often the host-side notification queues are drained (pure Rust,
/// never enters plug-in code — cheap and crash-safe at any moment).
#[cfg(target_os = "macos")]
const PATCH_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// A notification burst must be quiet this long before we risk a
/// `getState` — Spectrasonics keeps emitting while an instrument is
/// still streaming in, so quiet ≈ load finished.
#[cfg(target_os = "macos")]
const PATCH_CAPTURE_QUIET: Duration = Duration::from_millis(1500);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Sentinel probe worker? Probe one VST3 bundle, print JSON, exit —
    // never boot the app. Must be the FIRST thing run() does.
    if sentinel_scan::maybe_run_probe_worker() {
        return;
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(AppState {
                engine: Engine::new(44100, 512),
            });

            // Instantiate plug-ins on the MAIN thread (the industry
            // assumption — JUCE hosts load on the message thread). A
            // plug-in that pops a native dialog during load needs the
            // main run loop for the dialog to be clickable; loading on
            // a worker thread turns that dialog into an undismissable
            // zombie and wedges the engine (observed with a
            // half-installed Omnisphere). The command thread waits with
            // a timeout so a hung load reports an error instead of
            // freezing the app forever.
            let loader_handle = app.handle().clone();
            crate::engine::install_backend_loader(move |path, sr, buf| {
                if on_main_thread() {
                    return crate::engine::create_backend_direct(path, sr, buf);
                }
                let (tx, rx) = std::sync::mpsc::channel();
                let p = path.to_string();
                loader_handle
                    .run_on_main_thread(move || {
                        let _ = tx.send(crate::engine::create_backend_direct(&p, sr, buf));
                    })
                    .map_err(|e| format!("main-thread dispatch: {e}"))?;
                match rx.recv_timeout(Duration::from_secs(30)) {
                    Ok(result) => result,
                    Err(_) => Err(format!(
                        "插件加载超过 30 秒未完成（{path}）。\
                         它可能在等待一个对话框或已挂起 —— 检查屏幕上有没有插件弹窗，\
                         或重装该插件。"
                    )),
                }
            });

            let app_handle = app.handle().clone();
            std::thread::spawn(move || meter_loop(app_handle));

            #[cfg(target_os = "macos")]
            {
                let patch_handle = app.handle().clone();
                std::thread::spawn(move || patch_poll_loop(patch_handle));
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::cmd_snapshot,
            commands::cmd_transport_play,
            commands::cmd_transport_pause,
            commands::cmd_transport_stop,
            commands::cmd_transport_set_bpm,
            commands::cmd_transport_set_loop,
            commands::cmd_transport_set_metronome,
            commands::cmd_master_set_volume,
            commands::cmd_default_set_instrument,
            commands::cmd_channel_set_override,
            commands::cmd_channel_remove_override,
            commands::cmd_channel_set_volume,
            commands::cmd_channel_set_pan,
            commands::cmd_channel_set_mute,
            commands::cmd_channel_set_solo,
            commands::cmd_channel_set_color,
            commands::cmd_channel_set_program,
            commands::cmd_insert_add,
            commands::cmd_insert_remove,
            commands::cmd_insert_set_param,
            commands::cmd_insert_set_bypass,
            commands::cmd_send_bus_add,
            commands::cmd_channel_set_send_level,
            commands::cmd_plugins_scan,
            commands::cmd_load_midi,
            commands::cmd_open_plugin_gui,
            commands::cmd_save_plugin_state,
            commands::cmd_patch_library_list,
            commands::cmd_patch_library_load,
            commands::cmd_project_save_as,
            commands::cmd_project_open,
            commands::cmd_project_recent_list,
            commands::cmd_project_clear_recent,
            commands::cmd_project_forget_recent,
            commands::cmd_render_to_wav,
            commands::cmd_send_bus_set_param,
            commands::cmd_transport_seek,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn meter_loop(app: tauri::AppHandle) {
    loop {
        std::thread::sleep(METER_INTERVAL);
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        if !state.engine.is_runtime_started() {
            continue;
        }
        let snap = state.engine.meter_snapshot();
        // Emit as JSON array — small, simple, decodes everywhere.
        let _ = app.emit("meter", snap);
    }
}

/// Background loop that keeps the UI's patch-name display in sync with
/// the live plug-in state while a GUI window is open. Picks inside the
/// plug-in's native UI never go through any Tauri command, so without
/// this the bar would only refresh on window close.
///
/// Event-driven, NOT a state poll: each tick drains the host-side
/// notification queues (cheap, never enters plug-in code). Only after a
/// notification burst has been quiet for [`PATCH_CAPTURE_QUIET`] do we
/// call `getState` once. Spectrasonics' `getState` crashes when called
/// while its own threads are mid-instrument-load — the quiet period
/// waits the load out instead of probing blindly every 500 ms.
#[cfg(target_os = "macos")]
fn patch_poll_loop(app: tauri::AppHandle) {
    use serde::Serialize;
    use std::collections::HashMap;
    use std::time::Instant;

    #[derive(Serialize, Clone)]
    #[serde(rename_all = "camelCase")]
    struct PluginStateCaptured {
        path: String,
        patch_name: Option<String>,
    }

    let mut last_activity: HashMap<String, Instant> = HashMap::new();
    let mut last_seen: HashMap<String, Option<String>> = HashMap::new();
    loop {
        std::thread::sleep(PATCH_POLL_INTERVAL);
        if plugin_window::open_window_count() == 0 {
            last_activity.clear();
            continue;
        }
        let now = Instant::now();
        for path in plugin_window::poll_activity() {
            last_activity.insert(path, now);
        }
        let ready: Vec<String> = last_activity
            .iter()
            .filter(|(_, t)| now.duration_since(**t) >= PATCH_CAPTURE_QUIET)
            .map(|(p, _)| p.clone())
            .collect();
        for path in ready {
            last_activity.remove(&path);
            let Some((patch_name, state_bytes)) = plugin_window::capture_state_for(&path) else {
                // Plug-in busy (audio thread holds it) — re-arm and
                // retry after another quiet period.
                last_activity.insert(path, Instant::now());
                continue;
            };
            if last_seen.get(&path) == Some(&patch_name) {
                continue;
            }
            last_seen.insert(path.clone(), patch_name.clone());
            // Remember this patch as the plug-in's default, so the next
            // time it's picked as an instrument it sounds immediately.
            plugin_state_cache::store(&app, &path, &state_bytes);
            let _ = app.emit(
                "plugin_state_captured",
                PluginStateCaptured { path, patch_name },
            );
        }
    }
}

/// Is the caller already on the process main thread? Guards the
/// main-thread loader against self-deadlock: `run_on_main_thread` from
/// the main thread would queue behind the very wait we're about to do.
fn on_main_thread() -> bool {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn pthread_main_np() -> std::os::raw::c_int;
        }
        unsafe { pthread_main_np() == 1 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}
