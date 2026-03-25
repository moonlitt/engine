mod wav;

use clap::{Parser, Subcommand};
use moonlitt_engine::engine::Engine;

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
        Commands::MidiDevices => cmd_midi_devices(),
    }
}

fn cmd_scan(_dir: Option<String>) {
    let engine = Engine::new(44100, 256);
    let plugins = engine.scan_plugins();

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
    let mut engine = Engine::new(44100, 256);
    match engine.load(path) {
        Ok(()) => {
            if let Some(info) = engine.backend_info() {
                println!("Backend:    {}", info.name);
                println!("Type:       {:?}", info.backend_type);
                println!(
                    "Extensions: {}",
                    info.extensions.join(", ")
                );
            }
            let presets = engine.presets();
            println!("Presets:    {}", presets.len());
        }
        Err(e) => {
            eprintln!("Error loading {path}: {e}");
            std::process::exit(1);
        }
    }
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
    let mut engine = Engine::new(sample_rate, buffer_size);
    if let Err(e) = engine.load(path) {
        eprintln!("Error loading {path}: {e}");
        std::process::exit(1);
    }

    let total_samples = (sample_rate as f32 * duration) as usize;
    let num_buffers = total_samples.div_ceil(buffer_size as usize);

    // Note-on duration: 80% of total, then note-off for tail
    let note_off_buffer = (num_buffers as f32 * 0.8) as usize;

    let mut all_left = Vec::with_capacity(total_samples);
    let mut all_right = Vec::with_capacity(total_samples);

    let mut left = vec![0.0f32; buffer_size as usize];
    let mut right = vec![0.0f32; buffer_size as usize];

    engine.note_on(0, note, velocity);

    for i in 0..num_buffers {
        if i == note_off_buffer {
            engine.note_off(0, note);
        }
        engine.render(&mut left, &mut right);
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
    use moonlitt_runtime::Runtime;
    use std::thread;
    use std::time::Duration;

    let mut engine = Engine::new(sample_rate, buffer_size);
    if let Err(e) = engine.load(path) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }

    let mut rt = match Runtime::new(engine) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Audio error: {e}");
            std::process::exit(1);
        }
    };

    rt.start().unwrap();
    println!("Playing note {note} (velocity {velocity}) for {duration}s...");

    rt.note_on(0, note, velocity);
    thread::sleep(Duration::from_secs_f32(duration * 0.8));
    rt.note_off(0, note);
    thread::sleep(Duration::from_secs_f32(duration * 0.2));

    println!("Done.");
    rt.shutdown();
}

fn cmd_live(path: &str) {
    use moonlitt_runtime::Runtime;
    use std::time::Duration;

    let mut engine = Engine::new(44100, 256);
    if let Err(e) = engine.load(path) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }

    let mut rt = match Runtime::new(engine) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Audio error: {e}");
            std::process::exit(1);
        }
    };

    rt.start().unwrap();
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
    match moonlitt_runtime::Runtime::list_midi_inputs() {
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

fn cmd_presets(path: &str) {
    let mut engine = Engine::new(44100, 256);
    if let Err(e) = engine.load(path) {
        eprintln!("Error loading {path}: {e}");
        std::process::exit(1);
    }

    let presets = engine.presets();
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
