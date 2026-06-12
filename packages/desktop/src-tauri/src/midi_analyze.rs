//! Pure MIDI metadata extraction.
//!
//! Mirrors the logic that previously lived in `crates/moonlitt-node/src/engine.rs::analyze_midi`,
//! ported to direct Rust types so the Tauri frontend gets the same per-channel hints
//! (TrackName, first ProgramChange), tempo, time signature, and length-in-bars.

use std::collections::BTreeMap;

use midly::{MetaMessage, MidiMessage, Smf, TrackEventKind};
use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MidiChannelInfo {
    /// 0-based MIDI channel (wire format).
    pub channel: u8,
    /// 1-based human number (1..=16).
    pub display_number: u8,
    /// TrackName meta event from the MIDI track that owns this channel's notes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_name: Option<String>,
    /// First Program Change observed on this channel (0..=127), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<u8>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MidiInfo {
    pub channels: Vec<MidiChannelInfo>,
    pub track_count: u32,
    pub length_bars: f64,
    pub tempo_bpm: Option<f64>,
    /// `[numerator, denominator]` if a TimeSignature meta is present.
    pub time_signature: Option<[u8; 2]>,
}

pub fn analyze(path: &str) -> Result<MidiInfo, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    let smf = Smf::parse(&bytes).map_err(|e| format!("parse {path}: {e}"))?;

    let ticks_per_beat: u32 = match smf.header.timing {
        midly::Timing::Metrical(t) => t.as_int() as u32,
        midly::Timing::Timecode(_, _) => 480,
    };

    struct ChState {
        has_notes: bool,
        first_program: Option<u8>,
        track_name_from: Option<String>,
    }
    let mut chans: BTreeMap<u8, ChState> = BTreeMap::new();

    let mut tempo_bpm: Option<f64> = None;
    let mut time_signature: Option<(u8, u8)> = None;
    let mut max_ticks: u64 = 0;

    for track in &smf.tracks {
        // Pull out a clean TrackName once, attribute to channels emitting notes here.
        let mut track_name: Option<String> = None;
        for event in track {
            if let TrackEventKind::Meta(MetaMessage::TrackName(bytes)) = event.kind {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    let cleaned: String = s
                        .chars()
                        .filter(|c| !c.is_control())
                        .collect::<String>()
                        .trim()
                        .to_string();
                    if !cleaned.is_empty() {
                        track_name = Some(cleaned);
                        break;
                    }
                }
            }
        }

        let mut t: u64 = 0;
        for event in track {
            t += event.delta.as_int() as u64;
            match event.kind {
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    let entry = chans.entry(ch).or_insert(ChState {
                        has_notes: false,
                        first_program: None,
                        track_name_from: None,
                    });
                    match message {
                        MidiMessage::NoteOn { vel, .. } if vel.as_int() > 0 => {
                            entry.has_notes = true;
                            if entry.track_name_from.is_none() {
                                entry.track_name_from = track_name.clone();
                            }
                        }
                        MidiMessage::ProgramChange { program } => {
                            if entry.first_program.is_none() {
                                entry.first_program = Some(program.as_int());
                            }
                        }
                        _ => {}
                    }
                }
                TrackEventKind::Meta(MetaMessage::Tempo(us_per_beat)) => {
                    if tempo_bpm.is_none() {
                        let us = us_per_beat.as_int() as f64;
                        if us > 0.0 {
                            tempo_bpm = Some(60_000_000.0 / us);
                        }
                    }
                }
                TrackEventKind::Meta(MetaMessage::TimeSignature(num, den_pow, _, _)) => {
                    if time_signature.is_none() {
                        time_signature = Some((num, 1u8 << den_pow));
                    }
                }
                _ => {}
            }
        }
        if t > max_ticks {
            max_ticks = t;
        }
    }

    let beats_per_bar = time_signature
        .map(|(n, _)| n as f64)
        .unwrap_or(4.0)
        .max(1.0);
    let length_bars = if ticks_per_beat == 0 {
        0.0
    } else {
        max_ticks as f64 / (ticks_per_beat as f64 * beats_per_bar)
    };

    let channels = chans
        .into_iter()
        .filter(|(_, s)| s.has_notes)
        .map(|(ch, s)| MidiChannelInfo {
            channel: ch,
            display_number: ch + 1,
            track_name: s.track_name_from,
            program: s.first_program,
        })
        .collect();

    Ok(MidiInfo {
        channels,
        track_count: smf.tracks.len() as u32,
        length_bars,
        tempo_bpm,
        time_signature: time_signature.map(|(n, d)| [n, d]),
    })
}
