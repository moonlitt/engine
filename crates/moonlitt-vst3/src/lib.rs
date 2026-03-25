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
mod error;
mod events;
mod host;
mod module;
mod processor;
mod scanner;

pub use error::{Error, Result};
pub use events::{MidiEvent, MidiEventKind};
pub use scanner::PluginInfo;

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
    pending_events: Vec<MidiEvent>,
    #[allow(dead_code)]
    sample_rate: f64,
    #[allow(dead_code)]
    buffer_size: usize,
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

        Ok(Vst3Plugin {
            inner: loaded,
            pending_events: Vec::new(),
            sample_rate: self.sample_rate,
            buffer_size: self.buffer_size,
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
        let events: Vec<MidiEvent> = std::mem::take(&mut self.pending_events);
        processor::process_audio(
            &self.inner.processor,
            &self.inner.component,
            left,
            right,
            &events,
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

            if pinfo.flags & ParameterFlags_::kIsProgramChange as i32 != 0 {
                let normalized = if pinfo.stepCount > 0 {
                    id as f64 / pinfo.stepCount as f64
                } else {
                    0.0
                };
                unsafe { ctrl.setParamNormalized(pinfo.id, normalized) };
                return Ok(());
            }
        }

        Err(Error::NotSupported)
    }

    /// Get the plugin's display name.
    pub fn name(&self) -> &str {
        &self.inner.class_info.name
    }
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
