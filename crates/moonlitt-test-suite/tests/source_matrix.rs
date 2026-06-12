//! Source × capability acceptance matrix — the Definition-of-Done
//! artifact from the core-polish spec (§8).
//!
//! One test per sound source; each asserts the capability chain
//! {load, preset/program, params, state, offline render audible,
//! warm-up advisory}. Cells gate on locally-installed assets and PRINT
//! exactly what they skipped, so coverage is never silently overstated.
//!
//! Realtime playback and session roundtrips are covered end-to-end by
//! the audio-io e2e tests and the C-API/testbed suites respectively.

use moonlitt_core::AudioBackend;

const SF2: &str = "/Users/wangyan/Desktop/stardew valley mods/soundfonts/GeneralUser_GS.sf2";

fn sf2_available() -> bool {
    std::path::Path::new(SF2).exists()
}

fn find_vst3(name_contains: &str) -> Option<std::path::PathBuf> {
    std::fs::read_dir("/Library/Audio/Plug-Ins/VST3")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().to_lowercase().contains(name_contains))
        })
}

/// Render a chord and return the peak across `blocks` blocks.
fn render_peak(backend: &mut Box<dyn AudioBackend>, blocks: usize) -> f32 {
    backend.note_on(0, 60, 100);
    backend.note_on(0, 64, 100);
    backend.note_on(0, 67, 100);
    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];
    let mut peak = 0.0f32;
    for _ in 0..blocks {
        left.fill(0.0);
        right.fill(0.0);
        backend.render(&mut left, &mut right);
        for s in left.iter().chain(right.iter()) {
            peak = peak.max(s.abs());
        }
    }
    backend.all_notes_off();
    peak
}

#[test]
fn matrix_sf2_oxisynth() {
    if !sf2_available() {
        println!("[matrix] SF2-oxisynth: SKIPPED (no SF2 at {SF2})");
        return;
    }
    let mut b = moonlitt_engine::create(SF2, 44100, 256).expect("load");

    // Presets: GM banks must enumerate, and program selection must work.
    assert!(!b.presets().is_empty(), "oxisynth must expose GM presets");
    b.program_change(0, 24); // nylon guitar

    // Params: oxisynth exposes reverb/chorus/gain controls.
    assert!(b.param_count() > 0, "oxisynth must expose parameters");
    let id = b.param_info(0).expect("param 0 info").id;
    b.set_param(id, 1.0);
    assert!(b.get_param(id).is_some());

    // State: by design unsupported — sounds are addressed by preset.
    assert!(!b.supports_state());
    assert_eq!(b.recommended_warm_up_blocks(), 0);

    // Offline render must be audible.
    let peak = render_peak(&mut b, 64);
    assert!(peak > 1e-3, "render must be audible (peak={peak})");
    println!("[matrix] SF2-oxisynth: load/presets/params/render OK (peak={peak:.3})");
}

#[test]
fn matrix_sf2_sampler() {
    if !sf2_available() {
        println!("[matrix] SF2-sampler: SKIPPED (no SF2 at {SF2})");
        return;
    }
    let mut b = moonlitt_engine::create_with_sampler(SF2, 44100, 256).expect("load");

    assert!(!b.presets().is_empty(), "sampler must expose GM programs");
    b.program_change(0, 0);

    // MIDI expressiveness (the sampler's param story) is asserted in
    // depth by moonlitt-sampler's sprint6 suite; here we assert the
    // headline: CC must change the output level.
    let loud = render_peak(&mut b, 64);
    assert!(loud > 1e-3, "render must be audible (peak={loud})");

    let mut quiet_b = moonlitt_engine::create_with_sampler(SF2, 44100, 256).expect("load");
    quiet_b.cc(0, 7, 16);
    let quiet = render_peak(&mut quiet_b, 64);
    assert!(
        quiet < loud * 0.2,
        "CC7 must attenuate (loud={loud}, quiet={quiet})"
    );

    assert!(!b.supports_state());
    println!("[matrix] SF2-sampler: load/programs/CC/render OK (peak={loud:.3})");
}

#[test]
fn matrix_vst3_pianoteq() {
    let Some(path) = find_vst3("pianoteq") else {
        println!("[matrix] VST3-Pianoteq: SKIPPED (not installed)");
        return;
    };
    let mut b = moonlitt_engine::create(&path.to_string_lossy(), 44100, 256).expect("load");

    assert!(b.param_count() > 0, "Pianoteq must expose parameters");
    assert!(b.supports_state(), "VST3 must support state");

    // State roundtrip through the trait.
    let state = b.save_state().expect("save_state");
    assert!(!state.is_empty());
    b.load_state(&state).expect("load_state");

    // Modeled piano needs no warm-up and must be audible immediately.
    assert_eq!(b.recommended_warm_up_blocks(), 0);
    let peak = render_peak(&mut b, 64);
    assert!(peak > 1e-3, "render must be audible (peak={peak})");
    println!("[matrix] VST3-Pianoteq: load/params/state/render OK (peak={peak:.3})");
}

#[test]
fn matrix_vst3_keyscape_state_replay() {
    let Some(path) = find_vst3("keyscape") else {
        println!("[matrix] VST3-Keyscape: SKIPPED (not installed)");
        return;
    };
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../moonlitt-vst3/tests/fixtures/keyscape-default.mlstate");
    if !fixture.exists() {
        println!("[matrix] VST3-Keyscape: SKIPPED (no state fixture)");
        return;
    }

    let mut b = moonlitt_engine::create(&path.to_string_lossy(), 44100, 256).expect("load");
    assert!(b.supports_state());

    // The sample-streamer chain: state replay + advertised warm-up.
    let blob = std::fs::read(&fixture).expect("fixture");
    b.load_state(&blob).expect("load_state");
    let warm = b.recommended_warm_up_blocks();
    assert!(warm > 0, "Spectrasonics must advertise warm-up");
    b.warm_up(warm).expect("warm_up");

    let peak = render_peak(&mut b, 128);
    assert!(peak > 1e-3, "state replay must be audible (peak={peak})");
    println!("[matrix] VST3-Keyscape: load/state-replay/warm-up/render OK (peak={peak:.3})");
}
