//! Long-run render with an allocation assertion: after warm-up, the
//! audio render path must reach a steady state that performs **zero
//! heap allocations** — the observable form of the "audio thread never
//! allocates" invariant.
//!
//! Two tiers:
//! * the from-scratch path (moonlitt-sampler + mixer + built-in
//!   effects) asserts **exactly zero**;
//! * the OxiSynth compatibility backend allocates a small amount per
//!   note-on inside the upstream dep (measured ~1.2/event) — bounded
//!   here so a regression explosion still fails, and tracked as known
//!   upstream RT debt (the pure sampler is the RT-strict choice).
//!
//! Renders 30 s of audio by default; set `MOONLITT_LONG=1` for the
//! full 10-minute soak. Lives in its own test binary because it
//! installs a counting global allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use moonlitt_core::AudioBackend;

struct CountingAlloc;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

const SF2: &str = "/Users/wangyan/Desktop/stardew valley mods/soundfonts/GeneralUser_GS.sf2";

/// Both tests read the same global allocation counter — serialise them
/// so one test's steady-state window never counts the other's set-up.
fn serial_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn soak_seconds() -> u64 {
    if std::env::var("MOONLITT_LONG").as_deref() == Ok("1") {
        600
    } else {
        30
    }
}

/// Build a mixer with `backend` on channel 0 plus a reverb send, warm
/// it up, then render `seconds` of audio with periodic note events.
/// Returns (steady-state allocation count, note-event rounds).
fn soak(backend: Box<dyn AudioBackend>, seconds: u64) -> (u64, u64) {
    let sample_rate = 44100u32;
    let buffer_size = 256usize;
    let mut mixer = moonlitt_audio_io::mixer::Mixer::new(sample_rate, buffer_size);

    let t0 = mixer.add_track(backend, 0xFFFF);
    let reverb = Box::new(moonlitt_effects::DattorroReverb::new(sample_rate));
    let bus = mixer.add_send_bus(reverb);
    mixer.track_mut(t0).expect("track").send_levels[bus as usize] = 0.3;

    let mut left = vec![0.0f32; buffer_size];
    let mut right = vec![0.0f32; buffer_size];

    let total_blocks = (sample_rate as u64 * seconds) as usize / buffer_size;
    let warmup_blocks = 256;

    // Warm-up: one-time lazy allocations (scratch growth, voice tables)
    // are allowed here.
    mixer.note_on(0, 60, 100);
    for _ in 0..warmup_blocks {
        mixer.render(&mut left, &mut right);
    }

    // Steady state: count everything.
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    let mut rounds = 0u64;
    for block in 0..total_blocks {
        if block % 172 == 0 {
            let n = 48 + (rounds % 24) as u8;
            mixer.note_off(0, n);
            mixer.note_on(0, n, 100);
            rounds += 1;
        }
        mixer.render(&mut left, &mut right);
    }
    let after = ALLOCATIONS.load(Ordering::Relaxed);
    (after - before, rounds)
}

#[test]
fn long_run_sampler_path_is_allocation_free() {
    let _serial = serial_lock();
    if !std::path::Path::new(SF2).exists() {
        println!("[long-run] sampler: SKIPPED (no SF2 at {SF2})");
        return;
    }
    let seconds = soak_seconds();
    let backend =
        moonlitt_engine::create_with_sampler(SF2, 44100, 256).expect("load sampler backend");
    let (allocs, rounds) = soak(backend, seconds);
    println!(
        "[long-run] sampler+mixer+reverb: {seconds}s audio, {rounds} note rounds, \
         steady-state allocations: {allocs}"
    );
    assert_eq!(
        allocs, 0,
        "the from-scratch audio path allocated {allocs} times in steady state"
    );
}

#[test]
fn long_run_oxisynth_allocations_stay_bounded() {
    let _serial = serial_lock();
    if !std::path::Path::new(SF2).exists() {
        println!("[long-run] oxisynth: SKIPPED (no SF2 at {SF2})");
        return;
    }
    let seconds = soak_seconds();
    let backend = moonlitt_engine::create(SF2, 44100, 256).expect("load oxisynth backend");
    let (allocs, rounds) = soak(backend, seconds);
    println!(
        "[long-run] oxisynth: {seconds}s audio, {rounds} note rounds, \
         steady-state allocations: {allocs} (known upstream note-on debt)"
    );
    // Known upstream behaviour: ~1.2 allocations per note event inside
    // deps/oxisynth voice setup. Per-block allocations would blow far
    // past this bound; a regression still fails loudly.
    assert!(
        allocs <= rounds * 3,
        "oxisynth allocations exploded: {allocs} over {rounds} note rounds"
    );
}
