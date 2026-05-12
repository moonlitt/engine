//! Plug-in scan cache tests.
//!
//! The scan cache lets a DAW host avoid re-loading every .vst3 bundle on
//! every startup. Cache entries are keyed by (bundle path, mtime); a
//! bundle is re-probed iff its mtime advanced since the cache was
//! populated. Stale entries (path missing) are evicted.

use std::path::PathBuf;
use std::time::Duration;

use moonlitt_vst3::{PluginScanCache, Vst3Host};

fn temp_cache_path() -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "moonlitt-vst3-scan-cache-{}-{}.json",
        std::process::id(),
        rand_id()
    ));
    p
}

fn rand_id() -> u64 {
    // Tests run in parallel — avoid file path collisions without needing
    // a rand dep by using nanoseconds since UNIX epoch.
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos() as u64
}

#[test]
fn cache_round_trips_to_disk() {
    let path = temp_cache_path();
    let cache = PluginScanCache::load(&path);
    // Empty cache before any scan.
    assert_eq!(cache.entry_count(), 0);

    let host = Vst3Host::new(44100, 256).unwrap();
    let mut cache = cache;
    let plugins = host.scan_cached(&mut cache).unwrap();
    if plugins.is_empty() {
        eprintln!("No VST3 plug-ins installed — skipping cache test");
        let _ = std::fs::remove_file(&path);
        return;
    }
    cache.save(&path).expect("cache must persist to disk");
    assert!(cache.entry_count() > 0);

    // Round-trip: a fresh load from the same path must recover the
    // same entries.
    let reloaded = PluginScanCache::load(&path);
    assert_eq!(reloaded.entry_count(), cache.entry_count());

    let _ = std::fs::remove_file(&path);
}

#[test]
fn warm_scan_returns_same_plugins_as_cold() {
    let path = temp_cache_path();
    let host = Vst3Host::new(44100, 256).unwrap();

    let mut cold_cache = PluginScanCache::load(&path);
    let cold = host.scan_cached(&mut cold_cache).unwrap();
    cold_cache.save(&path).unwrap();
    if cold.is_empty() {
        eprintln!("No VST3 plug-ins installed — skipping");
        let _ = std::fs::remove_file(&path);
        return;
    }

    let mut warm_cache = PluginScanCache::load(&path);
    let warm = host.scan_cached(&mut warm_cache).unwrap();

    let mut cold_names: Vec<&str> = cold.iter().map(|p| p.name.as_str()).collect();
    let mut warm_names: Vec<&str> = warm.iter().map(|p| p.name.as_str()).collect();
    cold_names.sort();
    warm_names.sort();
    assert_eq!(
        cold_names, warm_names,
        "warm scan must return the same plug-ins as cold scan"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn warm_scan_is_faster_than_cold_scan() {
    let path = temp_cache_path();
    let host = Vst3Host::new(44100, 256).unwrap();

    let mut cache = PluginScanCache::load(&path);

    // Cold pass — actually loads every module.
    let cold_start = std::time::Instant::now();
    let cold = host.scan_cached(&mut cache).unwrap();
    let cold_elapsed = cold_start.elapsed();
    cache.save(&path).unwrap();

    if cold.is_empty() {
        eprintln!("No VST3 plug-ins installed — skipping speed test");
        let _ = std::fs::remove_file(&path);
        return;
    }

    // Warm pass — should skip dlopen for every up-to-date entry.
    let warm_start = std::time::Instant::now();
    let mut cache2 = PluginScanCache::load(&path);
    let _warm = host.scan_cached(&mut cache2).unwrap();
    let warm_elapsed = warm_start.elapsed();

    eprintln!(
        "  scan timings: cold={:?} warm={:?} ({}× speedup)",
        cold_elapsed,
        warm_elapsed,
        cold_elapsed.as_micros() as f64 / warm_elapsed.as_micros().max(1) as f64
    );

    // Warm should be measurably faster than cold. On a first-ever scan
    // cold is hundreds of ms per dlopen; warm is single-digit ms (file
    // stat). On repeated test runs the OS page cache has every .vst3
    // bundle hot, so cold is artificially fast — at minimum, warm must
    // still beat it (cache hit eliminates the factory-creation cost).
    assert!(
        warm_elapsed < cold_elapsed,
        "warm scan ({warm_elapsed:?}) should be at least as fast as cold ({cold_elapsed:?})"
    );

    let _ = std::fs::remove_file(&path);
}
