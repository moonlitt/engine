//! End-to-end proof of the Spectrasonics patch-library pipeline against
//! a real Keyscape install: scan the STEAM library, assemble a state
//! for a factory patch via [`spectrasonics::splice_library_patch`],
//! load it, and assert the plug-in actually streams that patch.
//!
//! Skips gracefully when Keyscape or its STEAM library is absent.
//! Name contains "keyscape" so CI's `--skip keyscape` filters it.

use moonlitt_vst3::spectrasonics;

fn find_keyscape() -> Option<std::path::PathBuf> {
    let host = moonlitt_vst3::Vst3Host::new(48_000, 512).ok()?;
    host.scan()
        .ok()?
        .into_iter()
        .find(|p| p.name == "Keyscape")
        .map(|info| info.path)
}

#[test]
fn keyscape_library_patch_loads_and_sounds() {
    let Some(plugin_path) = find_keyscape() else {
        eprintln!("Keyscape not installed — skipping");
        return;
    };
    let Some(product_dir) = spectrasonics::steam_product_dir("Keyscape") else {
        eprintln!("Keyscape STEAM library not found — skipping");
        return;
    };

    let patches = spectrasonics::scan_patch_library(&product_dir).expect("scan library");
    assert!(
        patches.len() > 100,
        "expected a real factory library, found {} patches",
        patches.len()
    );

    // A patch whose name is easy to assert on and sonically unmistakable.
    let target = patches
        .iter()
        .find(|p| p.category.contains("Clavinet"))
        .unwrap_or(&patches[0]);
    eprintln!("target patch: {} / {}", target.category, target.name);
    let patch_bytes = spectrasonics::load_patch_bytes(target).expect("patch bytes");

    let host = moonlitt_vst3::Vst3Host::new(48_000, 512).expect("host");
    let mut plugin = host.load_from_path(&plugin_path).expect("load Keyscape");

    // The freshly-initialised plug-in state is the wrapper donor — no
    // fixture needed.
    let wrapper = plugin.get_state().expect("initial get_state");
    let assembled =
        spectrasonics::splice_library_patch(&wrapper, &patch_bytes).expect("splice state");
    plugin.set_state(&assembled).expect("set_state");

    let blocks = plugin.recommended_warm_up_blocks();
    plugin.warm_up(blocks).expect("warm_up");

    plugin.note_on(0, 60, 110);
    let mut l = vec![0.0f32; 512];
    let mut r = vec![0.0f32; 512];
    let mut peak = 0.0f32;
    for _ in 0..200 {
        plugin.render(&mut l, &mut r).expect("render");
        for s in l.iter().chain(r.iter()) {
            peak = peak.max(s.abs());
        }
    }
    assert!(
        peak > 0.01,
        "assembled state for {:?} rendered silence (peak {peak})",
        target.name
    );

    // The round-tripped state must report the library patch's name.
    let back = plugin.get_state().expect("get_state after load");
    let text = String::from_utf8_lossy(&back);
    assert!(
        text.contains(&target.name),
        "round-tripped state does not mention {:?}",
        target.name
    );
}
