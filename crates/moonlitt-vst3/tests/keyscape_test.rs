use moonlitt_vst3::Vst3Host;

#[test]
fn test_keyscape_with_preset() {
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    
    let keyscape = plugins.iter().find(|p| p.name == "Keyscape");
    let keyscape = match keyscape {
        Some(k) => k,
        None => { println!("Keyscape not installed, skipping"); return; }
    };
    
    let mut plugin = host.load(keyscape).unwrap();
    println!("Keyscape loaded: {}", plugin.name());
    
    // List presets
    match plugin.presets() {
        Ok(presets) => {
            println!("Found {} presets:", presets.len());
            for (i, p) in presets.iter().take(20).enumerate() {
                println!("  [{i}] list={} idx={} \"{}\"", p.list_id, p.program_index, p.name);
            }
            
            // Try loading first preset
            if !presets.is_empty() {
                println!("\nLoading preset: \"{}\"", presets[0].name);
                match plugin.load_preset(presets[0].program_index) {
                    Ok(()) => println!("Preset loaded!"),
                    Err(e) => println!("Preset load failed: {e}"),
                }
            }
        }
        Err(e) => println!("No presets: {e}"),
    }
    
    // Play a chord and render
    plugin.note_on(0, 60, 100); // C4
    plugin.note_on(0, 64, 100); // E4
    plugin.note_on(0, 67, 100); // G4
    
    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];
    let mut max = 0.0f32;
    
    for block in 0..32 {
        plugin.render(&mut left, &mut right).unwrap();
        let block_max = left.iter().chain(right.iter())
            .map(|s| s.abs()).fold(0.0f32, f32::max);
        if block_max > max { max = block_max; }
        if block < 4 || block_max > 0.001 {
            println!("  block {block}: peak {block_max:.6}");
        }
    }
    
    println!("\nPeak amplitude: {max:.6}");
    if max > 0.001 {
        println!("\n✅ Keyscape produces audio via pure Rust VST3 hosting!");
    } else {
        println!("\n⚠ Silent — preset may not have loaded correctly");
    }
}
