//! MIDI event handling
//!
//! Converts MIDI events to VST3 IEventList format. Implements IEventList as
//! a Rust COM object so it can be passed directly to IAudioProcessor::process().

use vst3::Steinberg::Vst::{
    Event, Event_, Event__type0, IEventList, IEventListTrait, LegacyMIDICCOutEvent, NoteOffEvent,
    NoteOnEvent,
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
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8 },
    CC { channel: u8, cc: u8, value: u8 },
    PitchBend { channel: u8, value: i16 },
    ProgramChange { channel: u8, program: u8 },
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
            MidiEventKind::CC { channel, cc, value } => {
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
                // Convert signed i16 (-8192..8191) to unsigned 14-bit (0..16383, center=8192)
                let unsigned = (value as i32 + 8192).clamp(0, 16383) as u16;
                let mut e = zeroed_event();
                e.busIndex = 0;
                e.sampleOffset = sample_offset;
                e.r#type = Event_::EventTypes_::kLegacyMIDICCOutEvent as u16;
                e.__field0 = Event__type0 {
                    midiCCOut: LegacyMIDICCOutEvent {
                        controlNumber: 129, // kPitchBend
                        channel: channel as i8,
                        value: (unsigned & 0x7F) as i8,         // LSB
                        value2: ((unsigned >> 7) & 0x7F) as i8, // MSB
                    },
                };
                out.push(e);
            }
            MidiEventKind::ProgramChange { channel, program } => {
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

// ---------------------------------------------------------------------------
// Output IEventList (plugin writes, host drains)
// ---------------------------------------------------------------------------

use std::cell::UnsafeCell;

/// IEventList backing for `ProcessData::outputEvents`. Plug-ins like
/// arpeggiators or generative synths emit MIDI events back to the host
/// through this — the host's job is to drain them after each process()
/// and forward to whatever downstream consumer wants the MIDI stream.
///
/// UnsafeCell is sound here because plug-ins call into a single
/// IEventList serially during process(), and we never call into it
/// concurrently with the audio thread (drain happens after process()
/// returns).
pub(crate) struct OutputEventListImpl {
    events: UnsafeCell<Vec<Event>>,
}

impl Class for OutputEventListImpl {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for OutputEventListImpl {
    unsafe fn getEventCount(&self) -> int32 {
        (*self.events.get()).len() as int32
    }

    unsafe fn getEvent(&self, index: int32, e: *mut Event) -> tresult {
        let events: &Vec<Event> = &*self.events.get();
        match events.get(index as usize) {
            Some(event) => {
                std::ptr::write(e, *event);
                kResultOk
            }
            None => kResultFalse,
        }
    }

    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        if e.is_null() {
            return kResultFalse;
        }
        (*self.events.get()).push(*e);
        kResultOk
    }
}

/// Allocate a fresh writable IEventList for the plug-in to fill.
pub(crate) fn new_output_event_list() -> ComWrapper<OutputEventListImpl> {
    ComWrapper::new(OutputEventListImpl {
        events: UnsafeCell::new(Vec::new()),
    })
}

/// Drain the events the plug-in wrote during the last process() call,
/// converting them back into our public MidiEvent shape. Note events
/// and the legacy MIDI CC subset are translated; other event types
/// (note expression, chord, etc.) are dropped because we don't surface
/// them on the public API yet.
pub(crate) fn drain_output_events(out: &ComWrapper<OutputEventListImpl>) -> Vec<MidiEvent> {
    let raw: Vec<Event> = unsafe { std::mem::take(&mut *out.events.get()) };
    let mut result = Vec::with_capacity(raw.len());
    for e in raw {
        if let Some(me) = vst3_event_to_midi(&e) {
            result.push(me);
        }
    }
    result
}

fn vst3_event_to_midi(e: &Event) -> Option<MidiEvent> {
    let sample_offset = e.sampleOffset;
    match e.r#type as u32 {
        t if t == Event_::EventTypes_::kNoteOnEvent => {
            let n = unsafe { e.__field0.noteOn };
            Some(MidiEvent {
                kind: MidiEventKind::NoteOn {
                    channel: n.channel as u8,
                    note: n.pitch as u8,
                    velocity: (n.velocity * 127.0).round().clamp(0.0, 127.0) as u8,
                },
                sample_offset,
            })
        }
        t if t == Event_::EventTypes_::kNoteOffEvent => {
            let n = unsafe { e.__field0.noteOff };
            Some(MidiEvent {
                kind: MidiEventKind::NoteOff {
                    channel: n.channel as u8,
                    note: n.pitch as u8,
                },
                sample_offset,
            })
        }
        t if t == Event_::EventTypes_::kLegacyMIDICCOutEvent => {
            let m = unsafe { e.__field0.midiCCOut };
            match m.controlNumber {
                129 => Some(MidiEvent {
                    kind: MidiEventKind::PitchBend {
                        channel: m.channel as u8,
                        // Reconstruct signed 14-bit from LSB+MSB.
                        value: {
                            let lsb = (m.value as u8 & 0x7F) as i32;
                            let msb = (m.value2 as u8 & 0x7F) as i32;
                            ((msb << 7) | lsb) - 8192
                        } as i16,
                    },
                    sample_offset,
                }),
                130 => Some(MidiEvent {
                    kind: MidiEventKind::ProgramChange {
                        channel: m.channel as u8,
                        program: m.value as u8,
                    },
                    sample_offset,
                }),
                cc if cc < 128 => Some(MidiEvent {
                    kind: MidiEventKind::CC {
                        channel: m.channel as u8,
                        cc,
                        value: m.value as u8,
                    },
                    sample_offset,
                }),
                _ => None,
            }
        }
        _ => None,
    }
}
