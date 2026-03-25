use crate::event::AudioEvent;
use rtrb::Producer;

/// Information about an available MIDI input device.
pub struct MidiDeviceInfo {
    pub id: usize,
    pub name: String,
}

/// An active MIDI input connection.
/// Dropping this struct disconnects the MIDI device.
pub(crate) struct MidiInputConnection {
    _connection: midir::MidiInputConnection<()>,
}

impl MidiInputConnection {
    /// List available MIDI input devices.
    pub fn list_devices() -> Result<Vec<MidiDeviceInfo>, String> {
        let midi_in = midir::MidiInput::new("moonlitt").map_err(|e| e.to_string())?;
        Ok(midi_in
            .ports()
            .iter()
            .enumerate()
            .map(|(i, port)| MidiDeviceInfo {
                id: i,
                name: midi_in.port_name(port).unwrap_or_default(),
            })
            .collect())
    }

    /// Connect to a MIDI device by ID and feed events into the producer.
    pub fn connect(device_id: usize, mut producer: Producer<AudioEvent>) -> Result<Self, String> {
        let midi_in = midir::MidiInput::new("moonlitt").map_err(|e| e.to_string())?;
        let ports = midi_in.ports();
        let port = ports.get(device_id).ok_or("invalid MIDI device ID")?;

        let connection = midi_in
            .connect(
                port,
                "moonlitt-input",
                move |_timestamp, message, _| {
                    if let Some(event) = parse_midi_message(message) {
                        let _ = producer.push(event);
                    }
                },
                (),
            )
            .map_err(|e| e.to_string())?;

        Ok(Self {
            _connection: connection,
        })
    }
}

fn parse_midi_message(msg: &[u8]) -> Option<AudioEvent> {
    if msg.is_empty() {
        return None;
    }
    let status = msg[0] & 0xF0;
    let channel = msg[0] & 0x0F;
    match status {
        0x90 if msg.len() >= 3 && msg[2] > 0 => Some(AudioEvent::NoteOn {
            channel,
            note: msg[1],
            velocity: msg[2],
        }),
        0x90 if msg.len() >= 3 => Some(AudioEvent::NoteOff {
            channel,
            note: msg[1],
            velocity: 0,
        }),
        0x80 if msg.len() >= 3 => Some(AudioEvent::NoteOff {
            channel,
            note: msg[1],
            velocity: msg[2],
        }),
        0xB0 if msg.len() >= 3 => Some(AudioEvent::CC {
            channel,
            cc: msg[1],
            value: msg[2],
        }),
        0xE0 if msg.len() >= 3 => {
            let value = ((msg[2] as i16) << 7 | msg[1] as i16) - 8192;
            Some(AudioEvent::PitchBend { channel, value })
        }
        0xC0 if msg.len() >= 2 => Some(AudioEvent::ProgramChange {
            channel,
            program: msg[1],
        }),
        _ => None,
    }
}
