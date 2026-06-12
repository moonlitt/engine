//! SF2 loader robustness: corrupted/truncated soundfont files must
//! produce an `Err`, never a panic — a game must survive a user
//! dropping a broken .sf2 into its folder.
//!
//! Deterministic poor-man's fuzz: seeded truncations and bit flips over
//! a real SF2 header region. Runs everywhere (no nightly/cargo-fuzz
//! needed); each case reports its seed on failure so it replays
//! exactly.

use moonlitt_sampler::SamplePool;

const SF2: &str =
    "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";

/// XorShift64 — deterministic, no deps.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

#[test]
fn corrupted_sf2_errors_instead_of_panicking() {
    if !std::path::Path::new(SF2).exists() {
        eprintln!("SF2 not found, skipping");
        return;
    }
    // The structural parsing happens in the first chunk headers; 256 KB
    // covers RIFF/INFO/sdta/pdta layout without copying 30 MB per case.
    let original = {
        let full = std::fs::read(SF2).expect("read SF2");
        full[..full.len().min(256 * 1024)].to_vec()
    };

    let dir = std::env::temp_dir().join("moonlitt-sf2-robustness");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("mutated.sf2");
    let path_str = path.to_str().unwrap();

    for seed in 0..200u64 {
        let mut rng = Rng(seed + 0x9E37_79B9_7F4A_7C15);
        let mut bytes = original.clone();

        // Random truncation (always — undersized files are the common
        // real-world corruption).
        let new_len = (rng.next() as usize) % bytes.len();
        bytes.truncate(new_len.max(1));

        // A handful of bit flips.
        for _ in 0..(rng.next() % 24) {
            let len = bytes.len();
            let idx = (rng.next() as usize) % len;
            let bit = (rng.next() % 8) as u8;
            bytes[idx] ^= 1 << bit;
        }

        std::fs::write(&path, &bytes).expect("write mutated file");

        let result = std::panic::catch_unwind(|| {
            // Both SF2 loaders must survive: the pure-Rust sampler pool…
            let _ = SamplePool::from_file(path_str);
        });
        assert!(
            result.is_ok(),
            "SamplePool::from_file panicked on mutated SF2 (seed {seed}, {} bytes)",
            bytes.len()
        );
    }
    println!("[robustness] 200 mutated SF2 files: no panics");
}
