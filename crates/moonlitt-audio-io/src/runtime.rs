use crate::audio_output::AudioOutput;
use crate::midi_input::{MidiDeviceInfo, MidiInputConnection};
use moonlitt_core::{AudioBackend, AudioEvent, TimedEvent};
use moonlitt_mixer::{LevelMeter, Mixer};
use moonlitt_session::{AudioThread, MixerCommand, Sequencer, SequencerCommand, Transport};
use rtrb::RingBuffer;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

/// Length/resolution facts about a staged MIDI clip, captured at load
/// time for progress displays.
#[derive(Debug, Clone, Copy)]
pub struct MidiClipInfo {
    pub total_ticks: u64,
    pub ticks_per_beat: u16,
}

pub struct Runtime {
    producer: rtrb::Producer<TimedEvent>,
    /// Channel for structural commands (add/remove tracks, inserts, buses).
    command_tx: mpsc::Sender<MixerCommand>,
    /// Channel for sequencer load/unload.
    sequencer_tx: mpsc::Sender<SequencerCommand>,
    audio_output: Option<AudioOutput>,
    #[allow(dead_code)]
    midi_connection: Option<MidiInputConnection>,
    transport: Arc<Transport>,
    #[allow(dead_code)]
    buffer_size: u32,
    #[allow(dead_code)]
    sample_rate: u32,
    /// Counter for events dropped due to ring buffer overflow.
    dropped_events: Arc<AtomicU64>,
    /// Control-side mirror of the practice-loop region (ticks) — the
    /// authoritative copy lives in the audio-thread sequencer.
    loop_region: Option<(f64, f64)>,
    /// Pre-assigned ID counters (synchronized with Mixer on audio thread).
    next_track_id: u32,
    next_bus_id: u32,
    next_insert_id: u32,
    /// Cloned meter handles for cross-thread reading.
    /// The audio thread writes via the mixer's meters (same `Arc<AtomicU32>`),
    /// the main thread reads via these clones.
    master_meter: LevelMeter,
    track_meters: HashMap<u32, LevelMeter>,
    /// Lock-free metronome toggle — flipped from the control thread,
    /// read every audio chunk on the audio thread.
    metronome_enabled: Arc<AtomicBool>,
    /// Whether the audio output stream is currently running
    /// (set by `start`/`stop`).
    audio_running: AtomicBool,
}

impl Runtime {
    /// Create a runtime with a single backend (backward compatible).
    /// The backend is placed in a Mixer as the sole track handling all 16 channels.
    ///
    /// Audio device and config compatibility are checked BEFORE consuming the
    /// backend. On failure the backend is returned via the error tuple so the
    /// caller can retry or continue using it for offline rendering.
    pub fn new(
        backend: Box<dyn AudioBackend>,
        sample_rate: u32,
        buffer_size: u32,
    ) -> Result<Self, (String, Box<dyn AudioBackend>)> {
        // Pre-check audio device AND config compatibility BEFORE consuming
        // the backend into a Mixer. This ensures the backend is not lost on
        // predictable failures (no device, incompatible config).
        if let Err(e) = AudioOutput::pre_check(sample_rate) {
            return Err((e, backend));
        }

        // Device + config validated — safe to consume backend into mixer.
        let mut mixer = Mixer::new(sample_rate, buffer_size as usize);
        mixer.add_track(backend, 0xFFFF); // all 16 channels

        Self::with_mixer(mixer, buffer_size).map_err(|e| {
            (
                e,
                Box::new(moonlitt_core::NullBackend::new(sample_rate)) as Box<dyn AudioBackend>,
            )
        })
    }

    /// Create a runtime with a pre-configured Mixer and a fresh Transport.
    /// Convenience over `with_mixer_and_transport`.
    pub fn with_mixer(mixer: Mixer, buffer_size: u32) -> Result<Self, String> {
        Self::with_mixer_and_transport(mixer, Transport::new(), buffer_size)
    }

    /// Create a runtime with a pre-configured Mixer AND a pre-configured
    /// Transport. Used by session-restore to preserve captured tempo /
    /// loop state across reload.
    pub fn with_mixer_and_transport(
        mixer: Mixer,
        transport: Transport,
        buffer_size: u32,
    ) -> Result<Self, String> {
        let sample_rate = mixer.sample_rate();
        let next_track_id = mixer.next_track_id();
        let next_bus_id = mixer.next_bus_id();
        let next_insert_id = mixer.next_insert_id();

        // Clone meter handles BEFORE moving the mixer to the audio thread.
        // LevelMeter uses Arc<AtomicU32> internally, so clones share the same
        // atomic storage — the audio thread writes, the main thread reads.
        let master_meter = mixer.clone_master_meter();
        let track_meters: HashMap<u32, LevelMeter> =
            mixer.clone_track_meters().into_iter().collect();

        // Ring buffer capacity: 1024 events. Sufficient for real-time MIDI at
        // typical rates; events are drained every audio callback (~5ms).
        let (producer, consumer) = RingBuffer::new(1024);
        let (command_tx, command_rx) = mpsc::channel();
        let (sequencer_tx, sequencer_rx) = mpsc::channel();
        let transport = Arc::new(transport);

        let audio_thread = AudioThread::new(
            mixer,
            consumer,
            command_rx,
            sequencer_rx,
            None,
            transport.clone(),
            buffer_size as usize,
        );

        // Pull the metronome's enabled flag out before audio_thread moves
        // into the cpal callback — after that it's unreachable.
        let metronome_enabled = audio_thread.metronome_enabled_handle();

        let audio_output = AudioOutput::new(audio_thread)?;

        Ok(Self {
            producer,
            command_tx,
            sequencer_tx,
            audio_output: Some(audio_output),
            midi_connection: None,
            transport,
            buffer_size,
            sample_rate,
            dropped_events: Arc::new(AtomicU64::new(0)),
            loop_region: None,
            next_track_id,
            next_bus_id,
            next_insert_id,
            master_meter,
            track_meters,
            metronome_enabled,
            audio_running: AtomicBool::new(false),
        })
    }

    pub fn set_metronome_enabled(&self, enabled: bool) {
        self.metronome_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn is_metronome_enabled(&self) -> bool {
        self.metronome_enabled.load(Ordering::Relaxed)
    }

    pub fn start(&self) -> Result<(), String> {
        if let Some(ref output) = self.audio_output {
            output.start()?;
            self.audio_running.store(true, Ordering::Relaxed);
            Ok(())
        } else {
            Err("no audio output".into())
        }
    }

    pub fn stop(&self) -> Result<(), String> {
        if let Some(ref output) = self.audio_output {
            output.pause()?;
            self.audio_running.store(false, Ordering::Relaxed);
            Ok(())
        } else {
            Err("no audio output".into())
        }
    }

    /// Whether the audio output stream is currently running.
    pub fn is_audio_running(&self) -> bool {
        self.audio_running.load(Ordering::Relaxed)
    }

    /// Master-bus sample peak (L, R) — atomic read of the audio thread's
    /// most recent render block.
    pub fn master_peak(&self) -> (f32, f32) {
        self.master_meter.peak()
    }

    /// Master-bus RMS (L, R) — atomic read of the audio thread's most
    /// recent render block.
    pub fn master_rms(&self) -> (f32, f32) {
        self.master_meter.rms()
    }

    // --- MIDI events (lock-free SPSC — single caller only) ---

    /// Push an event to the audio thread. Returns `false` (and bumps
    /// `dropped_events`) when the ring buffer is full — the event is
    /// dropped, never blocked on.
    fn send(&mut self, event: AudioEvent) -> bool {
        if self
            .producer
            .push(TimedEvent {
                event,
                delay_samples: 0,
            })
            .is_err()
        {
            self.dropped_events.fetch_add(1, Ordering::Relaxed);
            false
        } else {
            true
        }
    }

    fn send_delayed(&mut self, event: AudioEvent, delay_samples: u32) -> bool {
        if self
            .producer
            .push(TimedEvent {
                event,
                delay_samples,
            })
            .is_err()
        {
            self.dropped_events.fetch_add(1, Ordering::Relaxed);
            false
        } else {
            true
        }
    }

    /// Number of events silently dropped due to ring buffer overflow.
    #[must_use]
    pub fn dropped_events(&self) -> u64 {
        self.dropped_events.load(Ordering::Relaxed)
    }

    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) -> bool {
        self.send(AudioEvent::NoteOn {
            channel,
            note,
            velocity,
        })
    }

    pub fn note_on_delayed(
        &mut self,
        channel: u8,
        note: u8,
        velocity: u8,
        delay_samples: u32,
    ) -> bool {
        self.send_delayed(
            AudioEvent::NoteOn {
                channel,
                note,
                velocity,
            },
            delay_samples,
        )
    }

    pub fn note_off(&mut self, channel: u8, note: u8) -> bool {
        self.send(AudioEvent::NoteOff {
            channel,
            note,
            velocity: 0,
        })
    }

    pub fn note_off_delayed(&mut self, channel: u8, note: u8, delay_samples: u32) -> bool {
        self.send_delayed(
            AudioEvent::NoteOff {
                channel,
                note,
                velocity: 0,
            },
            delay_samples,
        )
    }

    pub fn cc(&mut self, channel: u8, cc: u8, value: u8) -> bool {
        self.send(AudioEvent::CC { channel, cc, value })
    }

    pub fn pitch_bend(&mut self, channel: u8, value: i16) -> bool {
        self.send(AudioEvent::PitchBend { channel, value })
    }

    pub fn program_change(&mut self, channel: u8, program: u8) -> bool {
        self.send(AudioEvent::ProgramChange { channel, program })
    }

    pub fn all_notes_off(&mut self) -> bool {
        self.send(AudioEvent::AllNotesOff)
    }

    pub fn set_volume(&mut self, volume: f32) -> bool {
        self.send(AudioEvent::SetVolume(volume))
    }

    pub fn set_param(&mut self, id: u32, value: f64) -> bool {
        self.send(AudioEvent::SetParam { id, value })
    }

    // --- Mixer control (lock-free SPSC — single caller only) ---

    pub fn mixer_set_track_volume(&mut self, track_id: u8, volume: f32) -> bool {
        self.send(AudioEvent::MixerTrackVolume { track_id, volume })
    }

    pub fn mixer_set_track_pan(&mut self, track_id: u8, pan: f32) -> bool {
        self.send(AudioEvent::MixerTrackPan { track_id, pan })
    }

    pub fn mixer_set_track_trim(&mut self, track_id: u8, trim_db: f32) -> bool {
        self.send(AudioEvent::MixerTrackTrim { track_id, trim_db })
    }

    pub fn mixer_set_track_mute(&mut self, track_id: u8, mute: bool) -> bool {
        self.send(AudioEvent::MixerTrackMute { track_id, mute })
    }

    pub fn mixer_set_track_solo(&mut self, track_id: u8, solo: bool) -> bool {
        self.send(AudioEvent::MixerTrackSolo { track_id, solo })
    }

    pub fn mixer_set_track_send(&mut self, track_id: u8, bus_id: u8, level: f32) -> bool {
        self.send(AudioEvent::MixerTrackSend {
            track_id,
            bus_id,
            level,
        })
    }

    pub fn mixer_set_master_volume(&mut self, volume: f32) -> bool {
        self.send(AudioEvent::MixerMasterVolume(volume))
    }

    pub fn mixer_set_insert_bypass(&mut self, track_id: u8, insert_id: u8, bypass: bool) -> bool {
        self.send(AudioEvent::InsertBypass {
            track_id,
            insert_id,
            bypass,
        })
    }

    pub fn set_param_for_track(&mut self, track_id: u8, param_id: u16, value: f64) -> bool {
        self.send(AudioEvent::SetParamForTrack {
            track_id,
            param_id,
            value,
        })
    }

    pub fn set_insert_param(
        &mut self,
        track_id: u8,
        insert_id: u8,
        param_id: u16,
        value: f64,
    ) -> bool {
        self.send(AudioEvent::SetInsertParam {
            track_id,
            insert_id,
            param_id,
            value,
        })
    }

    pub fn set_send_bus_param(&mut self, bus_id: u8, param_id: u16, value: f64) -> bool {
        self.send(AudioEvent::SetSendBusParam {
            bus_id,
            param_id,
            value,
        })
    }

    /// Route a track's output. target_id = 0xFF for master, else group track ID.
    pub fn mixer_set_track_route(&mut self, track_id: u8, target_id: u8) -> bool {
        self.send(AudioEvent::MixerTrackRoute {
            track_id,
            target_id,
        })
    }

    /// Set external sidechain source for an insert effect.
    /// source_track_id = None means revert to internal sidechain.
    pub fn set_insert_sidechain(
        &mut self,
        track_id: u8,
        insert_id: u8,
        source_track_id: Option<u8>,
    ) -> bool {
        let src = source_track_id.unwrap_or(0xFF);
        self.send(AudioEvent::SetInsertSidechain {
            track_id,
            insert_id,
            source_track_id: src,
        })
    }

    // --- Structural commands (via mpsc command channel) ---
    // These carry heap-allocated data (Engine) and run on the audio thread.

    /// Add a track at runtime. Returns the pre-assigned track ID.
    pub fn add_track(&mut self, backend: Box<dyn AudioBackend>, channel_mask: u16) -> u32 {
        let id = self.next_track_id;
        self.next_track_id += 1;

        // Create meter on the main thread, clone it for the audio thread.
        let meter = LevelMeter::new();
        let meter_clone = meter.clone();
        self.track_meters.insert(id, meter);

        let _ = self.command_tx.send(Box::new(move |mixer| {
            mixer.add_track_with_meter(id, backend, channel_mask, meter_clone);
        }));
        id
    }

    /// Hot-swap a track's backend at runtime. Inserts, volume, pan, sends
    /// and meter are preserved; only the instrument changes. Active notes on
    /// the old backend are silenced.
    pub fn swap_track_backend(&mut self, track_id: u32, backend: Box<dyn AudioBackend>) {
        let _ = self.command_tx.send(Box::new(move |mixer| {
            mixer.replace_track_backend(track_id, backend, None);
        }));
    }

    /// Update which MIDI channels a track listens to (bit N = channel N).
    pub fn set_track_channel_mask(&mut self, track_id: u32, channel_mask: u16) {
        let _ = self.command_tx.send(Box::new(move |mixer| {
            mixer.set_track_channel_mask(track_id, channel_mask);
        }));
    }

    /// Remove a track at runtime. Notes are silenced before removal.
    pub fn remove_track(&mut self, track_id: u32) {
        self.track_meters.remove(&track_id);
        let _ = self.command_tx.send(Box::new(move |mixer| {
            if let Some(track) = mixer.track_mut(track_id) {
                track.backend.all_notes_off();
            }
            mixer.remove_track(track_id);
        }));
    }

    /// Add an insert effect to a track at runtime. Returns the pre-assigned insert ID.
    pub fn add_insert(&mut self, track_id: u32, backend: Box<dyn AudioBackend>) -> u32 {
        let id = self.next_insert_id;
        self.next_insert_id += 1;
        let _ = self.command_tx.send(Box::new(move |mixer| {
            mixer.add_insert_with_id(track_id, id, backend, None);
        }));
        id
    }

    /// Remove an insert effect from a track at runtime.
    pub fn remove_insert(&mut self, track_id: u32, insert_id: u32) {
        let _ = self.command_tx.send(Box::new(move |mixer| {
            mixer.remove_insert(track_id, insert_id);
        }));
    }

    /// Add a send bus at runtime. Returns the pre-assigned bus ID.
    pub fn add_send_bus(&mut self, backend: Box<dyn AudioBackend>) -> u32 {
        let id = self.next_bus_id;
        self.next_bus_id += 1;
        let _ = self.command_tx.send(Box::new(move |mixer| {
            mixer.add_send_bus_with_id(id, backend, None);
        }));
        id
    }

    // --- Metering (lock-free atomic reads) ---

    /// Read peak levels for a track. Returns (peak_l, peak_r) in linear scale.
    /// Returns (0.0, 0.0) if the track ID is unknown.
    pub fn track_levels(&self, track_id: u32) -> (f32, f32) {
        self.track_meters
            .get(&track_id)
            .map(|m| m.peak())
            .unwrap_or((0.0, 0.0))
    }

    /// Read peak levels for the master bus. Returns (peak_l, peak_r) in linear scale.
    pub fn master_levels(&self) -> (f32, f32) {
        self.master_meter.peak()
    }

    /// Number of tracks whose meters are available.
    pub fn track_count(&self) -> u32 {
        self.track_meters.len() as u32
    }

    // --- Transport ---

    pub fn play(&self) {
        self.transport.play();
    }

    pub fn pause_playback(&self) {
        self.transport.pause();
    }

    /// Stop sequencer playback, silence held notes, and rewind to the
    /// start (DAW "stop" semantics — a following play replays from 0).
    pub fn stop_playback(&mut self) {
        self.transport.stop();
        let _ = self.all_notes_off();
        let _ = self.sequencer_tx.send(Box::new(|slot| {
            if let Some(seq) = slot.as_mut() {
                seq.seek(0.0);
            }
        }));
    }

    pub fn is_playing(&self) -> bool {
        self.transport.is_playing()
    }

    pub fn set_tempo(&self, bpm: f64) {
        self.transport.set_tempo(bpm);
    }

    /// Revert to the MIDI file's embedded tempo map.
    pub fn clear_tempo_override(&self) {
        self.transport.clear_tempo();
    }

    /// The current tempo override, or `None` when following the file.
    pub fn tempo_override(&self) -> Option<f64> {
        self.transport.tempo()
    }

    pub fn set_loop(&self, enabled: bool) {
        self.transport.set_loop(enabled);
    }

    /// Set (or clear) the practice-loop region in ticks. Applies on the
    /// audio thread via the sequencer command channel; the sequencer
    /// sanitises the bounds. A control-side mirror is kept for session
    /// capture and UI reads.
    pub fn set_loop_region(&mut self, region: Option<(f64, f64)>) {
        self.loop_region = region;
        let _ = self.sequencer_tx.send(Box::new(move |slot| {
            if let Some(seq) = slot.as_mut() {
                seq.set_loop_region(region);
            }
        }));
    }

    /// Control-side mirror of the practice-loop region.
    pub fn loop_region(&self) -> Option<(f64, f64)> {
        self.loop_region
    }

    /// Jump the sequencer playhead to an absolute tick (clamped to the
    /// clip). Held notes are released first so nothing hangs across the
    /// jump; playback state is unchanged.
    pub fn seek_ticks(&mut self, tick: f64) {
        let _ = self.all_notes_off();
        let _ = self.sequencer_tx.send(Box::new(move |slot| {
            if let Some(seq) = slot.as_mut() {
                seq.seek(tick);
            }
        }));
    }

    /// Latest sequencer playhead position in fractional ticks, as
    /// published by the audio thread (atomic read — poll freely).
    pub fn position_ticks(&self) -> f64 {
        self.transport.position_ticks()
    }

    // --- MIDI file loading ---

    /// Parse a MIDI file and stage it on the audio thread for playback.
    /// The new sequencer takes effect on the next audio callback. Transport
    /// state (play/pause/stop) is unchanged — the caller decides when to start.
    pub fn load_midi(&mut self, path: &str) -> Result<MidiClipInfo, String> {
        let mut seq = Sequencer::from_file(path)?;
        let info = MidiClipInfo {
            total_ticks: seq.total_ticks(),
            ticks_per_beat: seq.ticks_per_beat(),
        };
        // The sequencer's own state gates `advance()`; AudioThread additionally
        // gates on Transport. We open the inner gate here so transport alone
        // controls playback once the sequencer is staged.
        seq.play();
        self.sequencer_tx
            .send(Box::new(move |slot| {
                *slot = Some(seq);
            }))
            .map_err(|e| format!("audio thread closed: {e}"))?;
        Ok(info)
    }

    /// Remove any loaded MIDI sequence from the audio thread.
    pub fn unload_midi(&mut self) {
        let _ = self.sequencer_tx.send(Box::new(|slot| {
            *slot = None;
        }));
    }

    // --- MIDI Input ---

    pub fn list_midi_inputs() -> Result<Vec<MidiDeviceInfo>, String> {
        MidiInputConnection::list_devices()
    }

    // --- Shutdown ---

    pub fn shutdown(mut self) {
        let _ = self.producer.push(TimedEvent {
            event: AudioEvent::Stop,
            delay_samples: 0,
        });
        drop(self.audio_output.take());
    }
}
