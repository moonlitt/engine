//! # moonlitt-clap
//!
//! Pure Rust CLAP plugin hosting. Load, play, and render any CLAP instrument or effect.
//!
//! CLAP (CLever Audio Plugin) uses a pure C API — no COM, no reference counting.
//! This makes it significantly simpler to host than VST3.
//!
//! ```no_run
//! use moonlitt_clap::{ClapHost, ClapPlugin};
//!
//! let host = ClapHost::new(44100, 256).unwrap();
//! let plugins = host.scan().unwrap();
//! let mut plugin = host.load(&plugins[0]).unwrap();
//! plugin.note_on(0, 60, 100);
//! let mut left = vec![0.0f32; 256];
//! let mut right = vec![0.0f32; 256];
//! plugin.render(&mut left, &mut right).unwrap();
//! ```

mod error;
mod events;
mod host;
mod module;
mod scanner;

pub use error::{Error, Result};
pub use events::{MidiEvent, MidiEventKind};
pub use scanner::PluginInfo;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::ext::params::{clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, CLAP_PROCESS_ERROR};
use events::{InputEventList, OutputEventList};
use host::HostContext;
use module::ClapModule;
use std::ffi::{CStr, CString};
use std::pin::Pin;
use std::ptr;

/// CLAP host — scans, loads, and manages CLAP plugins.
pub struct ClapHost {
    sample_rate: f64,
    buffer_size: u32,
}

/// A loaded and initialized CLAP plugin instance.
pub struct ClapPlugin {
    /// The raw clap_plugin pointer (owned by us; we call destroy on drop).
    plugin: *const clap_plugin,
    /// Keep the module alive so the shared library stays loaded.
    _module: ClapModule,
    /// Keep the host context alive (plugin holds a pointer to it).
    _host: Pin<Box<HostContext>>,
    /// Pending MIDI events (drained on each render call).
    pending_events: Vec<MidiEvent>,
    /// Plugin name (cached from descriptor).
    plugin_name: String,
    /// Params extension (if supported by plugin).
    params_ext: Option<*const clap_plugin_params>,
    #[allow(dead_code)]
    sample_rate: f64,
    #[allow(dead_code)]
    buffer_size: u32,
}

// SAFETY: ClapPlugin is `Send` because:
// 1. `*const clap_plugin` — CLAP spec requires the host to call process() from a
//    single designated "audio thread". Once activated, the plugin instance can be
//    moved to that thread. We only ever call process() from one thread at a time.
// 2. `ClapModule` — holds a dlopen handle (opaque `*mut c_void`), which is a plain
//    integer-like pointer safe to move across threads.
// 3. `Pin<Box<HostContext>>` — HostContext contains only the clap_host vtable struct,
//    which is read-only after construction.
// 4. `Vec<MidiEvent>`, `String`, `f64`, `u32` — all inherently Send.
//
// ClapPlugin is intentionally NOT Sync — concurrent render() calls from multiple
// threads violate the CLAP processing contract.
unsafe impl Send for ClapPlugin {}

impl ClapHost {
    /// Create a new CLAP host with the given sample rate and buffer size.
    pub fn new(sample_rate: u32, buffer_size: u32) -> Result<Self> {
        Ok(Self {
            sample_rate: sample_rate as f64,
            buffer_size,
        })
    }

    /// Scan default system paths for CLAP plugins.
    pub fn scan(&self) -> Result<Vec<PluginInfo>> {
        scanner::scan_default_paths()
    }

    /// Probe a specific .clap bundle path and load the first plugin.
    /// This avoids scanning all system directories.
    pub fn load_from_path(&self, path: &std::path::Path) -> Result<ClapPlugin> {
        let plugins = scanner::probe_path(path)?;
        let info = plugins
            .into_iter()
            .next()
            .ok_or_else(|| Error::LoadFailed("no plugins found in bundle".into()))?;
        self.load(&info)
    }

    /// Load a plugin from PluginInfo.
    ///
    /// Performs the full CLAP lifecycle:
    ///   factory.create_plugin → plugin.init → plugin.activate → plugin.start_processing
    pub fn load(&self, info: &PluginInfo) -> Result<ClapPlugin> {
        // 1. Load the module (dlopen + clap_entry.init + get_factory)
        let module = ClapModule::load(&info.path)?;

        // 2. Create host context
        let host_ctx = HostContext::new();

        // 3. Create plugin instance via factory
        let plugin_id = CString::new(info.plugin_id.as_str())
            .map_err(|e| Error::LoadFailed(e.to_string()))?;

        let factory = module.factory();
        let plugin = unsafe {
            let create_fn = (*factory)
                .create_plugin
                .ok_or(Error::LoadFailed("create_plugin is null".into()))?;
            create_fn(factory, host_ctx.as_ptr(), plugin_id.as_ptr())
        };

        if plugin.is_null() {
            return Err(Error::LoadFailed(format!(
                "create_plugin returned null for '{}'",
                info.plugin_id
            )));
        }

        // 4. plugin.init()
        unsafe {
            let init_fn = (*plugin)
                .init
                .ok_or(Error::PluginError("init is null"))?;
            if !init_fn(plugin) {
                // Must destroy on failure
                if let Some(destroy) = (*plugin).destroy {
                    destroy(plugin);
                }
                return Err(Error::PluginError("plugin.init() returned false"));
            }
        }

        // 5. plugin.activate(sample_rate, min_frames, max_frames)
        unsafe {
            let activate_fn = (*plugin)
                .activate
                .ok_or(Error::PluginError("activate is null"))?;
            if !activate_fn(plugin, self.sample_rate, 1, self.buffer_size) {
                if let Some(destroy) = (*plugin).destroy {
                    destroy(plugin);
                }
                return Err(Error::PluginError("plugin.activate() returned false"));
            }
        }

        // 6. plugin.start_processing()
        unsafe {
            if let Some(start) = (*plugin).start_processing {
                if !start(plugin) {
                    // Some plugins may not support start_processing;
                    // we continue anyway since process() may still work.
                }
            }
        }

        // 7. Query params extension
        let params_ext = unsafe {
            match (*plugin).get_extension {
                Some(get_ext) => {
                    let ext = get_ext(plugin, CLAP_EXT_PARAMS.as_ptr());
                    if ext.is_null() { None } else { Some(ext as *const clap_plugin_params) }
                }
                None => None,
            }
        };

        Ok(ClapPlugin {
            plugin,
            _module: module,
            _host: host_ctx,
            pending_events: Vec::new(),
            plugin_name: info.name.clone(),
            params_ext,
            sample_rate: self.sample_rate,
            buffer_size: self.buffer_size,
        })
    }
}

impl ClapPlugin {
    /// Queue a Note On event (will be sent on next render call).
    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        self.pending_events.push(MidiEvent {
            kind: MidiEventKind::NoteOn {
                channel,
                note,
                velocity,
            },
            sample_offset: 0,
        });
    }

    /// Queue a Note Off event.
    pub fn note_off(&mut self, channel: u8, note: u8) {
        self.pending_events.push(MidiEvent {
            kind: MidiEventKind::NoteOff { channel, note },
            sample_offset: 0,
        });
    }

    /// Queue a CC (Control Change) event.
    pub fn cc(&mut self, channel: u8, cc: u8, value: u8) {
        self.pending_events.push(MidiEvent {
            kind: MidiEventKind::CC { channel, cc, value },
            sample_offset: 0,
        });
    }

    /// Queue a Pitch Bend event.
    pub fn pitch_bend(&mut self, channel: u8, value: i16) {
        self.pending_events.push(MidiEvent {
            kind: MidiEventKind::PitchBend { channel, value },
            sample_offset: 0,
        });
    }

    /// Queue Note Off for all 128 notes (panic).
    pub fn all_notes_off(&mut self) {
        for note in 0..128u8 {
            self.pending_events.push(MidiEvent {
                kind: MidiEventKind::NoteOff {
                    channel: 0,
                    note,
                },
                sample_offset: 0,
            });
        }
    }

    /// Render one buffer of audio. Drains all pending MIDI events.
    ///
    /// `left` and `right` must be the same length (the buffer size).
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<()> {
        let num_frames = left.len().min(right.len()) as u32;
        if num_frames == 0 {
            return Ok(());
        }

        // Zero output buffers
        left.fill(0.0);
        right.fill(0.0);

        // Build input events from pending MIDI
        let events: Vec<MidiEvent> = std::mem::take(&mut self.pending_events);
        let input_events = InputEventList::from_midi_events(&events);
        let output_events = OutputEventList::new();

        // Build audio output buffer
        let mut channel_ptrs: [*mut f32; 2] = [left.as_mut_ptr(), right.as_mut_ptr()];
        let mut audio_output = clap_audio_buffer {
            data32: channel_ptrs.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };

        // Build process data
        let process_data = clap_process {
            steady_time: -1, // unknown
            frames_count: num_frames,
            transport: ptr::null(),
            audio_inputs: ptr::null(),
            audio_outputs: &mut audio_output,
            audio_inputs_count: 0,
            audio_outputs_count: 1,
            in_events: input_events.as_ptr(),
            out_events: output_events.as_ptr(),
        };

        // Call plugin.process()
        let status = unsafe {
            let process_fn = (*self.plugin)
                .process
                .ok_or(Error::PluginError("process is null"))?;
            process_fn(self.plugin, &process_data)
        };

        if status == CLAP_PROCESS_ERROR {
            return Err(Error::PluginError("process returned error"));
        }

        Ok(())
    }

    /// Get the plugin's display name.
    pub fn name(&self) -> &str {
        &self.plugin_name
    }

    // --- Parameters ---

    pub fn param_count(&self) -> u32 {
        let ext = match self.params_ext {
            Some(e) => e,
            None => return 0,
        };
        unsafe {
            match (*ext).count {
                Some(f) => f(self.plugin),
                None => 0,
            }
        }
    }

    pub fn param_info(&self, index: u32) -> Option<ClapParamInfo> {
        let ext = self.params_ext?;
        let get_info = unsafe { (*ext).get_info? };
        let mut info = unsafe { std::mem::zeroed::<clap_sys::ext::params::clap_param_info>() };
        let ok = unsafe { get_info(self.plugin, index, &mut info) };
        if !ok {
            return None;
        }
        Some(ClapParamInfo {
            id: info.id,
            name: unsafe { CStr::from_ptr(info.name.as_ptr()) }
                .to_string_lossy()
                .into_owned(),
            module: unsafe { CStr::from_ptr(info.module.as_ptr()) }
                .to_string_lossy()
                .into_owned(),
            min: info.min_value,
            max: info.max_value,
            default: info.default_value,
            flags: info.flags,
        })
    }

    pub fn get_param(&self, id: u32) -> Option<f64> {
        let ext = self.params_ext?;
        let get_value = unsafe { (*ext).get_value? };
        let mut value = 0.0f64;
        let ok = unsafe { get_value(self.plugin, id, &mut value) };
        if ok { Some(value) } else { None }
    }

    pub fn set_param(&mut self, _id: u32, _value: f64) {
        // CLAP params are set via process events, not directly.
        // For now, queue as a pending event would be needed.
        // TODO: implement via CLAP_EVENT_PARAM_VALUE in next process call
    }

    pub fn param_display(&self, id: u32, value: f64) -> Option<String> {
        let ext = self.params_ext?;
        let value_to_text = unsafe { (*ext).value_to_text? };
        let mut buf = [0i8; 256];
        let ok = unsafe { value_to_text(self.plugin, id, value, buf.as_mut_ptr(), 256) };
        if ok {
            Some(
                unsafe { CStr::from_ptr(buf.as_ptr()) }
                    .to_string_lossy()
                    .into_owned(),
            )
        } else {
            None
        }
    }
}

/// Parameter info from a CLAP plugin.
#[derive(Debug, Clone)]
pub struct ClapParamInfo {
    pub id: u32,
    pub name: String,
    pub module: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub flags: u32,
}

impl Drop for ClapPlugin {
    fn drop(&mut self) {
        unsafe {
            // stop_processing
            if let Some(stop) = (*self.plugin).stop_processing {
                stop(self.plugin);
            }

            // deactivate
            if let Some(deactivate) = (*self.plugin).deactivate {
                deactivate(self.plugin);
            }

            // destroy
            if let Some(destroy) = (*self.plugin).destroy {
                destroy(self.plugin);
            }
        }
    }
}
