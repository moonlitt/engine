use crate::audio_output::AudioOutput;
use crate::midi_input::{MidiDeviceInfo, MidiInputConnection};
use moonlitt_core::{AudioBackend, AudioEvent, TimedEvent};
use moonlitt_mixer::Mixer;
use moonlitt_session::{AudioThread, MixerCommand, Transport};
use rtrb::RingBuffer;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

pub struct Runtime {
    producer: rtrb::Producer<TimedEvent>,
    /// Channel for structural commands (add/remove tracks, inserts, buses).
    command_tx: mpsc::Sender<MixerCommand>,
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
    /// Pre-assigned ID counters (synchronized with Mixer on audio thread).
    next_track_id: u32,
    next_bus_id: u32,
    next_insert_id: u32,
}

impl Runtime {
    /// Create a runtime with a single backend (backward compatible).
    /// The backend is placed in a Mixer as the sole track handling all 16 channels.
    ///
    /// Audio device and config compatibility are checked BEFORE consuming the
    /// backend. On failure the backend is returned via the error tuple so the
    /// caller can retry or continue using it for offline rendering.
    pub fn new(backend: Box<dyn AudioBackend>, sample_rate: u32, buffer_size: u32) -> Result<Self, (String, Box<dyn AudioBackend>)> {
        // Pre-check audio device AND config compatibility BEFORE consuming
        // the backend into a Mixer. This ensures the backend is not lost on
        // predictable failures (no device, incompatible config).
        if let Err(e) = AudioOutput::pre_check(sample_rate) {
            return Err((e, backend));
        }

        // Device + config validated — safe to consume backend into mixer.
        let mut mixer = Mixer::new(sample_rate, buffer_size as usize);
        mixer.add_track(backend, 0xFFFF); // all 16 channels

        Self::with_mixer(mixer, buffer_size)
            .map_err(|e| (e, Box::new(moonlitt_core::NullBackend::new(sample_rate)) as Box<dyn AudioBackend>))
    }

    /// Create a runtime with a pre-configured Mixer.
    pub fn with_mixer(mixer: Mixer, buffer_size: u32) -> Result<Self, String> {
        let sample_rate = mixer.sample_rate();
        let next_track_id = mixer.next_track_id();
        let next_bus_id = mixer.next_bus_id();
        let next_insert_id = mixer.next_insert_id();

        // Ring buffer capacity: 1024 events. Sufficient for real-time MIDI at
        // typical rates; events are drained every audio callback (~5ms).
        let (producer, consumer) = RingBuffer::new(1024);
        let (command_tx, command_rx) = mpsc::channel();
        let transport = Arc::new(Transport::new());

        let audio_thread = AudioThread::new(
            mixer,
            consumer,
            command_rx,
            None,
            transport.clone(),
            buffer_size as usize,
        );

        let audio_output = AudioOutput::new(audio_thread)?;

        Ok(Self {
            producer,
            command_tx,
            audio_output: Some(audio_output),
            midi_connection: None,
            transport,
            buffer_size,
            sample_rate,
            dropped_events: Arc::new(AtomicU64::new(0)),
            next_track_id,
            next_bus_id,
            next_insert_id,
        })
    }

    pub fn start(&self) -> Result<(), String> {
        if let Some(ref output) = self.audio_output {
            output.start()
        } else {
            Err("no audio output".into())
        }
    }

    pub fn stop(&self) -> Result<(), String> {
        if let Some(ref output) = self.audio_output {
            output.pause()
        } else {
            Err("no audio output".into())
        }
    }

    // --- MIDI events (lock-free SPSC — single caller only) ---

    fn send(&mut self, event: AudioEvent) {
        if self.producer.push(TimedEvent {
            event,
            delay_samples: 0,
        }).is_err() {
            self.dropped_events.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn send_delayed(&mut self, event: AudioEvent, delay_samples: u32) {
        if self.producer.push(TimedEvent {
            event,
            delay_samples,
        }).is_err() {
            self.dropped_events.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Number of events silently dropped due to ring buffer overflow.
    #[must_use]
    pub fn dropped_events(&self) -> u64 {
        self.dropped_events.load(Ordering::Relaxed)
    }

    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        self.send(AudioEvent::NoteOn {
            channel,
            note,
            velocity,
        });
    }

    pub fn note_on_delayed(&mut self, channel: u8, note: u8, velocity: u8, delay_samples: u32) {
        self.send_delayed(
            AudioEvent::NoteOn {
                channel,
                note,
                velocity,
            },
            delay_samples,
        );
    }

    pub fn note_off(&mut self, channel: u8, note: u8) {
        self.send(AudioEvent::NoteOff {
            channel,
            note,
            velocity: 0,
        });
    }

    pub fn note_off_delayed(&mut self, channel: u8, note: u8, delay_samples: u32) {
        self.send_delayed(
            AudioEvent::NoteOff {
                channel,
                note,
                velocity: 0,
            },
            delay_samples,
        );
    }

    pub fn cc(&mut self, channel: u8, cc: u8, value: u8) {
        self.send(AudioEvent::CC { channel, cc, value });
    }

    pub fn pitch_bend(&mut self, channel: u8, value: i16) {
        self.send(AudioEvent::PitchBend { channel, value });
    }

    pub fn program_change(&mut self, channel: u8, program: u8) {
        self.send(AudioEvent::ProgramChange { channel, program });
    }

    pub fn all_notes_off(&mut self) {
        self.send(AudioEvent::AllNotesOff);
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.send(AudioEvent::SetVolume(volume));
    }

    pub fn set_param(&mut self, id: u32, value: f32) {
        self.send(AudioEvent::SetParam { id, value });
    }

    // --- Mixer control (lock-free SPSC — single caller only) ---

    pub fn mixer_set_track_volume(&mut self, track_id: u8, volume: f32) {
        self.send(AudioEvent::MixerTrackVolume { track_id, volume });
    }

    pub fn mixer_set_track_pan(&mut self, track_id: u8, pan: f32) {
        self.send(AudioEvent::MixerTrackPan { track_id, pan });
    }

    pub fn mixer_set_track_trim(&mut self, track_id: u8, trim_db: f32) {
        self.send(AudioEvent::MixerTrackTrim { track_id, trim_db });
    }

    pub fn mixer_set_track_mute(&mut self, track_id: u8, mute: bool) {
        self.send(AudioEvent::MixerTrackMute { track_id, mute });
    }

    pub fn mixer_set_track_solo(&mut self, track_id: u8, solo: bool) {
        self.send(AudioEvent::MixerTrackSolo { track_id, solo });
    }

    pub fn mixer_set_track_send(&mut self, track_id: u8, bus_id: u8, level: f32) {
        self.send(AudioEvent::MixerTrackSend { track_id, bus_id, level });
    }

    pub fn mixer_set_master_volume(&mut self, volume: f32) {
        self.send(AudioEvent::MixerMasterVolume(volume));
    }

    pub fn mixer_set_insert_bypass(&mut self, track_id: u8, insert_id: u8, bypass: bool) {
        self.send(AudioEvent::InsertBypass { track_id, insert_id, bypass });
    }

    pub fn set_param_for_track(&mut self, track_id: u8, param_id: u16, value: f32) {
        self.send(AudioEvent::SetParamForTrack { track_id, param_id, value });
    }

    pub fn set_insert_param(&mut self, track_id: u8, insert_id: u8, param_id: u16, value: f32) {
        self.send(AudioEvent::SetInsertParam { track_id, insert_id, param_id, value });
    }

    pub fn set_send_bus_param(&mut self, bus_id: u8, param_id: u16, value: f32) {
        self.send(AudioEvent::SetSendBusParam { bus_id, param_id, value });
    }

    /// Route a track's output. target_id = 0xFF for master, else group track ID.
    pub fn mixer_set_track_route(&mut self, track_id: u8, target_id: u8) {
        self.send(AudioEvent::MixerTrackRoute { track_id, target_id });
    }

    /// Set external sidechain source for an insert effect.
    /// source_track_id = None means revert to internal sidechain.
    pub fn set_insert_sidechain(&mut self, track_id: u8, insert_id: u8, source_track_id: Option<u8>) {
        let src = source_track_id.unwrap_or(0xFF);
        self.send(AudioEvent::SetInsertSidechain {
            track_id,
            insert_id,
            source_track_id: src,
        });
    }

    // --- Structural commands (via mpsc command channel) ---
    // These carry heap-allocated data (Engine) and run on the audio thread.

    /// Add a track at runtime. Returns the pre-assigned track ID.
    pub fn add_track(&mut self, backend: Box<dyn AudioBackend>, channel_mask: u16) -> u32 {
        let id = self.next_track_id;
        self.next_track_id += 1;
        let _ = self.command_tx.send(Box::new(move |mixer| {
            mixer.add_track_with_id(id, backend, None, channel_mask);
        }));
        id
    }

    /// Remove a track at runtime. Notes are silenced before removal.
    pub fn remove_track(&mut self, track_id: u32) {
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

    // --- Transport ---

    pub fn play(&self) {
        self.transport.play();
    }

    pub fn pause_playback(&self) {
        self.transport.pause();
    }

    pub fn stop_playback(&self) {
        self.transport.stop();
    }

    pub fn is_playing(&self) -> bool {
        self.transport.is_playing()
    }

    pub fn set_tempo(&self, bpm: f64) {
        self.transport.set_tempo(bpm);
    }

    pub fn set_loop(&self, enabled: bool) {
        self.transport.set_loop(enabled);
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
