//! MIDI event handling
//!
//! Converts MIDI events to VST3 IEventList format. Implements IEventList as
//! a Rust COM object so it can be passed directly to IAudioProcessor::process().

use vst3::Steinberg::Vst::{
    Event, Event_, Event__type0, IEventList, IEventListTrait, LegacyMIDICCOutEvent,
    NoteOffEvent, NoteOnEvent,
};
use vst3::Steinberg::{int32, tresult};
use vst3::Steinberg::{kResultFalse, kResultOk};
use vst3::{Class, ComWrapper};

/// A MIDI event with sample-accurate timing.
#[derive(Debug, Clone)]
pub struct MidiEvent {
    pub kind: MidiEventKind,
    pub sample_offset: i32,
}

/// The kind of MIDI event.
#[derive(Debug, Clone)]
pub enum MidiEventKind {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    CC {
        channel: u8,
        cc: u8,
        value: u8,
    },
    PitchBend {
        channel: u8,
        value: i16,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
}

/// Convert a slice of MidiEvents into VST3 Event structs.
fn midi_to_vst3_events(events: &[MidiEvent]) -> Vec<Event> {
    let mut out = Vec::with_capacity(events.len());

    for me in events {
        let sample_offset = me.sample_offset;

        match me.kind {
            MidiEventKind::NoteOn {
                channel,
                note,
                velocity,
            } => {
                let mut e = zeroed_event();
                e.busIndex = 0;
                e.sampleOffset = sample_offset;
                e.r#type = Event_::EventTypes_::kNoteOnEvent as u16;
                e.__field0 = Event__type0 {
                    noteOn: NoteOnEvent {
                        channel: channel as i16,
                        pitch: note as i16,
                        tuning: 0.0,
                        velocity: velocity as f32 / 127.0,
                        length: 0,
                        noteId: note as i32,
                    },
                };
                out.push(e);
            }
            MidiEventKind::NoteOff { channel, note } => {
                let mut e = zeroed_event();
                e.busIndex = 0;
                e.sampleOffset = sample_offset;
                e.r#type = Event_::EventTypes_::kNoteOffEvent as u16;
                e.__field0 = Event__type0 {
                    noteOff: NoteOffEvent {
                        channel: channel as i16,
                        pitch: note as i16,
                        velocity: 0.0,
                        noteId: note as i32,
                        tuning: 0.0,
                    },
                };
                out.push(e);
            }
            MidiEventKind::CC {
                channel,
                cc,
                value,
            } => {
                // VST3 has no direct CC event; use LegacyMIDICCOutEvent
                let mut e = zeroed_event();
                e.busIndex = 0;
                e.sampleOffset = sample_offset;
                e.r#type = Event_::EventTypes_::kLegacyMIDICCOutEvent as u16;
                e.__field0 = Event__type0 {
                    midiCCOut: LegacyMIDICCOutEvent {
                        controlNumber: cc,
                        channel: channel as i8,
                        value: value as i8,
                        value2: 0,
                    },
                };
                out.push(e);
            }
            MidiEventKind::PitchBend { channel, value } => {
                // Pitch bend as legacy MIDI (controlNumber=129 = kPitchBend)
                let mut e = zeroed_event();
                e.busIndex = 0;
                e.sampleOffset = sample_offset;
                e.r#type = Event_::EventTypes_::kLegacyMIDICCOutEvent as u16;
                e.__field0 = Event__type0 {
                    midiCCOut: LegacyMIDICCOutEvent {
                        controlNumber: 129, // kPitchBend
                        channel: channel as i8,
                        value: (value & 0x7F) as i8,
                        value2: ((value >> 7) & 0x7F) as i8,
                    },
                };
                out.push(e);
            }
            MidiEventKind::ProgramChange {
                channel,
                program,
            } => {
                // Program change as legacy MIDI (controlNumber=130 = kCtrlProgramChange)
                let mut e = zeroed_event();
                e.busIndex = 0;
                e.sampleOffset = sample_offset;
                e.r#type = Event_::EventTypes_::kLegacyMIDICCOutEvent as u16;
                e.__field0 = Event__type0 {
                    midiCCOut: LegacyMIDICCOutEvent {
                        controlNumber: 130,
                        channel: channel as i8,
                        value: program as i8,
                        value2: 0,
                    },
                };
                out.push(e);
            }
        }
    }

    out
}

/// Create a zeroed Event (all bytes zero).
fn zeroed_event() -> Event {
    unsafe { std::mem::zeroed() }
}

// ---------------------------------------------------------------------------
// IEventList COM implementation
// ---------------------------------------------------------------------------

/// A Rust-side IEventList that holds a Vec<Event>.
pub(crate) struct EventListImpl {
    events: Vec<Event>,
}

impl EventListImpl {
    fn from_midi_events(midi_events: &[MidiEvent]) -> Self {
        Self {
            events: midi_to_vst3_events(midi_events),
        }
    }
}

impl Class for EventListImpl {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for EventListImpl {
    unsafe fn getEventCount(&self) -> int32 {
        self.events.len() as int32
    }

    unsafe fn getEvent(&self, index: int32, e: *mut Event) -> tresult {
        if let Some(event) = self.events.get(index as usize) {
            std::ptr::write(e, *event);
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn addEvent(&self, _e: *mut Event) -> tresult {
        // This implementation is read-only (for input events).
        // Plugins should not call addEvent on input event lists.
        kResultFalse
    }
}

/// Create an IEventList COM wrapper from MIDI events.
pub(crate) fn create_event_list(events: &[MidiEvent]) -> ComWrapper<EventListImpl> {
    ComWrapper::new(EventListImpl::from_midi_events(events))
}

