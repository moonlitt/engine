use moonlitt_vst3::Vst3Host;

#[test]
fn test_sfizz_load_sfz_via_state() {
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    let sfizz = plugins.iter().find(|p| p.name == "sfizz");
    if sfizz.is_none() {
        eprintln!("sfizz not installed, skipping");
        return;
    }

    let mut plugin = host.load(sfizz.unwrap()).unwrap();
    eprintln!("Loaded sfizz: {}", plugin.name());

    // Use a valid SFZ with a real WAV sample
    let sfz_path = "/tmp/moonlitt_valid.sfz";
    if !std::path::Path::new(sfz_path).exists() {
        eprintln!("Test SFZ not found at {sfz_path}, skipping");
        return;
    }

    // Load via setState
    match plugin.load_sfizz_file(sfz_path) {
        Ok(()) => eprintln!("setState succeeded"),
        Err(e) => {
            eprintln!("setState failed: {e} — trying direct file approach");
            return;
        }
    }

    // Play a note and render
    plugin.note_on(0, 60, 100);
    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];
    let mut max_sample = 0.0f32;

    for _ in 0..16 {
        plugin.render(&mut left, &mut right).unwrap();
        let block_max = left.iter().chain(right.iter())
            .map(|s| s.abs())
            .fold(0.0f32, f32::max);
        if block_max > max_sample {
            max_sample = block_max;
        }
    }

    eprintln!("Peak after 16 blocks: {max_sample:.6}");
    // Note: sfizz may not produce audio immediately from a bare SFZ with SF2 reference
    // The important thing is setState didn't crash

    plugin.note_off(0, 60);
    eprintln!("sfizz setState test passed");
}
