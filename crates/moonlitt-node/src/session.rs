//! Audio session — wraps moonlitt-audio-io Runtime.
//!
//! Manages real-time audio output, MIDI events, mixer, and transport.
//! The full Session/AudioProcessor split is a future refinement;
//! for now this directly wraps Runtime.

use napi::Result;
use napi_derive::napi;

use crate::engine::Backend;
use crate::types::{MidiDevice, TrackLevels};

/// Audio session — manages real-time playback, tracks, and mixing.
///
/// Created from a `Backend` via `Session.create()`. The backend is consumed
/// and placed in a mixer as the primary track.
#[napi]
pub struct Session {
    runtime: Option<moonlitt_audio_io::Runtime>,
}

#[napi]
impl Session {
    /// Create a new session from an audio backend.
    ///
    /// The backend is consumed (moved into the internal mixer).
    /// To add more instruments, use `addTrack()` afterward.
    #[napi(factory)]
    pub fn create(
        backend: &mut Backend,
        sample_rate: u32,
        buffer_size: u32,
    ) -> Result<Session> {
        let b = backend
            .inner
            .take()
            .ok_or_else(|| napi::Error::from_reason("Backend already consumed"))?;
        let runtime = moonlitt_audio_io::Runtime::new(b, sample_rate, buffer_size)
            .map_err(|(e, _backend)| napi::Error::from_reason(e))?;
        Ok(Session {
            runtime: Some(runtime),
        })
    }

    // --- Audio output ---

    /// Start audio output (opens the system audio device).
    #[napi]
    pub fn start(&self) -> Result<()> {
        self.rt()?
            .start()
            .map_err(napi::Error::from_reason)
    }

    /// Stop (pause) audio output.
    #[napi]
    pub fn stop(&self) -> Result<()> {
        self.rt()?
            .stop()
            .map_err(napi::Error::from_reason)
    }

    // --- Transport ---

    /// Begin transport playback.
    #[napi]
    pub fn play(&self) -> Result<()> {
        self.rt()?.play();
        Ok(())
    }

    /// Pause transport.
    #[napi]
    pub fn pause(&self) -> Result<()> {
        self.rt()?.pause_playback();
        Ok(())
    }

    /// Stop transport (resets to beginning).
    #[napi]
    pub fn stop_playback(&self) -> Result<()> {
        self.rt()?.stop_playback();
        Ok(())
    }

    /// Whether the transport is currently playing.
    #[napi]
    pub fn is_playing(&self) -> Result<bool> {
        Ok(self.rt()?.is_playing())
    }

    /// Set tempo in BPM.
    #[napi]
    pub fn set_tempo(&self, bpm: f64) -> Result<()> {
        self.rt()?.set_tempo(bpm);
        Ok(())
    }

    /// Enable or disable loop playback.
    #[napi]
    pub fn set_loop(&self, enabled: bool) -> Result<()> {
        self.rt()?.set_loop(enabled);
        Ok(())
    }

    /// Load a MIDI file. Sequencer takes effect on the next audio callback.
    /// Transport state is unchanged — call `play()` to start playback.
    #[napi]
    pub fn load_midi(&mut self, path: String) -> Result<()> {
        self.rt_mut()?
            .load_midi(&path)
            .map_err(napi::Error::from_reason)
    }

    /// Remove any loaded MIDI sequence.
    #[napi]
    pub fn unload_midi(&mut self) -> Result<()> {
        self.rt_mut()?.unload_midi();
        Ok(())
    }

    // --- MIDI events ---

    /// Send MIDI note on.
    #[napi]
    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) -> Result<()> {
        self.rt_mut()?.note_on(channel, note, velocity);
        Ok(())
    }

    /// Send MIDI note off.
    #[napi]
    pub fn note_off(&mut self, channel: u8, note: u8) -> Result<()> {
        self.rt_mut()?.note_off(channel, note);
        Ok(())
    }

    /// Send MIDI CC (control change).
    #[napi]
    pub fn cc(&mut self, channel: u8, cc: u8, value: u8) -> Result<()> {
        self.rt_mut()?.cc(channel, cc, value);
        Ok(())
    }

    /// Send MIDI pitch bend.
    #[napi]
    pub fn pitch_bend(&mut self, channel: u8, value: i16) -> Result<()> {
        self.rt_mut()?.pitch_bend(channel, value);
        Ok(())
    }

    /// Send MIDI program change.
    #[napi]
    pub fn program_change(&mut self, channel: u8, program: u8) -> Result<()> {
        self.rt_mut()?.program_change(channel, program);
        Ok(())
    }

    /// Silence all active notes.
    #[napi]
    pub fn all_notes_off(&mut self) -> Result<()> {
        self.rt_mut()?.all_notes_off();
        Ok(())
    }

    /// Set master volume via MIDI event (0.0 to 1.0).
    #[napi]
    pub fn set_volume(&mut self, volume: f64) -> Result<()> {
        self.rt_mut()?.set_volume(volume as f32);
        Ok(())
    }

    /// Set a backend parameter by ID.
    #[napi]
    pub fn set_param(&mut self, id: u32, value: f64) -> Result<()> {
        self.rt_mut()?.set_param(id, value);
        Ok(())
    }

    // --- Mixer: tracks ---

    /// Add an instrument track. Returns the track ID.
    ///
    /// `channel_mask` is a 16-bit bitmask selecting MIDI channels (0xFFFF = all).
    #[napi]
    pub fn add_track(&mut self, backend: &mut Backend, channel_mask: u32) -> Result<u32> {
        let b = backend
            .inner
            .take()
            .ok_or_else(|| napi::Error::from_reason("Backend already consumed"))?;
        Ok(self.rt_mut()?.add_track(b, channel_mask as u16))
    }

    /// Remove a track by ID.
    #[napi]
    pub fn remove_track(&mut self, track_id: u32) -> Result<()> {
        self.rt_mut()?.remove_track(track_id);
        Ok(())
    }

    /// Replace a track's instrument while keeping the channel strip
    /// (volume, pan, sends, inserts, meter) intact.
    #[napi]
    pub fn swap_track_backend(&mut self, track_id: u32, backend: &mut Backend) -> Result<()> {
        let b = backend
            .inner
            .take()
            .ok_or_else(|| napi::Error::from_reason("Backend already consumed"))?;
        self.rt_mut()?.swap_track_backend(track_id, b);
        Ok(())
    }

    /// Update which MIDI channels reach this track. Bit N set ⇒ channel N
    /// dispatches to this track. Used by the MIDI-import flow to give each
    /// MIDI channel its own DAW track.
    #[napi]
    pub fn set_track_channel_mask(&mut self, track_id: u32, channel_mask: u32) -> Result<()> {
        self.rt_mut()?
            .set_track_channel_mask(track_id, channel_mask as u16);
        Ok(())
    }

    /// Add an insert effect to a track. Returns the insert ID.
    #[napi]
    pub fn add_insert(&mut self, track_id: u32, effect: &mut Backend) -> Result<u32> {
        let b = effect
            .inner
            .take()
            .ok_or_else(|| napi::Error::from_reason("Backend already consumed"))?;
        Ok(self.rt_mut()?.add_insert(track_id, b))
    }

    /// Remove an insert effect from a track.
    #[napi]
    pub fn remove_insert(&mut self, track_id: u32, insert_id: u32) -> Result<()> {
        self.rt_mut()?.remove_insert(track_id, insert_id);
        Ok(())
    }

    /// Add a send bus with an effect. Returns the bus ID.
    #[napi]
    pub fn add_send_bus(&mut self, effect: &mut Backend) -> Result<u32> {
        let b = effect
            .inner
            .take()
            .ok_or_else(|| napi::Error::from_reason("Backend already consumed"))?;
        Ok(self.rt_mut()?.add_send_bus(b))
    }

    // --- Mixer: levels ---

    /// Set track volume (0.0 to 1.0).
    #[napi]
    pub fn set_track_volume(&mut self, track_id: u8, volume: f64) -> Result<()> {
        self.rt_mut()?.mixer_set_track_volume(track_id, volume as f32);
        Ok(())
    }

    /// Set track pan (-1.0 left, 0.0 center, 1.0 right).
    #[napi]
    pub fn set_track_pan(&mut self, track_id: u8, pan: f64) -> Result<()> {
        self.rt_mut()?.mixer_set_track_pan(track_id, pan as f32);
        Ok(())
    }

    /// Set track trim in dB.
    #[napi]
    pub fn set_track_trim(&mut self, track_id: u8, trim_db: f64) -> Result<()> {
        self.rt_mut()?.mixer_set_track_trim(track_id, trim_db as f32);
        Ok(())
    }

    /// Mute or unmute a track.
    #[napi]
    pub fn set_track_mute(&mut self, track_id: u8, mute: bool) -> Result<()> {
        self.rt_mut()?.mixer_set_track_mute(track_id, mute);
        Ok(())
    }

    /// Solo or unsolo a track.
    #[napi]
    pub fn set_track_solo(&mut self, track_id: u8, solo: bool) -> Result<()> {
        self.rt_mut()?.mixer_set_track_solo(track_id, solo);
        Ok(())
    }

    /// Set track send level to a bus.
    #[napi]
    pub fn set_track_send(&mut self, track_id: u8, bus_id: u8, level: f64) -> Result<()> {
        self.rt_mut()?.mixer_set_track_send(track_id, bus_id, level as f32);
        Ok(())
    }

    /// Set master bus volume.
    #[napi]
    pub fn set_master_volume(&mut self, volume: f64) -> Result<()> {
        self.rt_mut()?.mixer_set_master_volume(volume as f32);
        Ok(())
    }

    /// Bypass or enable an insert effect.
    #[napi]
    pub fn set_insert_bypass(
        &mut self,
        track_id: u8,
        insert_id: u8,
        bypass: bool,
    ) -> Result<()> {
        self.rt_mut()?.mixer_set_insert_bypass(track_id, insert_id, bypass);
        Ok(())
    }

    /// Route a track's output. `target_id` 0xFF = master, else group track ID.
    #[napi]
    pub fn set_track_route(&mut self, track_id: u8, target_id: u8) -> Result<()> {
        self.rt_mut()?.mixer_set_track_route(track_id, target_id);
        Ok(())
    }

    /// Set external sidechain source for an insert effect.
    /// `source_track_id` = None reverts to internal sidechain.
    #[napi]
    pub fn set_insert_sidechain(
        &mut self,
        track_id: u8,
        insert_id: u8,
        source_track_id: Option<u8>,
    ) -> Result<()> {
        self.rt_mut()?.set_insert_sidechain(track_id, insert_id, source_track_id);
        Ok(())
    }

    /// Set a parameter on a track's backend.
    #[napi]
    pub fn set_param_for_track(
        &mut self,
        track_id: u8,
        param_id: u16,
        value: f64,
    ) -> Result<()> {
        self.rt_mut()?.set_param_for_track(track_id, param_id, value);
        Ok(())
    }

    /// Set a parameter on a track's insert effect.
    #[napi]
    pub fn set_insert_param(
        &mut self,
        track_id: u8,
        insert_id: u8,
        param_id: u16,
        value: f64,
    ) -> Result<()> {
        self.rt_mut()?.set_insert_param(track_id, insert_id, param_id, value);
        Ok(())
    }

    /// Set a parameter on a send bus effect.
    #[napi]
    pub fn set_send_bus_param(
        &mut self,
        bus_id: u8,
        param_id: u16,
        value: f64,
    ) -> Result<()> {
        self.rt_mut()?.set_send_bus_param(bus_id, param_id, value);
        Ok(())
    }

    // --- Metering ---

    /// Read peak levels for a track (linear scale, 0.0 to 1.0+).
    /// Returns { peakL, peakR }. Returns zeros if track ID is unknown.
    #[napi]
    pub fn track_levels(&self, track_id: u32) -> Result<TrackLevels> {
        let (l, r) = self.rt()?.track_levels(track_id);
        Ok(TrackLevels {
            peak_l: l as f64,
            peak_r: r as f64,
        })
    }

    /// Read peak levels for the master bus (linear scale, 0.0 to 1.0+).
    #[napi]
    pub fn master_levels(&self) -> Result<TrackLevels> {
        let (l, r) = self.rt()?.master_levels();
        Ok(TrackLevels {
            peak_l: l as f64,
            peak_r: r as f64,
        })
    }

    /// Number of tracks in the mixer.
    #[napi]
    pub fn track_count(&self) -> Result<u32> {
        Ok(self.rt()?.track_count())
    }

    // --- MIDI devices ---

    /// List available MIDI input devices.
    #[napi]
    pub fn list_midi_inputs() -> Result<Vec<MidiDevice>> {
        moonlitt_audio_io::Runtime::list_midi_inputs()
            .map(|devices| {
                devices
                    .into_iter()
                    .map(|d| MidiDevice {
                        id: d.id as u32,
                        name: d.name,
                    })
                    .collect()
            })
            .map_err(napi::Error::from_reason)
    }

    /// Number of events dropped due to ring buffer overflow.
    #[napi]
    pub fn dropped_events(&self) -> Result<u32> {
        Ok(self.rt()?.dropped_events() as u32)
    }

    /// Shutdown the session and release audio resources.
    #[napi]
    pub fn shutdown(&mut self) -> Result<()> {
        let rt = self
            .runtime
            .take()
            .ok_or_else(|| napi::Error::from_reason("Session not initialized"))?;
        rt.shutdown();
        Ok(())
    }
}

impl Session {
    fn rt(&self) -> Result<&moonlitt_audio_io::Runtime> {
        self.runtime
            .as_ref()
            .ok_or_else(|| napi::Error::from_reason("Session not initialized"))
    }

    fn rt_mut(&mut self) -> Result<&mut moonlitt_audio_io::Runtime> {
        self.runtime
            .as_mut()
            .ok_or_else(|| napi::Error::from_reason("Session not initialized"))
    }
}
