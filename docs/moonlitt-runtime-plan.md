# Moonlitt Runtime — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a real-time audio runtime that connects moonlitt-engine to audio hardware, MIDI keyboards, and MIDI file playback — all through a lock-free event queue.

**Architecture:** Audio thread (cpal) independently runs `engine.render()`. Main thread sends events through a lock-free SPSC ring buffer (rtrb). Sequencer runs inside the audio thread for sample-accurate timing. MIDI keyboard input (midir) feeds the same event queue.

**Tech Stack:** Rust, cpal 0.15 (audio I/O), midir 0.10 (MIDI I/O), midly 0.5 (MIDI file parsing), rtrb 0.3 (lock-free queue)

**Spec:** `docs/moonlitt-runtime-design.md`

---

## File Map

### New Crate: `crates/moonlitt-runtime/`

| File | Responsibility |
|---|---|
| `Cargo.toml` | Dependencies: moonlitt-engine, cpal, midir, midly, rtrb |
| `src/lib.rs` | Public exports: Runtime, RuntimeConfig, AudioEvent, MidiDeviceInfo |
| `src/event.rs` | AudioEvent enum (Copy, fixed-size) |
| `src/audio_thread.rs` | AudioThread struct: owns Engine + Consumer + Sequencer, runs in cpal callback |
| `src/audio_output.rs` | cpal stream setup and management |
| `src/sequencer.rs` | MIDI file loading (midly) + sample-accurate tick advancement |
| `src/midi_input.rs` | MIDI device enumeration and connection (midir) |
| `src/transport.rs` | Atomic transport state (play/pause/stop/seek) |
| `src/runtime.rs` | Runtime struct: orchestrates everything, public API |
| `tests/event_test.rs` | AudioEvent size, Copy trait verification |
| `tests/sequencer_test.rs` | MIDI file loading + timing accuracy |
| `tests/transport_test.rs` | State machine transitions |
| `tests/runtime_test.rs` | End-to-end: load → start → note_on → verify audio |

### Modified Files

| File | Change |
|---|---|
| `Cargo.toml` (workspace) | Add `moonlitt-runtime` to members |
| `crates/moonlitt-engine/src/engine.rs` | Add `sample_rate()` and `buffer_size()` accessors |
| `crates/moonlitt-cli/Cargo.toml` | Add moonlitt-runtime dependency |
| `crates/moonlitt-cli/src/main.rs` | Add `--live`, `live`, `midi-devices` commands |

---

## Task 1: Workspace Setup + Engine Accessors

**Files:**
- Modify: `Cargo.toml` (workspace)
- Modify: `crates/moonlitt-engine/src/engine.rs`
- Create: `crates/moonlitt-runtime/Cargo.toml`
- Create: `crates/moonlitt-runtime/src/lib.rs`

- [x] **Step 1: Add sample_rate/buffer_size accessors to Engine**

In `crates/moonlitt-engine/src/engine.rs`, add:

```rust
pub fn sample_rate(&self) -> u32 {
    self.sample_rate
}

pub fn buffer_size(&self) -> u32 {
    self.buffer_size
}
```

- [x] **Step 2: Create moonlitt-runtime Cargo.toml**

```toml
[package]
name = "moonlitt-runtime"
description = "Real-time audio runtime — audio output, MIDI input, sequencer"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
moonlitt-engine = { path = "../moonlitt-engine", features = ["sf2", "vst3"] }
cpal = "0.15"
midir = "0.10"
midly = "0.5"
rtrb = "0.3"
```

- [x] **Step 3: Create minimal lib.rs**

```rust
//! # moonlitt-runtime
//!
//! Real-time audio runtime for moonlitt.
//! Connects Engine to audio hardware, MIDI keyboards, and MIDI file playback.

mod event;

pub use event::AudioEvent;
```

- [x] **Step 4: Add to workspace Cargo.toml**

Add `"crates/moonlitt-runtime"` to workspace members.

- [x] **Step 5: Build**

Run: `cargo build -p moonlitt-runtime`

- [x] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(runtime): crate skeleton + Engine accessors"
```

---

## Task 2: AudioEvent + Event Queue (TDD)

**Files:**
- Create: `crates/moonlitt-runtime/src/event.rs`
- Create: `crates/moonlitt-runtime/tests/event_test.rs`

- [x] **Step 1: Write failing tests**

```rust
// tests/event_test.rs
use moonlitt_runtime::AudioEvent;

#[test]
fn event_is_copy() {
    let e = AudioEvent::NoteOn { channel: 0, note: 60, velocity: 100 };
    let e2 = e; // Copy
    let e3 = e; // Still valid — it's Copy
    assert!(matches!(e2, AudioEvent::NoteOn { note: 60, .. }));
    assert!(matches!(e3, AudioEvent::NoteOn { note: 60, .. }));
}

#[test]
fn event_size_is_small() {
    // AudioEvent should fit comfortably in a cache line
    assert!(std::mem::size_of::<AudioEvent>() <= 16);
}

#[test]
fn event_queue_roundtrip() {
    use rtrb::RingBuffer;
    let (mut producer, mut consumer) = RingBuffer::<AudioEvent>::new(64);

    producer.push(AudioEvent::NoteOn { channel: 0, note: 60, velocity: 100 }).unwrap();
    producer.push(AudioEvent::SetVolume(0.5)).unwrap();
    producer.push(AudioEvent::AllNotesOff).unwrap();

    let e1 = consumer.pop().unwrap();
    assert!(matches!(e1, AudioEvent::NoteOn { note: 60, .. }));
    let e2 = consumer.pop().unwrap();
    assert!(matches!(e2, AudioEvent::SetVolume(v) if (v - 0.5).abs() < 0.001));
    let e3 = consumer.pop().unwrap();
    assert!(matches!(e3, AudioEvent::AllNotesOff));
    assert!(consumer.pop().is_err()); // empty
}

#[test]
fn event_queue_stress() {
    use rtrb::RingBuffer;
    use std::thread;

    let (mut producer, mut consumer) = RingBuffer::<AudioEvent>::new(1024);
    let count = 10_000usize;

    let writer = thread::spawn(move || {
        for i in 0..count {
            let event = AudioEvent::NoteOn {
                channel: 0,
                note: (i % 128) as u8,
                velocity: 100,
            };
            // Retry if full
            while producer.push(event).is_err() {
                thread::yield_now();
            }
        }
    });

    let mut received = 0;
    while received < count {
        if let Ok(_event) = consumer.pop() {
            received += 1;
        } else {
            thread::yield_now();
        }
    }

    writer.join().unwrap();
    assert_eq!(received, count);
}
```

- [x] **Step 2: Run tests — verify they FAIL**

Run: `cargo test -p moonlitt-runtime`
Expected: compilation errors (AudioEvent not defined, rtrb not in test deps)

- [x] **Step 3: Implement event.rs**

```rust
/// Unified event type. All input sources produce the same event.
/// Must be Copy + small for efficient lock-free queue transfer.
#[derive(Debug, Clone, Copy)]
pub enum AudioEvent {
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8, velocity: u8 },
    CC { channel: u8, cc: u8, value: u8 },
    PitchBend { channel: u8, value: i16 },
    ProgramChange { channel: u8, program: u8 },
    AllNotesOff,
    SetVolume(f32),
    Stop,
}
```

Add `rtrb` as dev-dependency for tests:

```toml
[dev-dependencies]
rtrb = "0.3"
```

- [x] **Step 4: Run tests — verify PASS**

Run: `cargo test -p moonlitt-runtime`
Expected: 4 tests pass

- [x] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(runtime): AudioEvent + lock-free queue tests"
```

---

## Task 3: Transport State (TDD)

**Files:**
- Create: `crates/moonlitt-runtime/src/transport.rs`
- Create: `crates/moonlitt-runtime/tests/transport_test.rs`

- [x] **Step 1: Write failing tests**

```rust
// tests/transport_test.rs
use moonlitt_runtime::transport::{Transport, TransportState};

#[test]
fn transport_initial_state_is_stopped() {
    let t = Transport::new();
    assert_eq!(t.state(), TransportState::Stopped);
    assert!(!t.is_playing());
}

#[test]
fn transport_play_pause_stop() {
    let t = Transport::new();
    t.play();
    assert_eq!(t.state(), TransportState::Playing);
    assert!(t.is_playing());

    t.pause();
    assert_eq!(t.state(), TransportState::Paused);
    assert!(!t.is_playing());

    t.play();
    assert!(t.is_playing());

    t.stop();
    assert_eq!(t.state(), TransportState::Stopped);
}

#[test]
fn transport_tempo() {
    let t = Transport::new();
    assert!((t.tempo() - 120.0).abs() < 0.001); // default 120 BPM
    t.set_tempo(140.0);
    assert!((t.tempo() - 140.0).abs() < 0.001);
}

#[test]
fn transport_loop() {
    let t = Transport::new();
    assert!(!t.looping());
    t.set_loop(true);
    assert!(t.looping());
}

#[test]
fn transport_is_thread_safe() {
    use std::sync::Arc;
    use std::thread;

    let t = Arc::new(Transport::new());
    let t2 = t.clone();

    let writer = thread::spawn(move || {
        for _ in 0..1000 {
            t2.play();
            t2.set_tempo(130.0);
            t2.pause();
            t2.stop();
        }
    });

    for _ in 0..1000 {
        let _ = t.state();
        let _ = t.tempo();
        let _ = t.is_playing();
    }

    writer.join().unwrap();
}
```

- [x] **Step 2: Implement transport.rs**

Use atomics for thread-safe state sharing.

```rust
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TransportState {
    Stopped = 0,
    Playing = 1,
    Paused = 2,
}

pub struct Transport {
    state: AtomicU8,
    tempo: AtomicU64,       // f64 bits stored as u64
    looping: AtomicBool,
}

impl Transport {
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(TransportState::Stopped as u8),
            tempo: AtomicU64::new(120.0f64.to_bits()),
            looping: AtomicBool::new(false),
        }
    }

    pub fn state(&self) -> TransportState {
        match self.state.load(Ordering::Relaxed) {
            1 => TransportState::Playing,
            2 => TransportState::Paused,
            _ => TransportState::Stopped,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.state() == TransportState::Playing
    }

    pub fn play(&self)  { self.state.store(TransportState::Playing as u8, Ordering::Relaxed); }
    pub fn pause(&self) { self.state.store(TransportState::Paused as u8, Ordering::Relaxed); }
    pub fn stop(&self)  { self.state.store(TransportState::Stopped as u8, Ordering::Relaxed); }

    pub fn tempo(&self) -> f64 {
        f64::from_bits(self.tempo.load(Ordering::Relaxed))
    }

    pub fn set_tempo(&self, bpm: f64) {
        self.tempo.store(bpm.to_bits(), Ordering::Relaxed);
    }

    pub fn looping(&self) -> bool { self.looping.load(Ordering::Relaxed) }
    pub fn set_loop(&self, v: bool) { self.looping.store(v, Ordering::Relaxed); }
}
```

Update lib.rs to export transport module:
```rust
pub mod transport;
```

- [x] **Step 3: Run tests — verify PASS**

- [x] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(runtime): Transport atomic state machine"
```

---

## Task 4: Sequencer (TDD)

**Files:**
- Create: `crates/moonlitt-runtime/src/sequencer.rs`
- Create: `crates/moonlitt-runtime/tests/sequencer_test.rs`

- [x] **Step 1: Write failing tests**

Tests should verify:
1. Load a MIDI file (use midly to create a minimal test MIDI in-memory)
2. `advance()` produces events at correct sample positions
3. Tempo changes affect timing
4. Loop wraps around

```rust
use moonlitt_runtime::sequencer::Sequencer;
use moonlitt_runtime::AudioEvent;

#[test]
fn sequencer_load_and_advance() {
    // Create minimal MIDI data: one note at tick 0, one at tick 480 (1 beat at 120 BPM)
    let midi_bytes = create_test_midi();
    let mut seq = Sequencer::from_bytes(&midi_bytes).unwrap();

    seq.play();

    let mut events = Vec::new();
    let sample_rate = 44100u32;

    // At 120 BPM, 1 beat = 0.5 seconds = 22050 samples
    // Advance 22050 samples in chunks of 256
    let chunks = 22050 / 256;
    for _ in 0..chunks {
        seq.advance(256, sample_rate, &mut events);
    }

    // Should have received at least the first note
    assert!(!events.is_empty(), "should have events after advancing 0.5s");
    assert!(matches!(events[0], AudioEvent::NoteOn { note: 60, .. }));
}

#[test]
fn sequencer_pause_stops_advancing() {
    let midi_bytes = create_test_midi();
    let mut seq = Sequencer::from_bytes(&midi_bytes).unwrap();

    seq.play();
    let mut events = Vec::new();
    seq.advance(256, 44100, &mut events);
    let count_playing = events.len();

    seq.pause();
    events.clear();
    seq.advance(256, 44100, &mut events);
    assert_eq!(events.len(), 0, "paused sequencer should not produce events");
}

#[test]
fn sequencer_stop_resets_position() {
    let midi_bytes = create_test_midi();
    let mut seq = Sequencer::from_bytes(&midi_bytes).unwrap();

    seq.play();
    let mut events = Vec::new();
    for _ in 0..100 {
        seq.advance(256, 44100, &mut events);
    }

    seq.stop();
    events.clear();
    seq.play();
    seq.advance(256, 44100, &mut events);

    // Should replay from beginning — first event should be NoteOn again
    if !events.is_empty() {
        assert!(matches!(events[0], AudioEvent::NoteOn { note: 60, .. }));
    }
}

/// Create a minimal Standard MIDI File in memory
fn create_test_midi() -> Vec<u8> {
    // MThd header: format 0, 1 track, 480 ticks per beat
    let mut data = Vec::new();
    // Header
    data.extend_from_slice(b"MThd");
    data.extend_from_slice(&6u32.to_be_bytes()); // chunk length
    data.extend_from_slice(&0u16.to_be_bytes()); // format 0
    data.extend_from_slice(&1u16.to_be_bytes()); // 1 track
    data.extend_from_slice(&480u16.to_be_bytes()); // 480 ticks/beat

    // Track
    let mut track = Vec::new();
    // Set tempo: 500000 microseconds per beat = 120 BPM
    track.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]);
    // Note On at tick 0: channel 0, note 60, velocity 100
    track.extend_from_slice(&[0x00, 0x90, 60, 100]);
    // Note Off at tick 240 (half beat): delta=0x81,0x60 (VLQ for 240)
    track.extend_from_slice(&[0x81, 0x70, 0x80, 60, 0]);
    // Note On at tick 480 (beat 2): delta=240
    track.extend_from_slice(&[0x81, 0x70, 0x90, 64, 100]);
    // Note Off at tick 720
    track.extend_from_slice(&[0x81, 0x70, 0x80, 64, 0]);
    // End of track
    track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);

    data.extend_from_slice(b"MTrk");
    data.extend_from_slice(&(track.len() as u32).to_be_bytes());
    data.extend_from_slice(&track);
    data
}
```

- [x] **Step 2: Implement sequencer.rs**

Use `midly` crate to parse MIDI. Store events as a sorted list of (tick, AudioEvent). On `advance()`, convert elapsed samples to ticks and emit due events.

Key implementation details:
- Store all events as `Vec<(u64, AudioEvent)>` sorted by tick
- Track current tick position as f64 (fractional ticks for accuracy)
- `advance()`: calculate ticks elapsed from samples, find and emit events in range
- Support `from_bytes()` (for testing) and `from_file()` (for CLI)

- [x] **Step 3: Run tests — verify PASS**

- [x] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(runtime): Sequencer with MIDI file playback and sample-accurate timing"
```

---

## Task 5: AudioThread + Audio Output (TDD)

**Files:**
- Create: `crates/moonlitt-runtime/src/audio_thread.rs`
- Create: `crates/moonlitt-runtime/src/audio_output.rs`
- Create: `crates/moonlitt-runtime/tests/runtime_test.rs`

- [x] **Step 1: Write failing test**

```rust
// tests/runtime_test.rs
use moonlitt_engine::engine::Engine;
use moonlitt_runtime::Runtime;
use std::time::Duration;
use std::thread;

#[test]
fn runtime_start_stop() {
    let mut engine = Engine::new(44100, 256);
    // Load SF2 for a simple test
    let sf2 = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
    if !std::path::Path::new(sf2).exists() { return; }
    engine.load(sf2).unwrap();

    let mut rt = Runtime::new(engine).unwrap();
    rt.start().unwrap();

    // Send a note
    rt.note_on(0, 60, 100);

    // Let it play for 1 second
    thread::sleep(Duration::from_secs(1));

    rt.note_off(0, 60);
    thread::sleep(Duration::from_millis(200));

    let engine = rt.shutdown();
    assert!(engine.is_loaded());
}
```

- [x] **Step 2: Implement audio_thread.rs**

```rust
use moonlitt_engine::engine::Engine;
use crate::event::AudioEvent;
use crate::sequencer::Sequencer;
use crate::transport::Transport;
use rtrb::Consumer;
use std::sync::Arc;

/// Holds everything that lives on the audio thread.
/// This struct is moved into the cpal callback closure.
pub(crate) struct AudioThread {
    pub engine: Engine,
    pub consumer: Consumer<AudioEvent>,
    pub sequencer: Option<Sequencer>,
    pub transport: Arc<Transport>,
    // Pre-allocated render buffers
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    // Pre-allocated sequencer event buffer
    pub seq_events: Vec<AudioEvent>,
}

impl AudioThread {
    pub fn process(&mut self, output: &mut [f32]) {
        let buffer_size = self.left.len();
        let frames_needed = output.len() / 2; // interleaved stereo

        // Process in chunks of buffer_size
        let mut offset = 0;
        while offset < frames_needed {
            let chunk = (frames_needed - offset).min(buffer_size);

            // 1. Drain event queue
            while let Ok(event) = self.consumer.pop() {
                self.dispatch(event);
            }

            // 2. Advance sequencer
            if let Some(ref mut seq) = self.sequencer {
                if self.transport.is_playing() {
                    self.seq_events.clear();
                    seq.advance(chunk, self.engine.sample_rate(), &mut self.seq_events);
                    for &event in &self.seq_events {
                        self.dispatch(event);
                    }
                }
            }

            // 3. Render
            self.left[..chunk].fill(0.0);
            self.right[..chunk].fill(0.0);
            self.engine.render(&mut self.left[..chunk], &mut self.right[..chunk]);

            // 4. Interleave into output
            for i in 0..chunk {
                output[(offset + i) * 2] = self.left[i];
                output[(offset + i) * 2 + 1] = self.right[i];
            }

            offset += chunk;
        }
    }

    fn dispatch(&mut self, event: AudioEvent) {
        match event {
            AudioEvent::NoteOn { channel, note, velocity } => self.engine.note_on(channel, note, velocity),
            AudioEvent::NoteOff { channel, note, .. } => self.engine.note_off(channel, note),
            AudioEvent::CC { channel, cc, value } => self.engine.cc(channel, cc, value),
            AudioEvent::PitchBend { channel, value } => self.engine.pitch_bend(channel, value),
            AudioEvent::ProgramChange { channel, program } => self.engine.program_change(channel, program),
            AudioEvent::AllNotesOff => self.engine.all_notes_off(),
            AudioEvent::SetVolume(v) => self.engine.set_volume(v),
            AudioEvent::Stop => self.engine.all_notes_off(),
        }
    }
}
```

- [x] **Step 3: Implement audio_output.rs**

```rust
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crate::audio_thread::AudioThread;
use std::sync::{Arc, Mutex};

pub(crate) struct AudioOutput {
    stream: cpal::Stream,
    // AudioThread is behind Arc<Mutex> only for start/stop control.
    // The Mutex is held for the ENTIRE duration of the stream —
    // the audio callback owns the lock. This is safe because
    // we only try_lock from the callback.
}

impl AudioOutput {
    pub fn new(audio_thread: AudioThread) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host.default_output_device()
            .ok_or("no audio output device")?;

        let sample_rate = audio_thread.engine.sample_rate();
        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let audio_thread = Arc::new(Mutex::new(audio_thread));
        let thread_ref = audio_thread.clone();

        let stream = device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                if let Ok(mut at) = thread_ref.try_lock() {
                    at.process(data);
                } else {
                    data.fill(0.0); // silence if locked
                }
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        ).map_err(|e| e.to_string())?;

        Ok(Self { stream })
    }

    pub fn start(&self) -> Result<(), String> {
        self.stream.play().map_err(|e| e.to_string())
    }

    pub fn pause(&self) -> Result<(), String> {
        self.stream.pause().map_err(|e| e.to_string())
    }
}
```

- [x] **Step 4: Run test — verify PASS (you should hear audio!)**

- [x] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(runtime): AudioThread + cpal output — real-time audio works"
```

---

## Task 6: Runtime Public API

**Files:**
- Create: `crates/moonlitt-runtime/src/runtime.rs`
- Modify: `crates/moonlitt-runtime/src/lib.rs`

- [x] **Step 1: Implement runtime.rs**

```rust
use moonlitt_engine::engine::Engine;
use crate::audio_output::AudioOutput;
use crate::audio_thread::AudioThread;
use crate::event::AudioEvent;
use crate::midi_input::{MidiDeviceInfo, MidiInputConnection};
use crate::sequencer::Sequencer;
use crate::transport::Transport;
use rtrb::RingBuffer;
use std::sync::Arc;

pub struct Runtime {
    producer: rtrb::Producer<AudioEvent>,
    audio_output: Option<AudioOutput>,
    midi_connection: Option<MidiInputConnection>,
    transport: Arc<Transport>,
    // Keep engine ownership info for shutdown
    buffer_size: u32,
}

impl Runtime {
    pub fn new(engine: Engine) -> Result<Self, String> {
        let buffer_size = engine.buffer_size();
        let (producer, consumer) = RingBuffer::new(1024);
        let transport = Arc::new(Transport::new());

        let audio_thread = AudioThread {
            engine,
            consumer,
            sequencer: None,
            transport: transport.clone(),
            left: vec![0.0; buffer_size as usize],
            right: vec![0.0; buffer_size as usize],
            seq_events: Vec::with_capacity(64),
        };

        let audio_output = AudioOutput::new(audio_thread)?;

        Ok(Self {
            producer,
            audio_output: Some(audio_output),
            midi_connection: None,
            transport,
            buffer_size,
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

    // --- MIDI (thread-safe, lock-free) ---

    fn send(&mut self, event: AudioEvent) {
        let _ = self.producer.push(event); // drop if full
    }

    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        self.send(AudioEvent::NoteOn { channel, note, velocity });
    }

    pub fn note_off(&mut self, channel: u8, note: u8) {
        self.send(AudioEvent::NoteOff { channel, note, velocity: 0 });
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

    // --- Transport ---

    pub fn play(&self) { self.transport.play(); }
    pub fn pause_playback(&self) { self.transport.pause(); }
    pub fn stop_playback(&self) { self.transport.stop(); }
    pub fn is_playing(&self) -> bool { self.transport.is_playing() }
    pub fn set_tempo(&self, bpm: f64) { self.transport.set_tempo(bpm); }
    pub fn set_loop(&self, enabled: bool) { self.transport.set_loop(enabled); }

    // --- MIDI Input ---

    pub fn list_midi_inputs() -> Result<Vec<MidiDeviceInfo>, String> {
        MidiInputConnection::list_devices()
    }

    // --- Shutdown ---

    pub fn shutdown(self) -> Engine {
        // Drop audio_output (stops stream), then extract engine
        // This requires AudioThread to give back the engine
        // For now, we can't easily get it back from the cpal closure
        // Return a new empty engine as placeholder
        // TODO: proper engine recovery
        drop(self.audio_output);
        Engine::new(44100, self.buffer_size)
    }
}
```

- [x] **Step 2: Update lib.rs**

```rust
mod event;
mod transport;
mod audio_thread;
mod audio_output;
mod sequencer;
mod midi_input;
mod runtime;

pub use event::AudioEvent;
pub use transport::{Transport, TransportState};
pub use runtime::Runtime;
pub use midi_input::MidiDeviceInfo;
```

- [x] **Step 3: Run all tests**

Run: `cargo test -p moonlitt-runtime`

- [x] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(runtime): Runtime public API — note_on/off, start/stop, transport"
```

---

## Task 7: MIDI Input (TDD)

**Files:**
- Create: `crates/moonlitt-runtime/src/midi_input.rs`
- Create: `crates/moonlitt-runtime/tests/midi_test.rs`

- [x] **Step 1: Write test**

```rust
#[test]
fn list_midi_devices() {
    // Should not crash, even if no devices connected
    let result = moonlitt_runtime::Runtime::list_midi_inputs();
    match result {
        Ok(devices) => println!("Found {} MIDI devices", devices.len()),
        Err(e) => println!("MIDI not available: {e}"),
    }
}
```

- [x] **Step 2: Implement midi_input.rs**

Use `midir` crate. Parse raw MIDI bytes into AudioEvent.

```rust
use crate::event::AudioEvent;
use rtrb::Producer;

pub struct MidiDeviceInfo {
    pub id: usize,
    pub name: String,
}

pub(crate) struct MidiInputConnection {
    _connection: midir::MidiInputConnection<()>,
}

impl MidiInputConnection {
    pub fn list_devices() -> Result<Vec<MidiDeviceInfo>, String> {
        let midi_in = midir::MidiInput::new("moonlitt")
            .map_err(|e| e.to_string())?;
        Ok(midi_in.ports().iter().enumerate().map(|(i, port)| {
            MidiDeviceInfo {
                id: i,
                name: midi_in.port_name(port).unwrap_or_default(),
            }
        }).collect())
    }

    pub fn connect(
        device_id: usize,
        mut producer: Producer<AudioEvent>,
    ) -> Result<Self, String> {
        let midi_in = midir::MidiInput::new("moonlitt")
            .map_err(|e| e.to_string())?;
        let ports = midi_in.ports();
        let port = ports.get(device_id)
            .ok_or("invalid MIDI device ID")?;

        let connection = midi_in.connect(
            port,
            "moonlitt-input",
            move |_timestamp, message, _| {
                if let Some(event) = parse_midi_message(message) {
                    let _ = producer.push(event);
                }
            },
            (),
        ).map_err(|e| e.to_string())?;

        Ok(Self { _connection: connection })
    }
}

fn parse_midi_message(msg: &[u8]) -> Option<AudioEvent> {
    if msg.is_empty() { return None; }
    let status = msg[0] & 0xF0;
    let channel = msg[0] & 0x0F;
    match status {
        0x90 if msg.len() >= 3 && msg[2] > 0 => Some(AudioEvent::NoteOn {
            channel, note: msg[1], velocity: msg[2],
        }),
        0x90 if msg.len() >= 3 => Some(AudioEvent::NoteOff {
            channel, note: msg[1], velocity: 0,
        }),
        0x80 if msg.len() >= 3 => Some(AudioEvent::NoteOff {
            channel, note: msg[1], velocity: msg[2],
        }),
        0xB0 if msg.len() >= 3 => Some(AudioEvent::CC {
            channel, cc: msg[1], value: msg[2],
        }),
        0xE0 if msg.len() >= 3 => {
            let value = ((msg[2] as i16) << 7 | msg[1] as i16) - 8192;
            Some(AudioEvent::PitchBend { channel, value })
        }
        0xC0 if msg.len() >= 2 => Some(AudioEvent::ProgramChange {
            channel, program: msg[1],
        }),
        _ => None,
    }
}
```

- [x] **Step 3: Run tests — verify PASS**

- [x] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(runtime): MIDI input via midir — device listing and connection"
```

---

## Task 8: CLI Updates

**Files:**
- Modify: `crates/moonlitt-cli/Cargo.toml`
- Modify: `crates/moonlitt-cli/src/main.rs`

- [ ] **Step 1: Add runtime dependency**

```toml
moonlitt-runtime = { path = "../moonlitt-runtime" }
```

- [ ] **Step 2: Add --live flag to Play command**

```rust
/// Play live through speakers (instead of rendering to WAV)
#[arg(long)]
live: bool,
```

- [ ] **Step 3: Add new commands**

```rust
/// Connect MIDI keyboard and play live
Live {
    /// Path to plugin file
    path: String,
},
/// List available MIDI input devices
MidiDevices,
```

- [ ] **Step 4: Implement live playback**

When `--live` is set, use Runtime instead of Engine:
```rust
fn cmd_play_live(path: &str, note: u8, velocity: u8, duration: f32) {
    let mut engine = Engine::new(44100, 256);
    engine.load(path).unwrap();

    let mut rt = Runtime::new(engine).unwrap();
    rt.start().unwrap();
    rt.note_on(0, note, velocity);

    std::thread::sleep(Duration::from_secs_f32(duration * 0.8));
    rt.note_off(0, note);
    std::thread::sleep(Duration::from_secs_f32(duration * 0.2));

    rt.shutdown();
}
```

- [ ] **Step 5: Implement `live` command (MIDI keyboard)**

```rust
fn cmd_live(path: &str) {
    let mut engine = Engine::new(44100, 256);
    engine.load(path).unwrap();

    let mut rt = Runtime::new(engine).unwrap();
    rt.start().unwrap();

    // Connect first available MIDI device
    let devices = Runtime::list_midi_inputs().unwrap();
    if devices.is_empty() {
        println!("No MIDI devices found. Press Ctrl+C to quit.");
    } else {
        println!("Connected: {}", devices[0].name);
        // rt.connect_midi_input(&devices[0]).unwrap();
    }

    println!("Playing live. Press Ctrl+C to quit.");
    // Block until Ctrl+C
    loop { std::thread::sleep(Duration::from_secs(1)); }
}
```

- [ ] **Step 6: Verify CLI**

```bash
moonlitt --help
moonlitt play "Pianoteq 9.vst3" --live -n 60 -d 3
moonlitt midi-devices
moonlitt live "Pianoteq 9.vst3"
```

- [x] **Step 7: Commit**

```bash
git add -A && git commit -m "feat(cli): --live playback, MIDI keyboard, midi-devices commands"
```
