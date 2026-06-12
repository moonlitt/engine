//! Mixer tests — moved verbatim from `mixer.rs` as part of the module split.

use crate::channel::DelayLine;
use crate::render::{apply_pan, process_insert_chain, soft_limit};
use crate::{InsertEffect, Mixer, OutputTarget};
use moonlitt_core::{AudioBackend, NullBackend};

/// Shorthand for creating a boxed NullBackend in tests.
fn null(sr: u32) -> Box<dyn AudioBackend> {
    Box::new(NullBackend::new(sr))
}

#[test]
fn test_pan_center_is_minus_3db() {
    let mut l = vec![1.0];
    let mut r = vec![1.0];
    apply_pan(&mut l, &mut r, 0.0);
    // At center: gain = cos(π/4) ≈ 0.7071
    let expected = std::f32::consts::FRAC_1_SQRT_2;
    assert!(
        (l[0] - expected).abs() < 0.001,
        "Center L should be ~0.707, got {}",
        l[0]
    );
    assert!(
        (r[0] - expected).abs() < 0.001,
        "Center R should be ~0.707, got {}",
        r[0]
    );
}

#[test]
fn test_pan_hard_left() {
    let mut l = vec![1.0];
    let mut r = vec![1.0];
    apply_pan(&mut l, &mut r, -1.0);
    assert!(l[0] > 0.99); // nearly full
    assert!(r[0] < 0.01); // nearly zero
}

#[test]
fn test_pan_hard_right() {
    let mut l = vec![1.0];
    let mut r = vec![1.0];
    apply_pan(&mut l, &mut r, 1.0);
    assert!(l[0] < 0.01);
    assert!(r[0] > 0.99);
}

#[test]
fn test_soft_limit_below_threshold() {
    assert_eq!(soft_limit(0.5, 0.95), 0.5);
    assert_eq!(soft_limit(-0.3, 0.95), -0.3);
}

#[test]
fn test_soft_limit_above_threshold() {
    let limited = soft_limit(2.0, 0.95);
    assert!(limited > 0.95);
    // Output approaches 1.0 asymptotically but never exceeds it meaningfully
    assert!(limited <= 1.0 + f32::EPSILON);
    // Should be less than the input
    assert!(limited < 2.0);
}

#[test]
fn test_soft_limit_preserves_sign() {
    let pos = soft_limit(2.0, 0.95);
    let neg = soft_limit(-2.0, 0.95);
    assert!(pos > 0.0);
    assert!(neg < 0.0);
    assert!((pos + neg).abs() < 0.001);
}

#[test]
fn test_mixer_empty_renders_silence() {
    let mut mixer = Mixer::new(44100, 256);
    let mut left = vec![1.0; 256];
    let mut right = vec![1.0; 256];
    mixer.render(&mut left, &mut right);
    assert!(left.iter().all(|&s| s == 0.0));
    assert!(right.iter().all(|&s| s == 0.0));
}

#[test]
fn test_mixer_single_track() {
    let mut mixer = Mixer::new(44100, 256);
    let engine = null(44100);
    mixer.add_track(engine, 0xFFFF); // all channels
    let mut left = vec![0.0; 256];
    let mut right = vec![0.0; 256];
    mixer.render(&mut left, &mut right);
    // Engine with no backend renders silence — should pass without crash
}

#[test]
fn test_mixer_mute() {
    let mut mixer = Mixer::new(44100, 256);
    let engine = null(44100);
    let id = mixer.add_track(engine, 0xFFFF);
    mixer.track_mut(id).unwrap().mute = true;
    let mut left = vec![0.0; 256];
    let mut right = vec![0.0; 256];
    mixer.render(&mut left, &mut right);
    // Muted track contributes nothing
    assert!(left.iter().all(|&s| s == 0.0));
}

// --- Insert effect tests ---

#[test]
fn test_add_insert() {
    let mut mixer = Mixer::new(44100, 256);
    let engine = null(44100);
    let track_id = mixer.add_track(engine, 0xFFFF);

    let effect = null(44100);
    let insert_id = mixer.add_insert(track_id, effect);
    assert!(insert_id.is_some());
    assert_eq!(mixer.track(track_id).unwrap().inserts.len(), 1);
}

#[test]
fn test_add_insert_invalid_track() {
    let mut mixer = Mixer::new(44100, 256);
    let effect = null(44100);
    assert!(mixer.add_insert(999, effect).is_none());
}

#[test]
fn test_remove_insert() {
    let mut mixer = Mixer::new(44100, 256);
    let engine = null(44100);
    let track_id = mixer.add_track(engine, 0xFFFF);

    let effect = null(44100);
    let insert_id = mixer.add_insert(track_id, effect).unwrap();
    assert_eq!(mixer.track(track_id).unwrap().inserts.len(), 1);

    let removed = mixer.remove_insert(track_id, insert_id);
    assert!(removed.is_some());
    assert_eq!(mixer.track(track_id).unwrap().inserts.len(), 0);
}

#[test]
fn test_remove_insert_invalid() {
    let mut mixer = Mixer::new(44100, 256);
    let engine = null(44100);
    let track_id = mixer.add_track(engine, 0xFFFF);
    assert!(mixer.remove_insert(track_id, 999).is_none());
    assert!(mixer.remove_insert(999, 0).is_none());
}

#[test]
fn test_insert_bypass() {
    let mut mixer = Mixer::new(44100, 256);
    let engine = null(44100);
    let track_id = mixer.add_track(engine, 0xFFFF);

    let effect = null(44100);
    let insert_id = mixer.add_insert(track_id, effect).unwrap();

    // Default: not bypassed
    assert!(!mixer.track(track_id).unwrap().inserts[0].bypass);

    mixer.set_insert_bypass(track_id, insert_id, true);
    assert!(mixer.track(track_id).unwrap().inserts[0].bypass);

    mixer.set_insert_bypass(track_id, insert_id, false);
    assert!(!mixer.track(track_id).unwrap().inserts[0].bypass);
}

#[test]
fn test_insert_ids_are_unique() {
    let mut mixer = Mixer::new(44100, 256);
    let engine = null(44100);
    let track_id = mixer.add_track(engine, 0xFFFF);

    let id1 = mixer.add_insert(track_id, null(44100)).unwrap();
    let id2 = mixer.add_insert(track_id, null(44100)).unwrap();
    let id3 = mixer.add_insert(track_id, null(44100)).unwrap();
    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
}

#[test]
fn test_insert_chain_renders_without_crash() {
    let mut mixer = Mixer::new(44100, 256);
    let engine = null(44100);
    let track_id = mixer.add_track(engine, 0xFFFF);

    // Add 3 inserts (no-backend engines = they zero the output, simulating effects)
    mixer.add_insert(track_id, null(44100));
    mixer.add_insert(track_id, null(44100));
    mixer.add_insert(track_id, null(44100));

    let mut left = vec![0.0; 256];
    let mut right = vec![0.0; 256];
    mixer.render(&mut left, &mut right);
    // Should not crash
}

#[test]
fn test_insert_chain_all_bypassed() {
    let mut mixer = Mixer::new(44100, 256);
    let engine = null(44100);
    let track_id = mixer.add_track(engine, 0xFFFF);

    let id1 = mixer.add_insert(track_id, null(44100)).unwrap();
    let id2 = mixer.add_insert(track_id, null(44100)).unwrap();
    mixer.set_insert_bypass(track_id, id1, true);
    mixer.set_insert_bypass(track_id, id2, true);

    let mut left = vec![0.0; 256];
    let mut right = vec![0.0; 256];
    mixer.render(&mut left, &mut right);
    // All bypassed = same as no inserts
}

#[test]
fn test_process_insert_chain_passthrough_when_empty() {
    // With no inserts, audio should be unmodified
    let mut left = vec![0.5; 64];
    let mut right = vec![0.3; 64];
    let mut scratch_l = vec![0.0; 64];
    let mut scratch_r = vec![0.0; 64];
    let mut inserts: Vec<InsertEffect> = vec![];

    process_insert_chain(
        &mut inserts,
        &mut left,
        &mut right,
        &mut scratch_l,
        &mut scratch_r,
        64,
    );

    assert!(left.iter().all(|&s| (s - 0.5).abs() < f32::EPSILON));
    assert!(right.iter().all(|&s| (s - 0.3).abs() < f32::EPSILON));
}

#[test]
fn test_process_insert_chain_all_bypassed_passthrough() {
    let mut left = vec![0.5; 64];
    let mut right = vec![0.3; 64];
    let mut scratch_l = vec![0.0; 64];
    let mut scratch_r = vec![0.0; 64];
    let mut inserts = vec![
        InsertEffect {
            id: 0,
            backend: null(44100),
            bypass: true,
            source_path: None,
            sidechain_source: None,
        },
        InsertEffect {
            id: 1,
            backend: null(44100),
            bypass: true,
            source_path: None,
            sidechain_source: None,
        },
    ];

    process_insert_chain(
        &mut inserts,
        &mut left,
        &mut right,
        &mut scratch_l,
        &mut scratch_r,
        64,
    );

    // All bypassed = audio unchanged
    assert!(left.iter().all(|&s| (s - 0.5).abs() < f32::EPSILON));
    assert!(right.iter().all(|&s| (s - 0.3).abs() < f32::EPSILON));
}

#[test]
fn test_track_insert_accessor() {
    let mut mixer = Mixer::new(44100, 256);
    let engine = null(44100);
    let track_id = mixer.add_track(engine, 0xFFFF);
    let insert_id = mixer.add_insert(track_id, null(44100)).unwrap();

    assert!(mixer.track_insert(track_id, insert_id).is_some());
    assert!(mixer.track_insert(track_id, 999).is_none());
    assert!(mixer.track_insert(999, insert_id).is_none());
}

#[test]
fn test_multiple_tracks_with_inserts() {
    let mut mixer = Mixer::new(44100, 256);
    let t1 = mixer.add_track(null(44100), 0x0001);
    let t2 = mixer.add_track(null(44100), 0x0002);

    mixer.add_insert(t1, null(44100));
    mixer.add_insert(t1, null(44100));
    mixer.add_insert(t2, null(44100));

    assert_eq!(mixer.track(t1).unwrap().inserts.len(), 2);
    assert_eq!(mixer.track(t2).unwrap().inserts.len(), 1);

    let mut left = vec![0.0; 256];
    let mut right = vec![0.0; 256];
    mixer.render(&mut left, &mut right);
    // Should not crash with multiple tracks each having inserts
}

// --- PDC tests ---

#[test]
fn test_delay_line_passthrough_when_zero() {
    let mut dl = DelayLine::new();
    let mut left = vec![1.0, 2.0, 3.0];
    let mut right = vec![4.0, 5.0, 6.0];
    dl.process(&mut left, &mut right);
    // Zero delay = passthrough
    assert_eq!(left, vec![1.0, 2.0, 3.0]);
    assert_eq!(right, vec![4.0, 5.0, 6.0]);
}

#[test]
fn test_delay_line_delays_by_n_samples() {
    let mut dl = DelayLine::new();
    dl.set_delay(2);

    // First block: input [1,2,3], output should be [0,0,1] (delayed by 2)
    let mut left = vec![1.0, 2.0, 3.0];
    let mut right = vec![0.0; 3];
    dl.process(&mut left, &mut right);
    assert_eq!(left, vec![0.0, 0.0, 1.0]);

    // Second block: input [4,5,6], output should be [2,3,4] (continuing)
    let mut left2 = vec![4.0, 5.0, 6.0];
    let mut right2 = vec![0.0; 3];
    dl.process(&mut left2, &mut right2);
    assert_eq!(left2, vec![2.0, 3.0, 4.0]);
}

#[test]
fn test_delay_line_set_delay_clears_buffer() {
    let mut dl = DelayLine::new();
    dl.set_delay(4);
    let mut left = vec![1.0; 4];
    let mut right = vec![0.0; 4];
    dl.process(&mut left, &mut right);
    // Output is zeros (delay buffer was initialized to zero)
    assert!(left.iter().all(|&s| s == 0.0));

    // Changing delay clears buffer
    dl.set_delay(2);
    let mut left2 = vec![5.0; 2];
    let mut right2 = vec![0.0; 2];
    dl.process(&mut left2, &mut right2);
    assert!(left2.iter().all(|&s| s == 0.0)); // Fresh zero buffer
}

#[test]
fn test_pdc_no_inserts_no_delay() {
    let mut mixer = Mixer::new(44100, 256);
    mixer.add_track(null(44100), 0xFFFF);
    mixer.add_track(null(44100), 0xFFFF);
    mixer.recalculate_pdc();

    // No inserts → no latency → no delay
    assert_eq!(mixer.tracks()[0].delay_line.delay, 0);
    assert_eq!(mixer.tracks()[1].delay_line.delay, 0);
}

#[test]
fn test_pdc_recalculate_on_insert_add() {
    let mut mixer = Mixer::new(44100, 256);
    mixer.add_track(null(44100), 0x0001);
    mixer.add_track(null(44100), 0x0002);

    // Add insert to track 0 (Engine with no backend reports 0 latency)
    mixer.add_insert(0, null(44100));

    // Both tracks have 0 latency (no backend) → no compensation
    assert_eq!(mixer.tracks()[0].delay_line.delay, 0);
    assert_eq!(mixer.tracks()[1].delay_line.delay, 0);
}

#[test]
fn test_pdc_renders_without_crash() {
    let mut mixer = Mixer::new(44100, 256);
    mixer.add_track(null(44100), 0xFFFF);
    mixer.add_track(null(44100), 0xFFFF);
    mixer.add_insert(0, null(44100));
    mixer.recalculate_pdc();

    let mut left = vec![0.0; 256];
    let mut right = vec![0.0; 256];
    mixer.render(&mut left, &mut right);
    // Should render without crash
}

#[test]
fn test_pdc_bypass_recalculates() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0xFFFF);
    mixer.add_track(null(44100), 0xFFFF);
    let i0 = mixer.add_insert(t0, null(44100)).unwrap();

    // Bypass should trigger recalculation
    mixer.set_insert_bypass(t0, i0, true);
    // No crash, PDC updated
}

// --- Group track / submix tests ---

#[test]
fn test_set_track_output_to_group() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0x0001);
    let t1 = mixer.add_track(null(44100), 0x0002); // group

    assert!(mixer.set_track_output(t0, OutputTarget::Group(t1)));
    assert_eq!(
        mixer.track(t0).unwrap().output_target,
        OutputTarget::Group(t1)
    );
}

#[test]
fn test_set_track_output_rejects_self() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0xFFFF);
    assert!(!mixer.set_track_output(t0, OutputTarget::Group(t0)));
}

#[test]
fn test_set_track_output_rejects_cycle() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0x0001);
    let t1 = mixer.add_track(null(44100), 0x0002);
    mixer.set_track_output(t0, OutputTarget::Group(t1));
    // t1 → t0 would create cycle
    assert!(!mixer.set_track_output(t1, OutputTarget::Group(t0)));
}

#[test]
fn test_set_track_output_rejects_nonexistent() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0xFFFF);
    assert!(!mixer.set_track_output(t0, OutputTarget::Group(999)));
}

#[test]
fn test_group_track_renders_without_crash() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0x0001);
    let t1 = mixer.add_track(null(44100), 0x0002);
    let group = mixer.add_track(null(44100), 0x0000); // group (no MIDI)

    mixer.set_track_output(t0, OutputTarget::Group(group));
    mixer.set_track_output(t1, OutputTarget::Group(group));

    let mut left = vec![0.0; 256];
    let mut right = vec![0.0; 256];
    mixer.render(&mut left, &mut right);
}

#[test]
fn test_remove_group_target_resets_routing() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0x0001);
    let group = mixer.add_track(null(44100), 0x0000);
    mixer.set_track_output(t0, OutputTarget::Group(group));

    mixer.remove_track(group);
    // t0 should be reset to Master
    assert_eq!(mixer.track(t0).unwrap().output_target, OutputTarget::Master);
}

#[test]
fn test_render_order_sources_before_groups() {
    let mut mixer = Mixer::new(44100, 256);
    let _t0 = mixer.add_track(null(44100), 0x0001);
    let group = mixer.add_track(null(44100), 0x0000);
    let _t1 = mixer.add_track(null(44100), 0x0002);

    mixer.set_track_output(_t0, OutputTarget::Group(group));
    mixer.set_track_output(_t1, OutputTarget::Group(group));

    // Group should be last in render order
    let last_idx = *mixer.render_order.last().unwrap();
    assert_eq!(mixer.tracks[last_idx].id, group);
}

// --- Trim tests ---

#[test]
fn test_trim_zero_is_passthrough() {
    let mut mixer = Mixer::new(44100, 256);
    let id = mixer.add_track(null(44100), 0xFFFF);
    // trim_db defaults to 0.0
    assert_eq!(mixer.track(id).unwrap().trim_db, 0.0);

    // Manually write known data into the track buffer and render
    // With a no-backend engine, output is silence regardless, so verify
    // the field default and setter round-trip
    mixer.set_track_trim(id, 0.0);
    assert_eq!(mixer.track(id).unwrap().trim_db, 0.0);
}

#[test]
fn test_trim_plus_6db() {
    let mut mixer = Mixer::new(44100, 4);
    let id = mixer.add_track(null(44100), 0xFFFF);
    mixer.set_track_trim(id, 6.0);

    let expected_gain = 10f32.powf(6.0 / 20.0); // ~1.9953

    // Directly verify the trim_db is stored
    assert!((mixer.track(id).unwrap().trim_db - 6.0).abs() < f32::EPSILON);

    // Verify the gain factor
    assert!(
        (expected_gain - 1.9953).abs() < 0.001,
        "6 dB gain should be ~1.9953, got {}",
        expected_gain
    );
}

#[test]
fn test_trim_clamp() {
    let mut mixer = Mixer::new(44100, 256);
    let id = mixer.add_track(null(44100), 0xFFFF);

    // Above +24 should clamp to +24
    mixer.set_track_trim(id, 30.0);
    assert_eq!(mixer.track(id).unwrap().trim_db, 24.0);

    // Below -24 should clamp to -24
    mixer.set_track_trim(id, -30.0);
    assert_eq!(mixer.track(id).unwrap().trim_db, -24.0);

    // Within range should pass through
    mixer.set_track_trim(id, -12.5);
    assert!((mixer.track(id).unwrap().trim_db - (-12.5)).abs() < f32::EPSILON);
}

// --- Sidechain tests ---

#[test]
fn test_sidechain_source_default_none() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0xFFFF);
    let i0 = mixer.add_insert(t0, null(44100)).unwrap();
    assert_eq!(mixer.track_insert(t0, i0).unwrap().sidechain_source, None);
}

#[test]
fn test_set_insert_sidechain() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0x0001);
    let t1 = mixer.add_track(null(44100), 0x0002);
    let i1 = mixer.add_insert(t1, null(44100)).unwrap();

    // Set sidechain: t1's insert uses t0 as sidechain source
    assert!(mixer.set_insert_sidechain(t1, i1, Some(t0)));
    assert_eq!(
        mixer.track_insert(t1, i1).unwrap().sidechain_source,
        Some(t0)
    );

    // Clear sidechain
    assert!(mixer.set_insert_sidechain(t1, i1, None));
    assert_eq!(mixer.track_insert(t1, i1).unwrap().sidechain_source, None);
}

#[test]
fn test_sidechain_rejects_self() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0xFFFF);
    let i0 = mixer.add_insert(t0, null(44100)).unwrap();

    // Can't sidechain to self
    assert!(!mixer.set_insert_sidechain(t0, i0, Some(t0)));
}

#[test]
fn test_sidechain_rejects_nonexistent() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0xFFFF);
    let i0 = mixer.add_insert(t0, null(44100)).unwrap();

    // Nonexistent source track
    assert!(!mixer.set_insert_sidechain(t0, i0, Some(999)));

    // Nonexistent track
    assert!(!mixer.set_insert_sidechain(999, 0, Some(t0)));

    // Nonexistent insert
    assert!(!mixer.set_insert_sidechain(t0, 999, Some(t0)));
}

#[test]
fn test_sidechain_cycle_rejected() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0x0001);
    let t1 = mixer.add_track(null(44100), 0x0002);
    let i0 = mixer.add_insert(t0, null(44100)).unwrap();
    let i1 = mixer.add_insert(t1, null(44100)).unwrap();

    // A sidechains from B
    assert!(mixer.set_insert_sidechain(t0, i0, Some(t1)));
    // B sidechains from A — would create a cycle
    assert!(!mixer.set_insert_sidechain(t1, i1, Some(t0)));
}

#[test]
fn test_render_order_with_sidechain() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0x0001); // source
    let t1 = mixer.add_track(null(44100), 0x0002); // dependent
    let i1 = mixer.add_insert(t1, null(44100)).unwrap();

    // t1's insert uses t0 as sidechain source => t0 must render first
    mixer.set_insert_sidechain(t1, i1, Some(t0));

    let t0_idx = mixer.tracks().iter().position(|t| t.id == t0).unwrap();
    let t1_idx = mixer.tracks().iter().position(|t| t.id == t1).unwrap();

    let t0_order = mixer
        .render_order
        .iter()
        .position(|&i| i == t0_idx)
        .unwrap();
    let t1_order = mixer
        .render_order
        .iter()
        .position(|&i| i == t1_idx)
        .unwrap();
    assert!(
        t0_order < t1_order,
        "Source track must render before dependent track"
    );
}

#[test]
fn test_sidechain_renders_without_crash() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0x0001);
    let t1 = mixer.add_track(null(44100), 0x0002);
    let i1 = mixer.add_insert(t1, null(44100)).unwrap();

    mixer.set_insert_sidechain(t1, i1, Some(t0));

    let mut left = vec![0.0; 256];
    let mut right = vec![0.0; 256];
    mixer.render(&mut left, &mut right);
    // Should not crash
}

#[test]
fn test_remove_track_clears_sidechain_refs() {
    let mut mixer = Mixer::new(44100, 256);
    let t0 = mixer.add_track(null(44100), 0x0001);
    let t1 = mixer.add_track(null(44100), 0x0002);
    let i1 = mixer.add_insert(t1, null(44100)).unwrap();

    mixer.set_insert_sidechain(t1, i1, Some(t0));
    assert_eq!(
        mixer.track_insert(t1, i1).unwrap().sidechain_source,
        Some(t0)
    );

    // Remove source track — sidechain ref should be cleared
    mixer.remove_track(t0);
    assert_eq!(mixer.track_insert(t1, i1).unwrap().sidechain_source, None);
}
