//! # moonlitt-vst3
//!
//! Pure Rust VST3 plugin hosting. Load, play, and render any VST3 instrument or effect.
//!
//! ```no_run
//! use moonlitt_vst3::{Vst3Host, Vst3Plugin};
//!
//! let host = Vst3Host::new(44100, 256).unwrap();
//! let plugins = host.scan().unwrap();
//! let mut plugin = host.load(&plugins[0]).unwrap();
//! plugin.note_on(0, 60, 100);
//! let mut left = vec![0.0f32; 256];
//! let mut right = vec![0.0f32; 256];
//! plugin.render(&mut left, &mut right).unwrap();
//! ```

mod component;
mod component_handler;
mod error;
mod events;
mod host;
mod module;
mod parameter_changes;
mod processor;
mod scanner;
pub mod stream;
pub mod view;

pub use error::{Error, Result};
pub use events::{MidiEvent, MidiEventKind};
pub use scanner::PluginInfo;
pub use view::{platform, Vst3PluginView};

use component::LoadedPlugin;

/// VST3 host — scans, loads, and manages VST3 plugins.
pub struct Vst3Host {
    sample_rate: f64,
    buffer_size: usize,
    host: vst3::ComWrapper<host::HostApp>,
}

/// A loaded and initialized VST3 plugin instance.
pub struct Vst3Plugin {
    inner: LoadedPlugin,
    /// Pending MIDI events drained on each render call.
    /// Unbounded in theory, but in practice limited by the calling rate
    /// (~172 calls/sec at 44100 Hz / 256 buffer). A single buffer rarely
    /// accumulates more than a handful of events.
    pending_events: Vec<MidiEvent>,
    #[allow(dead_code)]
    sample_rate: f64,
    #[allow(dead_code)]
    buffer_size: usize,
    /// Pre-allocated silent input buffer (left channel) to avoid hot-path allocation.
    silent_left: Vec<f32>,
    /// Pre-allocated silent input buffer (right channel) to avoid hot-path allocation.
    silent_right: Vec<f32>,
}

/// Information about a factory preset.
#[derive(Debug, Clone)]
pub struct PresetInfo {
    pub list_id: i32,
    pub program_index: i32,
    pub name: String,
}

impl Vst3Host {
    /// Create a new VST3 host with the given sample rate and buffer size.
    pub fn new(sample_rate: u32, buffer_size: u32) -> Result<Self> {
        Ok(Self {
            sample_rate: sample_rate as f64,
            buffer_size: buffer_size as usize,
            host: host::create_host(),
        })
    }

    /// Scan default system paths for VST3 plugins.
    pub fn scan(&self) -> Result<Vec<PluginInfo>> {
        scanner::scan_default_paths()
    }

    /// Probe a specific .vst3 bundle path and load the first audio class.
    /// This avoids scanning all system directories.
    pub fn load_from_path(&self, path: &std::path::Path) -> Result<Vst3Plugin> {
        let plugins = scanner::probe_path(path)?;
        let info = plugins
            .into_iter()
            .next()
            .ok_or_else(|| Error::LoadFailed("no audio classes found in bundle".into()))?;
        self.load(&info)
    }

    /// Load a plugin from PluginInfo.
    pub fn load(&self, info: &PluginInfo) -> Result<Vst3Plugin> {
        let module = module::load_module(&info.path)?;
        let loaded = component::load_plugin(
            module,
            &info.class_id,
            &self.host,
            self.sample_rate,
            self.buffer_size,
        )?;

        let bs = self.buffer_size;
        Ok(Vst3Plugin {
            inner: loaded,
            pending_events: Vec::new(),
            sample_rate: self.sample_rate,
            buffer_size: bs,
            silent_left: vec![0.0f32; bs],
            silent_right: vec![0.0f32; bs],
        })
    }
}

impl Vst3Plugin {
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

    /// Queue a MIDI Program Change event. Translated to a VST3 legacy MIDI
    /// CC out event (controlNumber=130), so plugins that respond to MIDI PC
    /// (GM-compatible synths) switch programs on the next render.
    pub fn program_change(&mut self, channel: u8, program: u8) {
        self.pending_events.push(MidiEvent {
            kind: MidiEventKind::ProgramChange { channel, program },
            sample_offset: 0,
        });
    }

    /// Send All Notes Off (CC#123) on all 16 channels.
    ///
    /// Uses the standard MIDI CC#123 (All Notes Off) message instead of
    /// sending 128 individual NoteOff events, reducing event list overhead.
    pub fn all_notes_off(&mut self) {
        for channel in 0..16u8 {
            self.pending_events.push(MidiEvent {
                kind: MidiEventKind::CC {
                    channel,
                    cc: 123,  // All Notes Off
                    value: 0,
                },
                sample_offset: 0,
            });
        }
    }

    /// Render one buffer of audio (instrument mode). Drains all pending MIDI events.
    ///
    /// `left` and `right` must be the same length (the buffer size).
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<()> {
        let events: Vec<MidiEvent> = std::mem::take(&mut self.pending_events);
        // Drain controller→processor parameter edits queued since the last
        // render (e.g. from load_preset, or from the plugin's UI).
        let pending_params = match &self.inner.param_queue {
            Some(q) => component_handler::drain(q),
            None => Vec::new(),
        };
        // Re-zero silent buffers before each render (plugins may write into them)
        let num_frames = left.len().min(right.len());
        self.silent_left[..num_frames].fill(0.0);
        self.silent_right[..num_frames].fill(0.0);
        processor::process_audio(
            &self.inner.processor,
            &self.inner.component,
            left,
            right,
            &events,
            &pending_params,
            &mut self.silent_left[..num_frames],
            &mut self.silent_right[..num_frames],
        )
    }

    /// Process audio through the plugin as an effect (audio in → audio out).
    ///
    /// Reads from `in_left`/`in_right`, writes processed audio to `out_left`/`out_right`.
    pub fn process_effect(
        &mut self,
        in_left: &[f32],
        in_right: &[f32],
        out_left: &mut [f32],
        out_right: &mut [f32],
    ) -> Result<()> {
        processor::process_effect(
            &self.inner.processor,
            &self.inner.component,
            in_left,
            in_right,
            out_left,
            out_right,
        )
    }

    /// List factory presets via IUnitInfo (if the plugin supports it).
    pub fn presets(&self) -> Result<Vec<PresetInfo>> {
        use vst3::Steinberg::Vst::{
            IUnitInfo, IUnitInfoTrait, ProgramListInfo, String128,
        };
        use vst3::Steinberg::kResultOk;

        // Try to get IUnitInfo from controller, then component
        let unit_info: Option<vst3::ComPtr<IUnitInfo>> = self
            .inner
            .controller
            .as_ref()
            .and_then(|c| c.cast::<IUnitInfo>())
            .or_else(|| self.inner.component.cast::<IUnitInfo>());

        let unit_info =
            unit_info.ok_or(Error::NotSupported)?;

        let list_count = unsafe { unit_info.getProgramListCount() };
        let mut presets = Vec::new();

        for li in 0..list_count {
            let mut list_info = std::mem::MaybeUninit::<ProgramListInfo>::uninit();
            if unsafe { unit_info.getProgramListInfo(li, list_info.as_mut_ptr()) } != kResultOk {
                continue;
            }
            let list_info = unsafe { list_info.assume_init() };

            for pi in 0..list_info.programCount {
                let mut name128: String128 = [0u16; 128];
                if unsafe {
                    unit_info.getProgramName(list_info.id, pi, &mut name128)
                } != kResultOk
                {
                    continue;
                }

                let name = string128_to_string(&name128);
                presets.push(PresetInfo {
                    list_id: list_info.id,
                    program_index: pi,
                    name,
                });
            }
        }

        Ok(presets)
    }

    /// Load a preset by list ID and program index.
    pub fn load_preset(&mut self, id: i32) -> Result<()> {
        use vst3::Steinberg::Vst::{
            IEditControllerTrait, ParameterInfo, ParameterInfo_::ParameterFlags_,
        };
        use vst3::Steinberg::kResultOk;

        let ctrl = self
            .inner
            .controller
            .as_ref()
            .ok_or(Error::NotSupported)?;

        let param_count = unsafe { ctrl.getParameterCount() };

        for i in 0..param_count {
            let mut pinfo = std::mem::MaybeUninit::<ParameterInfo>::uninit();
            if unsafe { ctrl.getParameterInfo(i, pinfo.as_mut_ptr()) } != kResultOk {
                continue;
            }
            let pinfo = unsafe { pinfo.assume_init() };

            if pinfo.flags & ParameterFlags_::kIsProgramChange != 0 {
                let normalized = if pinfo.stepCount > 0 {
                    id as f64 / pinfo.stepCount as f64
                } else {
                    0.0
                };
                unsafe { ctrl.setParamNormalized(pinfo.id, normalized) };
                // Host-initiated edits don't round-trip via performEdit
                // (the controller doesn't notify the host of writes the host
                // itself made). Forward directly so the processor sees it on
                // the next render().
                if let Some(ref queue) = self.inner.param_queue {
                    if let Ok(mut q) = queue.lock() {
                        q.push(component_handler::PendingParam {
                            id: pinfo.id,
                            value: normalized,
                        });
                    }
                }
                return Ok(());
            }
        }

        Err(Error::NotSupported)
    }

    /// Get the plugin's display name.
    pub fn name(&self) -> &str {
        &self.inner.class_info.name
    }

    /// Set plugin state from a binary blob.
    /// Handles the full stop → deactivate → setState → activate → start cycle.
    pub fn set_state(&mut self, data: &[u8]) -> Result<()> {
        use vst3::Steinberg::Vst::{IAudioProcessorTrait, IComponentTrait};
        use vst3::Steinberg::kResultOk;

        // 1. Stop processing
        unsafe { let _ = self.inner.processor.setProcessing(0); }
        // 2. Deactivate
        unsafe { let _ = self.inner.component.setActive(0); }

        // 3. Set state on component
        let mut stream = stream::MemoryStream::from_data(data.to_vec());
        let ptr = stream.as_ibstream_ptr();
        let comp_result = unsafe { self.inner.component.setState(ptr) };

        // 4. Sync controller state
        if let Some(ref ctrl) = self.inner.controller {
            use vst3::Steinberg::Vst::IEditControllerTrait;
            let mut stream2 = stream::MemoryStream::from_data(data.to_vec());
            let ptr2 = stream2.as_ibstream_ptr();
            let _ = unsafe { ctrl.setComponentState(ptr2) };
        }

        // 5. Reactivate
        unsafe { let _ = self.inner.component.setActive(1); }
        // 6. Restart processing
        unsafe { let _ = self.inner.processor.setProcessing(1); }

        if comp_result != kResultOk {
            return Err(Error::Other(format!("setState failed: {comp_result}")));
        }
        Ok(())
    }

    /// Get current plugin state as raw bytes.
    #[must_use = "discarding plugin state bytes is likely a bug"]
    pub fn get_state(&self) -> Result<Vec<u8>> {
        use vst3::Steinberg::Vst::IComponentTrait;
        use vst3::Steinberg::kResultOk;

        let mut stream = stream::MemoryStream::new_writable();
        let ptr = stream.as_ibstream_ptr();
        let result = unsafe { self.inner.component.getState(ptr) };
        if result != kResultOk {
            return Err(Error::Other(format!("getState failed: {result}")));
        }
        Ok(stream.data().to_vec())
    }

    /// Load an SFZ file into sfizz by constructing and setting its state.
    pub fn load_sfizz_file(&mut self, sfz_path: &str) -> Result<()> {
        let state = stream::build_sfizz_state(sfz_path);
        self.set_state(&state)
    }

    // --- Parameters ---

    /// Number of parameters exposed by the plugin.
    pub fn param_count(&self) -> u32 {
        use vst3::Steinberg::Vst::IEditControllerTrait;
        match self.inner.controller.as_ref() {
            Some(ctrl) => (unsafe { ctrl.getParameterCount() }) as u32,
            None => 0,
        }
    }

    /// Get info for parameter at `index` (0-based).
    pub fn param_info(&self, index: u32) -> Option<Vst3ParamInfo> {
        use vst3::Steinberg::Vst::{IEditControllerTrait, ParameterInfo, ParameterInfo_::ParameterFlags_};
        use vst3::Steinberg::kResultOk;

        let ctrl = self.inner.controller.as_ref()?;
        let mut pinfo = std::mem::MaybeUninit::<ParameterInfo>::uninit();
        if unsafe { ctrl.getParameterInfo(index as i32, pinfo.as_mut_ptr()) } != kResultOk {
            return None;
        }
        let pinfo = unsafe { pinfo.assume_init() };

        let flags = pinfo.flags;
        Some(Vst3ParamInfo {
            id: pinfo.id,
            name: string128_to_string(&pinfo.title),
            short_name: string128_to_string(&pinfo.shortTitle),
            units: string128_to_string(&pinfo.units),
            step_count: pinfo.stepCount as u32,
            default_normalized: pinfo.defaultNormalizedValue,
            is_hidden: flags & ParameterFlags_::kIsHidden != 0,
            is_readonly: flags & ParameterFlags_::kIsReadOnly != 0,
            is_program_change: flags & ParameterFlags_::kIsProgramChange != 0,
            is_bypass: flags & ParameterFlags_::kIsBypass != 0,
        })
    }

    /// Get current normalized value (0.0-1.0) for a parameter.
    pub fn get_param(&self, id: u32) -> Option<f64> {
        use vst3::Steinberg::Vst::IEditControllerTrait;
        let ctrl = self.inner.controller.as_ref()?;
        Some(unsafe { ctrl.getParamNormalized(id) })
    }

    /// Set normalized value (0.0-1.0) for a parameter.
    pub fn set_param(&mut self, id: u32, value: f64) {
        use vst3::Steinberg::Vst::IEditControllerTrait;
        if let Some(ctrl) = self.inner.controller.as_ref() {
            unsafe { ctrl.setParamNormalized(id, value) };
        }
    }

    /// Get display string for a parameter value.
    pub fn param_display(&self, id: u32, value: f64) -> Option<String> {
        use vst3::Steinberg::Vst::IEditControllerTrait;
        use vst3::Steinberg::kResultOk;

        let ctrl = self.inner.controller.as_ref()?;
        let mut buf = [0u16; 128];
        if unsafe { ctrl.getParamStringByValue(id, value, &mut buf) } == kResultOk {
            Some(string128_to_string(&buf))
        } else {
            None
        }
    }
}

/// Parameter info from a VST3 plugin.
#[derive(Debug, Clone)]
pub struct Vst3ParamInfo {
    pub id: u32,
    pub name: String,
    pub short_name: String,
    pub units: String,
    pub step_count: u32,
    pub default_normalized: f64,
    pub is_hidden: bool,
    pub is_readonly: bool,
    pub is_program_change: bool,
    pub is_bypass: bool,
}

impl Drop for Vst3Plugin {
    fn drop(&mut self) {
        component::unload_plugin(&mut self.inner);
    }
}

/// Convert a UTF-16 String128 to a Rust String.
fn string128_to_string(s: &[u16; 128]) -> String {
    let end = s.iter().position(|&c| c == 0).unwrap_or(128);
    String::from_utf16_lossy(&s[..end])
}
