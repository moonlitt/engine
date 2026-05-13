//! moonlitt-desktop Tauri backend.
//!
//! Boots the audio engine, registers Tauri commands, spawns the 60 Hz
//! metering broadcaster.

mod commands;
mod engine;
mod midi_analyze;
#[cfg(target_os = "macos")]
mod plugin_window;
mod recent_files;
mod state_metadata;

use std::time::Duration;

use tauri::{Emitter, Manager};

use crate::commands::AppState;
use crate::engine::Engine;

const METER_INTERVAL: Duration = Duration::from_millis(16);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(AppState {
                engine: Engine::new(44100, 512),
            });

            let app_handle = app.handle().clone();
            std::thread::spawn(move || meter_loop(app_handle));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::cmd_snapshot,
            commands::cmd_transport_play,
            commands::cmd_transport_stop,
            commands::cmd_transport_set_bpm,
            commands::cmd_master_set_volume,
            commands::cmd_default_set_instrument,
            commands::cmd_channel_set_override,
            commands::cmd_channel_remove_override,
            commands::cmd_channel_set_volume,
            commands::cmd_channel_set_mute,
            commands::cmd_channel_set_solo,
            commands::cmd_channel_set_program,
            commands::cmd_insert_add,
            commands::cmd_insert_remove,
            commands::cmd_insert_set_param,
            commands::cmd_plugins_scan,
            commands::cmd_load_midi,
            commands::cmd_open_plugin_gui,
            commands::cmd_apply_open_plugin_state,
            commands::cmd_save_plugin_state,
            commands::cmd_project_save_as,
            commands::cmd_project_open,
            commands::cmd_project_recent_list,
            commands::cmd_project_clear_recent,
            commands::cmd_project_forget_recent,
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
