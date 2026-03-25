use moonlitt_runtime::Runtime;

#[test]
fn list_midi_devices() {
    // Should not crash, even if no devices connected
    let result = Runtime::list_midi_inputs();
    match result {
        Ok(devices) => println!("Found {} MIDI devices", devices.len()),
        Err(e) => println!("MIDI not available: {e}"),
    }
}
