use crate::audio_output::AudioOutput;
use crate::audio_thread::AudioThread;
use crate::event::{AudioEvent, TimedEvent};
use crate::midi_input::{MidiDeviceInfo, MidiInputConnection};
use crate::mixer::Mixer;
use crate::transport::Transport;
use moonlitt_engine::engine::Engine;
use rtrb::RingBuffer;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub struct Runtime {
    producer: rtrb::Producer<TimedEvent>,
    audio_output: Option<AudioOutput>,
    #[allow(dead_code)]
    midi_connection: Option<MidiInputConnection>,
    transport: Arc<Transport>,
    #[allow(dead_code)]
    buffer_size: u32,
    /// Counter for events dropped due to ring buffer overflow.
    dropped_events: Arc<AtomicU64>,
}

impl Runtime {
    /// Create a runtime with a single engine (backward compatible).
    /// The engine is placed in a Mixer as the sole track handling all 16 channels.
    pub fn new(engine: Engine) -> Result<Self, String> {
        let buffer_size = engine.buffer_size();
        let sample_rate = engine.sample_rate();

        let mut mixer = Mixer::new(sample_rate, buffer_size as usize);
        mixer.add_track(engine, 0xFFFF); // all 16 channels

        Self::with_mixer(mixer, buffer_size)
    }

    /// Create a runtime with a pre-configured Mixer.
    pub fn with_mixer(mixer: Mixer, buffer_size: u32) -> Result<Self, String> {
        // Ring buffer capacity: 1024 events. Sufficient for real-time MIDI at
        // typical rates; events are drained every audio callback (~5ms).
        let (producer, consumer) = RingBuffer::new(1024);
        let transport = Arc::new(Transport::new());

        let audio_thread = AudioThread::new(
            mixer,
            consumer,
            None,
            transport.clone(),
            buffer_size as usize,
        );

        let audio_output = AudioOutput::new(audio_thread)?;

        Ok(Self {
            producer,
            audio_output: Some(audio_output),
            midi_connection: None,
            transport,
            buffer_size,
            dropped_events: Arc::new(AtomicU64::new(0)),
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

    // --- Mixer control (thread-safe via ring buffer) ---

    pub fn mixer_set_track_volume(&mut self, track_id: u8, volume: f32) {
        self.send(AudioEvent::MixerTrackVolume { track_id, volume });
    }

    pub fn mixer_set_track_pan(&mut self, track_id: u8, pan: f32) {
        self.send(AudioEvent::MixerTrackPan { track_id, pan });
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
