//! IMidiMapping integration.
//!
//! When the plug-in's controller exposes `IMidiMapping`, it can tell the
//! host which `ParamID` a particular MIDI controller (CC, pitch bend,
//! aftertouch) on a given channel and bus is bound to. The host's job
//! is then to translate incoming MIDI controller events into parameter
//! changes routed through `ProcessData::inputParameterChanges` instead
//! of leaving them as raw MIDI bytes in the event list.
//!
//! This matters because some plug-ins only react to the parameter-changes
//! path -- they don't process MIDI CC events directly. Without this
//! translation, MIDI controller messages from a real keyboard or a
//! sequencer end up silently dropped on those plug-ins.
//!
//! The mapping query is allowed to fail per-CC (kResultFalse means "this
//! CC is unmapped"); in that case the original event flows through as a
//! raw `LegacyMIDICCOutEvent`, preserving compatibility with plug-ins
//! that *do* read raw CCs.

use vst3::Steinberg::Vst::{
    ControllerNumbers_::kPitchBend, IMidiMapping, IMidiMappingTrait, ParamID,
};
use vst3::Steinberg::kResultOk;
use vst3::ComPtr;

use crate::component_handler::PendingParam;
use crate::events::{MidiEvent, MidiEventKind};

/// Result of splitting a batch of MIDI events through the plug-in's
/// IMidiMapping. Mapped events become parameter changes; the rest stay
/// in the event stream so the plug-in can consume them as raw MIDI.
pub(crate) struct MappedEvents {
    pub events: Vec<MidiEvent>,
    pub params: Vec<PendingParam>,
}

/// Split incoming MIDI events: route mapped CCs/pitch-bend/aftertouch
/// to parameter changes (normalized to 0.0..=1.0) and leave note events
/// and unmapped controllers in the raw event stream.
pub(crate) fn split_for_param_routing(
    mapping: Option<&ComPtr<IMidiMapping>>,
    bus_index: i32,
    raw: Vec<MidiEvent>,
) -> MappedEvents {
    let Some(mapping) = mapping else {
        return MappedEvents {
            events: raw,
            params: Vec::new(),
        };
    };

    let mut events: Vec<MidiEvent> = Vec::with_capacity(raw.len());
    let mut params: Vec<PendingParam> = Vec::new();

    for me in raw {
        match me.kind {
            MidiEventKind::CC { channel, cc, value } => {
                if let Some(p) = lookup_cc_param(
                    mapping,
                    bus_index,
                    channel as i16,
                    cc as i16,
                    value as f64 / 127.0,
                ) {
                    params.push(p);
                } else {
                    events.push(me);
                }
            }
            MidiEventKind::PitchBend { channel, value } => {
                // Pitch bend is 14-bit signed, center 0. Normalize to 0..1
                // by shifting to unsigned and dividing by full range.
                let normalized = (value as f64 + 8192.0) / 16383.0;
                let normalized = normalized.clamp(0.0, 1.0);
                if let Some(p) = lookup_cc_param(
                    mapping,
                    bus_index,
                    channel as i16,
                    kPitchBend as i16,
                    normalized,
                ) {
                    params.push(p);
                } else {
                    events.push(me);
                }
            }
            // Aftertouch isn't currently emitted by our MIDI front-end,
            // but we already plumb the lookup so adding it later is a
            // one-line change. Note/program-change pass through unchanged.
            _ => events.push(me),
        }
    }

    MappedEvents { events, params }
}

fn lookup_cc_param(
    mapping: &ComPtr<IMidiMapping>,
    bus_index: i32,
    channel: i16,
    midi_controller: i16,
    normalized_value: f64,
) -> Option<PendingParam> {
    let mut id: ParamID = 0;
    let r = unsafe {
        mapping.getMidiControllerAssignment(bus_index, channel, midi_controller, &mut id)
    };
    if r == kResultOk {
        Some(PendingParam {
            id,
            value: normalized_value,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    //! Behavior-level tests for the mapper. We don't have a fake
    //! IMidiMapping here (it would require constructing a COM vtable),
    //! so we exercise only the no-mapping passthrough; the round-trip
    //! through a real plug-in is covered by the integration tests.
    use super::*;

    #[test]
    fn no_mapping_passes_events_unchanged() {
        let raw = vec![
            MidiEvent {
                kind: MidiEventKind::CC { channel: 0, cc: 7, value: 100 },
                sample_offset: 0,
            },
            MidiEvent {
                kind: MidiEventKind::NoteOn { channel: 0, note: 60, velocity: 100 },
                sample_offset: 0,
            },
        ];
        let out = split_for_param_routing(None, 0, raw.clone());
        assert!(out.params.is_empty());
        assert_eq!(out.events.len(), 2);
    }

    #[test]
    fn no_mapping_passes_pitch_bend() {
        let raw = vec![MidiEvent {
            kind: MidiEventKind::PitchBend { channel: 0, value: 4096 },
            sample_offset: 0,
        }];
        let out = split_for_param_routing(None, 0, raw);
        assert!(out.params.is_empty());
        assert_eq!(out.events.len(), 1);
    }

    /// Note/program-change events should pass through unchanged regardless
    /// of mapping availability — IMidiMapping covers controllers, not notes.
    #[test]
    fn unmapped_program_change_is_not_split() {
        let raw = vec![MidiEvent {
            kind: MidiEventKind::ProgramChange { channel: 0, program: 5 },
            sample_offset: 0,
        }];
        let out = split_for_param_routing(None, 0, raw);
        assert!(out.params.is_empty());
        assert_eq!(out.events.len(), 1);
    }
}
