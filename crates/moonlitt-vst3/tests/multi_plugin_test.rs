use moonlitt_vst3::{Vst3Host, MidiEvent, MidiEventKind};

#[test]
fn test_all_available_plugins() {
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    
    println!("\n=== Moonlitt Multi-Plugin Compatibility Test ===\n");
    
    let mut passed = 0;
    let mut failed = 0;
    
    for info in &plugins {
        print!("{:<25} ", info.name);
        
        match host.load(info) {
            Ok(mut plugin) => {
                // Send a note
                plugin.note_on(0, 60, 100);
                
                // Render a few blocks
                let mut left = vec![0.0f32; 256];
                let mut right = vec![0.0f32; 256];
                let mut max = 0.0f32;
                
                for _ in 0..8 {
                    match plugin.render(&mut left, &mut right) {
                        Ok(()) => {
                            for s in left.iter().chain(right.iter()) {
                                max = max.max(s.abs());
                            }
                        }
                        Err(e) => {
                            println!("RENDER FAIL: {e}");
                            failed += 1;
                            continue;
                        }
                    }
                }
                
                if max > 0.001 {
                    println!("OK  (peak: {max:.4})");
                } else {
                    println!("OK  (silent — effect plugin or needs preset)");
                }
                passed += 1;
            }
            Err(e) => {
                println!("LOAD FAIL: {e}");
                failed += 1;
            }
        }
    }
    
    println!("\n{passed} passed, {failed} failed out of {} plugins", plugins.len());
    assert!(passed > 0, "at least one plugin should load");
}
