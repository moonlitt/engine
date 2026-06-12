//! MIDI event handling for CLAP.
//!
//! Builds clap_input_events from MIDI events. CLAP uses `clap_event_note`
//! for note on/off and `clap_event_midi` for raw MIDI (CC, pitch bend, etc.).

use clap_sys::events::{
    clap_event_header, clap_event_midi, clap_event_note, clap_input_events, clap_output_events,
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_MIDI, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON,
};
use std::ffi::c_void;
use std::mem;

/// A MIDI event with sample-accurate timing.
#[derive(Debug, Clone)]
pub struct MidiEvent {
    pub kind: MidiEventKind,
    /// Sample offset within the current buffer. Uses i32 for consistency
    /// with VST3's native type; converted to u32 internally for CLAP.
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

// ---------------------------------------------------------------------------
// Input event list — passed to plugin.process()
// ---------------------------------------------------------------------------

/// Holds CLAP events for passing to plugin.process().
///
/// Events are stored as raw bytes because CLAP events are variable-sized
/// structs that share a common header. We store each event's byte offset
/// so the `get` callback can return a pointer into the buffer.
pub(crate) struct InputEventList {
    /// Raw event data — each event is a repr(C) struct starting with clap_event_header.
    buffer: Vec<u8>,
    /// Byte offsets into `buffer` for each event.
    offsets: Vec<usize>,
    /// The clap_input_events vtable (self-referential, set after construction).
    input_events: clap_input_events,
}

impl InputEventList {
    /// Build an InputEventList from MIDI events.
    pub fn from_midi_events(events: &[MidiEvent]) -> Box<Self> {
        // Pre-allocate generously
        let mut buffer = Vec::with_capacity(events.len() * mem::size_of::<clap_event_note>());
        let mut offsets = Vec::with_capacity(events.len());

        for ev in events {
            match ev.kind {
                MidiEventKind::NoteOn {
                    channel,
                    note,
                    velocity,
                } => {
                    let offset = buffer.len();
                    offsets.push(offset);

                    let event = clap_event_note {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_note>() as u32,
                            time: ev.sample_offset.max(0) as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_ON,
                            flags: 0,
                        },
                        note_id: -1,
                        port_index: 0,
                        channel: channel as i16,
                        key: note as i16,
                        velocity: velocity as f64 / 127.0,
                    };

                    let bytes = unsafe {
                        std::slice::from_raw_parts(
                            &event as *const _ as *const u8,
                            mem::size_of::<clap_event_note>(),
                        )
                    };
                    buffer.extend_from_slice(bytes);
                }
                MidiEventKind::NoteOff { channel, note } => {
                    let offset = buffer.len();
                    offsets.push(offset);

                    let event = clap_event_note {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_note>() as u32,
                            time: ev.sample_offset.max(0) as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_OFF,
                            flags: 0,
                        },
                        note_id: -1,
                        port_index: 0,
                        channel: channel as i16,
                        key: note as i16,
                        velocity: 0.0,
                    };

                    let bytes = unsafe {
                        std::slice::from_raw_parts(
                            &event as *const _ as *const u8,
                            mem::size_of::<clap_event_note>(),
                        )
                    };
                    buffer.extend_from_slice(bytes);
                }
                MidiEventKind::CC { channel, cc, value } => {
                    let offset = buffer.len();
                    offsets.push(offset);

                    let event = clap_event_midi {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_midi>() as u32,
                            time: ev.sample_offset.max(0) as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_MIDI,
                            flags: 0,
                        },
                        port_index: 0,
                        data: [0xB0 | (channel & 0x0F), cc, value],
                    };

                    let bytes = unsafe {
                        std::slice::from_raw_parts(
                            &event as *const _ as *const u8,
                            mem::size_of::<clap_event_midi>(),
                        )
                    };
                    buffer.extend_from_slice(bytes);
                }
                MidiEventKind::PitchBend { channel, value } => {
                    let offset = buffer.len();
                    offsets.push(offset);

                    // Convert signed i16 (-8192..8191) to unsigned 14-bit (0..16383)
                    let unsigned = (value as i32 + 8192) as u16;
                    let lsb = (unsigned & 0x7F) as u8;
                    let msb = ((unsigned >> 7) & 0x7F) as u8;

                    let event = clap_event_midi {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_midi>() as u32,
                            time: ev.sample_offset.max(0) as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_MIDI,
                            flags: 0,
                        },
                        port_index: 0,
                        data: [0xE0 | (channel & 0x0F), lsb, msb],
                    };

                    let bytes = unsafe {
                        std::slice::from_raw_parts(
                            &event as *const _ as *const u8,
                            mem::size_of::<clap_event_midi>(),
                        )
                    };
                    buffer.extend_from_slice(bytes);
                }
                MidiEventKind::ProgramChange { channel, program } => {
                    let offset = buffer.len();
                    offsets.push(offset);

                    let event = clap_event_midi {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_midi>() as u32,
                            time: ev.sample_offset.max(0) as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_MIDI,
                            flags: 0,
                        },
                        port_index: 0,
                        data: [0xC0 | (channel & 0x0F), program, 0],
                    };

                    let bytes = unsafe {
                        std::slice::from_raw_parts(
                            &event as *const _ as *const u8,
                            mem::size_of::<clap_event_midi>(),
                        )
                    };
                    buffer.extend_from_slice(bytes);
                }
            }
        }

        let mut list = Box::new(Self {
            buffer,
            offsets,
            input_events: clap_input_events {
                ctx: std::ptr::null_mut(),
                size: Some(input_events_size),
                get: Some(input_events_get),
            },
        });

        // Set ctx to point at ourselves so the callbacks can access our data
        list.input_events.ctx = &*list as *const Self as *mut c_void;
        list
    }

    /// Get a pointer to the clap_input_events struct.
    pub fn as_ptr(&self) -> *const clap_input_events {
        &self.input_events as *const clap_input_events
    }
}

unsafe extern "C" fn input_events_size(list: *const clap_input_events) -> u32 {
    let ctx = (*list).ctx as *const InputEventList;
    (*ctx).offsets.len() as u32
}

unsafe extern "C" fn input_events_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    let ctx = (*list).ctx as *const InputEventList;
    let event_list = &*ctx;

    if let Some(&offset) = event_list.offsets.get(index as usize) {
        event_list.buffer.as_ptr().add(offset) as *const clap_event_header
    } else {
        std::ptr::null()
    }
}

// ---------------------------------------------------------------------------
// Output event list (no-op — we don't read plugin output events)
// ---------------------------------------------------------------------------

/// A no-op output event list. Plugins may push events here but we ignore them.
pub(crate) struct OutputEventList {
    output_events: clap_output_events,
}

impl OutputEventList {
    pub fn new() -> Self {
        Self {
            output_events: clap_output_events {
                ctx: std::ptr::null_mut(),
                try_push: Some(output_events_try_push),
            },
        }
    }

    pub fn as_ptr(&self) -> *const clap_output_events {
        &self.output_events as *const clap_output_events
    }
}

unsafe extern "C" fn output_events_try_push(
    _list: *const clap_output_events,
    _event: *const clap_event_header,
) -> bool {
    // Accept but ignore all output events
    true
}
