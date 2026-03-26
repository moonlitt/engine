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

    // First: dump sfizz's default state to understand the format
    match plugin.get_state() {
        Ok(state) => {
            eprintln!("Default state: {} bytes", state.len());
            eprintln!("Hex: {:02x?}", &state[..state.len().min(80)]);
            // Parse: first 8 bytes = version
            if state.len() >= 8 {
                let version = u64::from_le_bytes(state[0..8].try_into().unwrap());
                eprintln!("Version: {version}");
            }
            if state.len() >= 12 {
                let str_len = i32::from_le_bytes(state[8..12].try_into().unwrap());
                eprintln!("sfzFile length: {str_len}");
                if str_len > 0 && state.len() >= 12 + str_len as usize {
                    let s = String::from_utf8_lossy(&state[12..12+str_len as usize]);
                    eprintln!("sfzFile: '{s}'");
                }
            }
        }
        Err(e) => eprintln!("getState failed: {e}"),
    }

    // Use a valid SFZ with a real WAV sample
    let sfz_path = "/tmp/moonlitt_valid.sfz";
    if !std::path::Path::new(sfz_path).exists() {
        eprintln!("Test SFZ not found at {sfz_path}, skipping");
        return;
    }

    // Build our state and compare to default
    let our_state = moonlitt_vst3::stream::build_sfizz_state_v5(sfz_path);
    eprintln!("Our state: {} bytes", our_state.len());
    eprintln!("Our hex: {:02x?}", &our_state[..our_state.len().min(80)]);

    // Load via setState
    match plugin.load_sfizz_file(sfz_path) {
        Ok(()) => eprintln!("setState succeeded"),
        Err(e) => {
            eprintln!("setState failed: {e}");
            return;
        }
    }

    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];

    // Warm up: render many blocks + sleep to let sfizz finish async loading
    for _ in 0..100 {
        plugin.render(&mut left, &mut right).unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    for _ in 0..100 {
        plugin.render(&mut left, &mut right).unwrap();
    }

    // Play a note and render
    plugin.note_on(0, 60, 100);
    let mut max_sample = 0.0f32;

    for block in 0..64 {
        plugin.render(&mut left, &mut right).unwrap();
        let block_max = left.iter().chain(right.iter())
            .map(|s| s.abs())
            .fold(0.0f32, f32::max);
        if block_max > max_sample {
            max_sample = block_max;
        }
        if block < 4 || block_max > 0.001 {
            eprintln!("  block {block}: max={block_max:.6}");
        }
    }

    eprintln!("Peak after 64 blocks: {max_sample:.6}");

    plugin.note_off(0, 60);

    if max_sample > 0.001 {
        eprintln!("SUCCESS: sfizz produced audio via setState + SFZ!");
    } else {
        eprintln!("No audio — sfizz may need different loading mechanism");
        // Check: how many regions loaded?
        let param_count = plugin.param_count();
        eprintln!("Plugin has {param_count} params");
    }
}
