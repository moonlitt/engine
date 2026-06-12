//! Offline bounce — render a session + MIDI clip straight to a WAV
//! file on a **fresh engine instance**, fully independent of any live
//! runtime (the DAW keeps playing while a bounce runs).
//!
//! The pipeline mirrors the live audio thread exactly: restore the
//! session (plugins loaded, patch states replayed, sample streamers
//! warmed up), then drive the same `Sequencer` → `dispatch_to_mixer` →
//! `Mixer::render` chain the processor runs in real time — just as
//! fast as the CPU allows instead of at wall-clock speed.

use crate::processor::dispatch_to_mixer;
use crate::sequencer::Sequencer;
use crate::Session;

/// What a finished bounce reports back to the caller/UI.
#[derive(Debug, Clone, Copy)]
pub struct RenderStats {
    /// Frames written (per channel).
    pub frames: usize,
    /// Rendered audio length in seconds.
    pub duration_secs: f64,
    /// Absolute sample peak across both channels (1.0 = full scale).
    pub peak: f32,
}

/// Ring-out time appended after the last MIDI event so releases and
/// reverb tails aren't cut off.
const TAIL_SECONDS: f64 = 2.0;

/// Hard safety cap so a pathological clip can't fill the disk.
const MAX_RENDER_SECONDS: usize = 60 * 30;

/// Render `session` playing `midi_path` into `output_path` (stereo
/// 32-bit float WAV at the session's sample rate).
pub fn render_to_wav(
    session: &Session,
    midi_path: &str,
    output_path: &str,
    buffer_size: usize,
) -> Result<RenderStats, String> {
    if buffer_size == 0 {
        return Err("buffer_size must be > 0".into());
    }

    let restored = session
        .restore(buffer_size)
        .map_err(|e| format!("restore session: {e}"))?;
    let mut mixer = restored.mixer;
    let sample_rate = session.sample_rate;

    let mut seq =
        Sequencer::from_file(midi_path).map_err(|e| format!("load MIDI '{midi_path}': {e}"))?;
    seq.play();
    let tempo_override = session.transport.tempo_override_bpm;

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(output_path, spec)
        .map_err(|e| format!("create '{output_path}': {e}"))?;

    let mut left = vec![0.0f32; buffer_size];
    let mut right = vec![0.0f32; buffer_size];
    let mut events: Vec<moonlitt_core::AudioEvent> = Vec::with_capacity(64);

    let tail_frames = (TAIL_SECONDS * sample_rate as f64) as usize;
    let max_frames = sample_rate as usize * MAX_RENDER_SECONDS;
    let mut frames = 0usize;
    let mut tail_frames_done = 0usize;
    let mut peak = 0.0f32;

    loop {
        events.clear();
        seq.advance(buffer_size, sample_rate, &mut events, tempo_override, false);
        for &event in &events {
            dispatch_to_mixer(&mut mixer, event);
        }

        mixer.render(&mut left, &mut right);
        for i in 0..buffer_size {
            peak = peak.max(left[i].abs()).max(right[i].abs());
            writer
                .write_sample(left[i])
                .and_then(|()| writer.write_sample(right[i]))
                .map_err(|e| format!("write '{output_path}': {e}"))?;
        }
        frames += buffer_size;

        if seq.is_finished() {
            tail_frames_done += buffer_size;
            if tail_frames_done >= tail_frames {
                break;
            }
        }
        if frames >= max_frames {
            break;
        }
    }

    writer
        .finalize()
        .map_err(|e| format!("finalize '{output_path}': {e}"))?;

    Ok(RenderStats {
        frames,
        duration_secs: frames as f64 / sample_rate as f64,
        peak,
    })
}
