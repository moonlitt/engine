mod wav;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "moonlitt", about = "Audio engine CLI for scanning, playing, and rendering")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan for available audio plugins (VST3/CLAP/SF2)
    Scan {
        /// Directory to scan (default: system paths)
        #[arg(short, long)]
        dir: Option<String>,
    },
    /// Show detailed info about a plugin
    Info {
        /// Path to plugin file (.sf2, .vst3, .clap)
        path: String,
    },
    /// Load a plugin and play a test note (renders to WAV)
    Play {
        /// Path to plugin file
        path: String,
        /// MIDI note number (default: 60 = middle C)
        #[arg(short, long, default_value = "60")]
        note: u8,
        /// Velocity (default: 100)
        #[arg(short, long, default_value = "100")]
        velocity: u8,
        /// Duration in seconds (default: 2.0)
        #[arg(short, long, default_value = "2.0")]
        duration: f32,
        /// Output WAV file (default: output.wav)
        #[arg(short, long, default_value = "output.wav")]
        output: String,
        /// Sample rate (default: 44100)
        #[arg(long, default_value = "44100")]
        sample_rate: u32,
        /// Buffer size (default: 256)
        #[arg(long, default_value = "256")]
        buffer_size: u32,
        /// Play live through speakers (instead of rendering to WAV)
        #[arg(long)]
        live: bool,
    },
    /// List presets for a plugin
    Presets {
        /// Path to plugin file
        path: String,
    },
    /// Connect MIDI keyboard and play live
    Live {
        /// Path to plugin file
        path: String,
    },
    /// Play a MIDI file through a soundfont/plugin
    Midi {
        /// Path to MIDI file
        midi: String,
        /// Path to soundfont/plugin (SF2/VST3/CLAP)
        #[arg(short, long)]
        sound: String,
        /// Play live through speakers (default: render to WAV)
        #[arg(long)]
        live: bool,
        /// Use moonlitt-sampler (pure Rust, Sinc 72) instead of OxiSynth (SF2 only)
        #[arg(long)]
        sampler: bool,
        /// Insert effect. Repeat for chain. Format: "type[:k=v,k=v,...]"
        /// Types: compressor, plate, freeverb. Examples:
        ///   --insert "compressor:threshold=-18,ratio=4,makeup=6"
        ///   --insert "plate:wet=0.2,decay=0.6"
        #[arg(short = 'i', long, value_name = "SPEC")]
        insert: Vec<String>,
        /// Output WAV file (when not --live)
        #[arg(short, long, default_value = "output.wav")]
        output: String,
    },
    /// List available MIDI input devices
    MidiDevices,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Scan { dir } => cmd_scan(dir),
        Commands::Info { path } => cmd_info(&path),
        Commands::Play {
            path,
            note,
            velocity,
            duration,
            output,
            sample_rate,
            buffer_size,
            live,
        } => {
            if live {
                cmd_play_live(&path, note, velocity, duration, sample_rate, buffer_size);
            } else {
                cmd_play(&path, note, velocity, duration, &output, sample_rate, buffer_size);
            }
        }
        Commands::Presets { path } => cmd_presets(&path),
        Commands::Live { path } => cmd_live(&path),
        Commands::Midi { midi, sound, live, sampler, insert, output } => {
            if live {
                cmd_midi_live(&midi, &sound, sampler, &insert);
            } else {
                cmd_midi_render(&midi, &sound, &output, sampler);
            }
        }
        Commands::MidiDevices => cmd_midi_devices(),
    }
}

fn cmd_scan(_dir: Option<String>) {
    let plugins = moonlitt_engine::scan_plugins(44100, 256);

    if plugins.is_empty() {
        println!("No plugins found.");
        return;
    }

    println!("{:<40} {:<8} Path", "Name", "Format");
    println!("{}", "-".repeat(80));
    for p in &plugins {
        println!("{:<40} {:<8} {}", p.name, format!("{:?}", p.format), p.path);
    }
    println!("\nTotal: {} plugins", plugins.len());
}

fn cmd_info(path: &str) {
    let backend = match moonlitt_engine::create(path, 44100, 256) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error loading {path}: {e}");
            std::process::exit(1);
        }
    };

    let info = backend.info();
    println!("Backend:    {}", info.name);
    println!("Type:       {:?}", info.backend_type);
    println!("Extensions: {}", info.extensions.join(", "));

    let presets = backend.presets();
    println!("Presets:    {}", presets.len());
}

fn cmd_play(
    path: &str,
    note: u8,
    velocity: u8,
    duration: f32,
    output: &str,
    sample_rate: u32,
    buffer_size: u32,
) {
    let mut backend = match moonlitt_engine::create(path, sample_rate, buffer_size) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error loading {path}: {e}");
            std::process::exit(1);
        }
    };

    let total_samples = (sample_rate as f32 * duration) as usize;
    let num_buffers = total_samples.div_ceil(buffer_size as usize);

    // Note-on duration: 80% of total, then note-off for tail
    let note_off_buffer = (num_buffers as f32 * 0.8) as usize;

    let mut all_left = Vec::with_capacity(total_samples);
    let mut all_right = Vec::with_capacity(total_samples);

    let mut left = vec![0.0f32; buffer_size as usize];
    let mut right = vec![0.0f32; buffer_size as usize];

    backend.note_on(0, note, velocity);

    for i in 0..num_buffers {
        if i == note_off_buffer {
            backend.note_off(0, note);
        }
        backend.render(&mut left, &mut right);
        all_left.extend_from_slice(&left);
        all_right.extend_from_slice(&right);
    }

    // Trim to exact length
    all_left.truncate(total_samples);
    all_right.truncate(total_samples);

    let peak = all_left
        .iter()
        .chain(all_right.iter())
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);

    match wav::write_wav(output, sample_rate, &all_left, &all_right) {
        Ok(()) => {
            println!("Rendered {duration}s to {output}");
            println!("  Note: {note}, Velocity: {velocity}");
            println!("  Sample rate: {sample_rate} Hz");
            println!("  Peak amplitude: {peak:.4}");
            println!("  Samples: {total_samples}");
        }
        Err(e) => {
            eprintln!("Error writing WAV: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_play_live(path: &str, note: u8, velocity: u8, duration: f32, sample_rate: u32, buffer_size: u32) {
    use moonlitt_audio_io::Runtime;
    use std::thread;
    use std::time::Duration;

    let backend = match moonlitt_engine::create(path, sample_rate, buffer_size) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    let mut rt = match Runtime::new(backend, sample_rate, buffer_size) {
        Ok(r) => r,
        Err((e, _)) => {
            eprintln!("Audio error: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = rt.start() {
        eprintln!("Failed to start audio output: {e}");
        std::process::exit(1);
    }
    println!("Playing note {note} (velocity {velocity}) for {duration}s...");

    rt.note_on(0, note, velocity);
    thread::sleep(Duration::from_secs_f32(duration * 0.8));
    rt.note_off(0, note);
    thread::sleep(Duration::from_secs_f32(duration * 0.2));

    println!("Done.");
    rt.shutdown();
}

fn cmd_live(path: &str) {
    use moonlitt_audio_io::Runtime;
    use std::time::Duration;

    let backend = match moonlitt_engine::create(path, 44100, 256) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    let mut rt = match Runtime::new(backend, 44100, 256) {
        Ok(r) => r,
        Err((e, _)) => {
            eprintln!("Audio error: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = rt.start() {
        eprintln!("Failed to start audio output: {e}");
        std::process::exit(1);
    }
    println!("Live mode. Press Ctrl+C to quit.");

    // TODO: Runtime needs a connect_midi_input(device_id) method so we can
    // route MIDI events from the keyboard into the audio thread's event queue.
    // For now, we list devices for informational purposes only.
    match Runtime::list_midi_inputs() {
        Ok(devices) if !devices.is_empty() => {
            println!("MIDI input detected: {}", devices[0].name);
            println!("(MIDI routing not yet implemented — playing test chord instead)");
        }
        _ => println!("No MIDI devices found. Playing test chord."),
    }

    // Play a C major chord so the command produces audible output
    rt.note_on(0, 60, 80); // C4
    rt.note_on(0, 64, 80); // E4
    rt.note_on(0, 67, 80); // G4
    std::thread::sleep(Duration::from_secs(2));
    rt.note_off(0, 60);
    rt.note_off(0, 64);
    rt.note_off(0, 67);
    std::thread::sleep(Duration::from_millis(500)); // let the tail ring

    println!("Test chord done. Waiting for Ctrl+C...");

    // Block until Ctrl+C
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn cmd_midi_devices() {
    match moonlitt_audio_io::Runtime::list_midi_inputs() {
        Ok(devices) => {
            if devices.is_empty() {
                println!("No MIDI input devices found.");
            } else {
                println!("{:<4} Name", "ID");
                println!("{}", "-".repeat(40));
                for d in &devices {
                    println!("{:<4} {}", d.id, d.name);
                }
            }
        }
        Err(e) => eprintln!("Error: {e}"),
    }
}

// =============================================================================
// MIDI file playback
// =============================================================================
// TODO: This MIDI parsing logic duplicates moonlitt_audio_io::sequencer's
// MIDI parsing. Refactor to share a common MIDI parser crate or move the
// note-event extraction into moonlitt-runtime and expose it publicly.

struct MidiNote {
    time_sec: f64,
    channel: u8,
    note: u8,
    velocity: u8,
    duration_sec: f64,
    program: u8,
}

fn parse_midi_file(path: &str) -> Result<(Vec<MidiNote>, Vec<(f64, u8, u8)>), String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let smf = midly::Smf::parse(&data).map_err(|e| e.to_string())?;

    let tpb = match smf.header.timing {
        midly::Timing::Metrical(t) => t.as_int() as f64,
        _ => return Err("SMPTE not supported".into()),
    };

    // Build tempo map from all tracks
    let mut tempo_events: Vec<(u64, u32)> = vec![(0, 500_000)];
    for track in &smf.tracks {
        let mut abs = 0u64;
        for ev in track {
            abs += ev.delta.as_int() as u64;
            if let midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(t)) = ev.kind {
                tempo_events.push((abs, t.as_int()));
            }
        }
    }
    tempo_events.sort_by_key(|&(t, _)| t);
    tempo_events.dedup_by_key(|e| e.0);

    let tick_to_sec = |tick: u64| -> f64 {
        let mut elapsed = 0.0;
        let mut prev_tick = 0u64;
        let mut prev_tempo = 500_000u32;
        for &(t, tempo) in &tempo_events {
            if t >= tick { break; }
            elapsed += (t - prev_tick) as f64 * prev_tempo as f64 / (tpb * 1_000_000.0);
            prev_tick = t;
            prev_tempo = tempo;
        }
        elapsed + (tick - prev_tick) as f64 * prev_tempo as f64 / (tpb * 1_000_000.0)
    };

    let mut notes = Vec::new();
    let mut program_changes: Vec<(f64, u8, u8)> = Vec::new(); // (time, ch, program)
    let mut programs = std::collections::HashMap::new();

    for track in &smf.tracks {
        let mut abs = 0u64;
        let mut active: std::collections::HashMap<(u8, u8), (u64, u8)> = std::collections::HashMap::new();

        for ev in track {
            abs += ev.delta.as_int() as u64;
            match ev.kind {
                midly::TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        midly::MidiMessage::ProgramChange { program } => {
                            let p = program.as_int();
                            programs.insert(ch, p);
                            program_changes.push((tick_to_sec(abs), ch, p));
                        }
                        midly::MidiMessage::NoteOn { key, vel } => {
                            let v = vel.as_int();
                            if v == 0 {
                                if let Some((start, vel)) = active.remove(&(ch, key.as_int())) {
                                    let s = tick_to_sec(start);
                                    let e = tick_to_sec(abs);
                                    notes.push(MidiNote {
                                        time_sec: s, channel: ch, note: key.as_int(),
                                        velocity: vel, duration_sec: e - s,
                                        program: *programs.get(&ch).unwrap_or(&0),
                                    });
                                }
                            } else {
                                active.insert((ch, key.as_int()), (abs, v));
                            }
                        }
                        midly::MidiMessage::NoteOff { key, .. } => {
                            if let Some((start, vel)) = active.remove(&(ch, key.as_int())) {
                                let s = tick_to_sec(start);
                                let e = tick_to_sec(abs);
                                notes.push(MidiNote {
                                    time_sec: s, channel: ch, note: key.as_int(),
                                    velocity: vel, duration_sec: e - s,
                                    program: *programs.get(&ch).unwrap_or(&0),
                                });
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    notes.sort_by(|a, b| a.time_sec.partial_cmp(&b.time_sec).unwrap());
    Ok((notes, program_changes))
}

fn cmd_midi_live(midi_path: &str, sound_path: &str, use_sampler: bool, insert_specs: &[String]) {
    use moonlitt_audio_io::Runtime;
    use moonlitt_mixer::Mixer;
    use std::thread;
    use std::time::{Duration, Instant};

    const SAMPLE_RATE: u32 = 44100;
    const BUFFER_SIZE: u32 = 256;

    let (notes, program_changes) = match parse_midi_file(midi_path) {
        Ok(v) => v,
        Err(e) => { eprintln!("MIDI parse error: {e}"); std::process::exit(1); }
    };

    let duration = notes.iter()
        .map(|n| n.time_sec + n.duration_sec)
        .fold(0.0f64, f64::max);

    println!("MIDI: {} notes, {:.1}s", notes.len(), duration);

    let backend_result = if use_sampler {
        moonlitt_engine::create_with_sampler(sound_path, SAMPLE_RATE, BUFFER_SIZE)
    } else {
        moonlitt_engine::create(sound_path, SAMPLE_RATE, BUFFER_SIZE)
    };
    let mut backend = match backend_result {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error loading {sound_path}: {e}");
            std::process::exit(1);
        }
    };
    println!("Sound: {}", backend.info().name);

    // Send program changes BEFORE handing the backend to the mixer.
    let mut sent_programs = std::collections::HashSet::new();
    for &(_, ch, prog) in &program_changes {
        if sent_programs.insert((ch, prog)) {
            backend.program_change(ch, prog);
        }
    }

    // Build the mixer: synth track + parsed insert chain (in order).
    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE as usize);
    let track_id = mixer.add_track(backend, 0xFFFF);

    for spec in insert_specs {
        match build_insert(spec, SAMPLE_RATE) {
            Ok(eff) => {
                mixer.add_insert(track_id, eff);
                println!("Insert: {spec}");
            }
            Err(e) => {
                eprintln!("Insert error in '{spec}': {e}");
                std::process::exit(2);
            }
        }
    }

    let mut rt = match Runtime::with_mixer(mixer, BUFFER_SIZE) {
        Ok(r) => r,
        Err(e) => { eprintln!("Audio error: {e}"); std::process::exit(1); }
    };
    if let Err(e) = rt.start() {
        eprintln!("Failed to start audio output: {e}");
        std::process::exit(1);
    }

    println!("Playing...");
    let start = Instant::now();
    let mut note_idx = 0;

    // Schedule note-offs: (time, ch, note)
    let mut pending_offs: Vec<(f64, u8, u8)> = Vec::new();

    loop {
        let elapsed = start.elapsed().as_secs_f64();
        if elapsed > duration + 1.0 { break; }

        // Process note-offs
        pending_offs.retain(|&(off_time, ch, note)| {
            if elapsed >= off_time {
                rt.note_off(ch, note);
                false
            } else {
                true
            }
        });

        // Process note-ons
        while note_idx < notes.len() && notes[note_idx].time_sec <= elapsed {
            let n = &notes[note_idx];
            rt.note_on(n.channel, n.note, n.velocity);
            pending_offs.push((n.time_sec + n.duration_sec, n.channel, n.note));
            note_idx += 1;
        }

        thread::sleep(Duration::from_millis(1));
    }

    println!("Done.");
    rt.shutdown();
}

fn cmd_midi_render(midi_path: &str, sound_path: &str, output: &str, use_sampler: bool) {
    let (notes, program_changes) = match parse_midi_file(midi_path) {
        Ok(v) => v,
        Err(e) => { eprintln!("MIDI parse error: {e}"); std::process::exit(1); }
    };

    let duration = notes.iter()
        .map(|n| n.time_sec + n.duration_sec)
        .fold(0.0f64, f64::max) + 2.0; // 2s tail

    println!("MIDI: {} notes, rendering {:.1}s", notes.len(), duration);

    let sample_rate = 44100u32;
    let buffer_size = 256u32;
    let backend_result = if use_sampler {
        moonlitt_engine::create_with_sampler(sound_path, sample_rate, buffer_size)
    } else {
        // Offline rendering uses highest quality (Sinc72 for SF2)
        moonlitt_engine::create_high_quality(sound_path, sample_rate, buffer_size)
    };
    let mut backend = match backend_result {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error loading {sound_path}: {e}");
            std::process::exit(1);
        }
    };

    // Send initial program changes
    let mut sent = std::collections::HashSet::new();
    for &(_, ch, prog) in &program_changes {
        if sent.insert((ch, prog)) {
            backend.program_change(ch, prog);
        }
    }

    let total_samples = (sample_rate as f64 * duration) as usize;
    let num_buffers = total_samples.div_ceil(buffer_size as usize);

    let mut all_left = Vec::with_capacity(total_samples);
    let mut all_right = Vec::with_capacity(total_samples);
    let mut left = vec![0.0f32; buffer_size as usize];
    let mut right = vec![0.0f32; buffer_size as usize];

    let mut note_idx = 0;
    let mut pending_offs: Vec<(f64, u8, u8)> = Vec::new();

    for buf_i in 0..num_buffers {
        let buf_start = buf_i as f64 * buffer_size as f64 / sample_rate as f64;
        let buf_end = buf_start + buffer_size as f64 / sample_rate as f64;

        // Note-offs
        pending_offs.retain(|&(off_time, ch, note)| {
            if off_time <= buf_end {
                backend.note_off(ch, note);
                false
            } else {
                true
            }
        });

        // Note-ons
        while note_idx < notes.len() && notes[note_idx].time_sec <= buf_end {
            let n = &notes[note_idx];
            backend.note_on(n.channel, n.note, n.velocity);
            pending_offs.push((n.time_sec + n.duration_sec, n.channel, n.note));
            note_idx += 1;
        }

        backend.render(&mut left, &mut right);
        all_left.extend_from_slice(&left);
        all_right.extend_from_slice(&right);
    }

    all_left.truncate(total_samples);
    all_right.truncate(total_samples);

    let peak = all_left.iter().chain(all_right.iter())
        .map(|s| s.abs()).fold(0.0f32, f32::max);

    match wav::write_wav(output, sample_rate, &all_left, &all_right) {
        Ok(()) => {
            println!("Rendered to {output}");
            println!("  Peak: {peak:.4}");
            println!("  Samples: {total_samples}");
        }
        Err(e) => eprintln!("Error: {e}"),
    }
}

fn cmd_presets(path: &str) {
    let backend = match moonlitt_engine::create(path, 44100, 256) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error loading {path}: {e}");
            std::process::exit(1);
        }
    };

    let presets = backend.presets();
    if presets.is_empty() {
        println!("No presets found.");
        return;
    }

    println!("{:<6} Name", "ID");
    println!("{}", "-".repeat(40));
    for p in &presets {
        println!("{:<6} {}", p.id, p.name);
    }
    println!("\nTotal: {} presets", presets.len());
}

// =============================================================================
// Insert effect spec parser
// =============================================================================
// Spec format: "type[:k=v,k=v,...]"
// Each effect type maps human-readable param names to numeric IDs to keep
// the CLI ergonomic without exposing trait param IDs to users.

fn build_insert(spec: &str, sample_rate: u32) -> Result<Box<dyn moonlitt_core::AudioBackend>, String> {
    let (kind, params_str) = match spec.split_once(':') {
        Some((k, p)) => (k.trim(), p.trim()),
        None => (spec.trim(), ""),
    };

    let pairs: Vec<(String, f64)> = if params_str.is_empty() {
        Vec::new()
    } else {
        params_str
            .split(',')
            .map(|kv| {
                let (k, v) = kv.split_once('=').ok_or_else(|| format!("expected k=v in '{kv}'"))?;
                let val: f64 = v.trim().parse().map_err(|e| format!("bad number '{v}': {e}"))?;
                Ok::<_, String>((k.trim().to_string(), val))
            })
            .collect::<Result<Vec<_>, _>>()?
    };

    use moonlitt_core::AudioBackend;

    match kind {
        "compressor" | "comp" => {
            let mut e = moonlitt_effects::Compressor::new(sample_rate);
            for (k, v) in &pairs {
                let id = match k.as_str() {
                    "threshold" => 0,
                    "ratio"     => 1,
                    "attack"    => 2,
                    "release"   => 3,
                    "knee"      => 4,
                    "makeup"    => 5,
                    "sc_hpf"    => 6,
                    "detect"    => 7,
                    "bypass"    => 8,
                    other => return Err(format!("compressor: unknown param '{other}'")),
                };
                e.set_param(id, *v);
            }
            Ok(Box::new(e))
        }
        "plate" | "dattorro" => {
            let mut e = moonlitt_effects::DattorroReverb::new(sample_rate);
            for (k, v) in &pairs {
                let id = match k.as_str() {
                    "predelay"  => 0,
                    "decay"     => 1,
                    "damping"   => 2,
                    "diffusion" => 3,
                    "wet_lp"    => 4,
                    "wet_hp"    => 5,
                    "width"     => 6,
                    "wet" | "mix" | "dry_wet" => 7,
                    "bypass"    => 8,
                    other => return Err(format!("plate: unknown param '{other}'")),
                };
                e.set_param(id, *v);
            }
            Ok(Box::new(e))
        }
        "freeverb" | "reverb" => {
            let mut e = moonlitt_effects::Reverb::new(sample_rate);
            for (k, v) in &pairs {
                let id = match k.as_str() {
                    "predelay"  => 0,
                    "room" | "room_size" => 1,
                    "damping"   => 2,
                    "diffusion" => 3,
                    "wet_lp"    => 4,
                    "wet_hp"    => 5,
                    "width"     => 6,
                    "wet" | "mix" | "dry_wet" => 7,
                    "bypass"    => 8,
                    other => return Err(format!("freeverb: unknown param '{other}'")),
                };
                e.set_param(id, *v);
            }
            Ok(Box::new(e))
        }
        other => Err(format!(
            "unknown effect type '{other}' (try: compressor, plate, freeverb)"
        )),
    }
}
