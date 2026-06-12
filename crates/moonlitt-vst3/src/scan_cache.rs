//! Plug-in scan cache.
//!
//! Scanning a VST3 library means `dlopen`-ing every `.vst3` bundle, asking
//! its factory for class info, and discarding the module. On a 100-plug-in
//! system that's seconds of startup time even though the plug-in metadata
//! hasn't changed.
//!
//! This module caches scan results keyed by `(bundle_path, mtime)`. A
//! warm scan stat-only checks each bundle's mtime — if unchanged, reuse
//! the cached entry; otherwise re-probe and update. Entries for paths
//! that no longer exist are evicted on the next scan.
//!
//! Persisted to disk as JSON via serde — easy to inspect, forward
//! compatible (extra fields ignored).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::scanner::PluginInfo;

/// On-disk cache of scanned plug-ins, keyed by bundle path.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct PluginScanCache {
    /// One entry per .vst3 bundle that was successfully probed. Keyed by
    /// canonicalized bundle path so two scans from different working
    /// directories agree.
    #[serde(default)]
    entries: BTreeMap<PathBuf, CacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    /// Bundle mtime when this entry was last populated. Stored as nanoseconds
    /// since UNIX_EPOCH so the cache file is platform-independent.
    mtime_ns: u128,
    /// Plug-ins discovered in this bundle. May be empty (some .vst3
    /// bundles have non-Audio classes only).
    plugins: Vec<CachedPluginInfo>,
}

/// Cache-side mirror of [`PluginInfo`] — same fields, but the class_id
/// serializes as a hex string for JSON readability.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedPluginInfo {
    name: String,
    class_id_hex: String,
    category: String,
    subcategories: Option<String>,
    vendor: Option<String>,
    version: Option<String>,
}

impl PluginScanCache {
    /// Load the cache from `path`. Missing or corrupt files yield an
    /// empty cache — the caller's next scan repopulates it.
    pub fn load(path: &Path) -> Self {
        let Ok(bytes) = fs::read(path) else {
            return Self::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    /// Persist the cache to `path` atomically (write to a sibling then
    /// rename) so a crashed write doesn't corrupt the file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&tmp, &bytes)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Number of cached bundles. Useful for tests and diagnostics.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Look up a cache entry by canonical path + check it's fresh against
    /// the current mtime on disk. Returns `Some(plugins)` if fresh,
    /// `None` if stale or missing.
    pub(crate) fn fresh_entry(&self, path: &Path, mtime_ns: u128) -> Option<Vec<PluginInfo>> {
        let entry = self.entries.get(path)?;
        if entry.mtime_ns != mtime_ns {
            return None;
        }
        Some(
            entry
                .plugins
                .iter()
                .map(|p| PluginInfo {
                    name: p.name.clone(),
                    path: path.to_path_buf(),
                    class_id: hex_to_cid(&p.class_id_hex),
                    category: p.category.clone(),
                    subcategories: p.subcategories.clone(),
                    vendor: p.vendor.clone(),
                    version: p.version.clone(),
                })
                .collect(),
        )
    }

    /// Insert a freshly-probed entry. Overwrites any previous entry for
    /// the same path.
    pub(crate) fn upsert(&mut self, path: PathBuf, mtime_ns: u128, plugins: &[PluginInfo]) {
        let cached = plugins
            .iter()
            .map(|p| CachedPluginInfo {
                name: p.name.clone(),
                class_id_hex: cid_to_hex(&p.class_id),
                category: p.category.clone(),
                subcategories: p.subcategories.clone(),
                vendor: p.vendor.clone(),
                version: p.version.clone(),
            })
            .collect();
        self.entries.insert(
            path,
            CacheEntry {
                mtime_ns,
                plugins: cached,
            },
        );
    }

    /// Drop any entry whose path is not in `live`. Called at the end of
    /// a scan so the cache doesn't grow unbounded.
    pub(crate) fn retain_paths(&mut self, live: &std::collections::HashSet<PathBuf>) {
        self.entries.retain(|p, _| live.contains(p));
    }
}

/// Read the bundle's mtime as nanoseconds since UNIX_EPOCH. Used as the
/// cache invalidation key. Returns 0 if the metadata isn't available
/// (rare; equivalent to "always re-probe").
pub(crate) fn bundle_mtime_ns(path: &Path) -> u128 {
    let Ok(meta) = fs::metadata(path) else {
        return 0;
    };
    let Ok(modified) = meta.modified() else {
        return 0;
    };
    modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn cid_to_hex(cid: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in cid {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn hex_to_cid(hex: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        let lo = i * 2;
        let hi = lo + 2;
        if hi <= hex.len() {
            *b = u8::from_str_radix(&hex[lo..hi], 16).unwrap_or(0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cid_hex_roundtrip() {
        let cid = [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54,
            0x32, 0x10,
        ];
        let hex = cid_to_hex(&cid);
        assert_eq!(hex, "0123456789abcdeffedcba9876543210");
        assert_eq!(hex_to_cid(&hex), cid);
    }

    #[test]
    fn fresh_entry_returns_none_on_mtime_change() {
        let mut cache = PluginScanCache::default();
        let path = PathBuf::from("/fake/path.vst3");
        let info = PluginInfo {
            name: "Test".into(),
            path: path.clone(),
            class_id: [0u8; 16],
            category: "Audio Module Class".into(),
            subcategories: Some("Instrument|Synth".into()),
            vendor: Some("Acme".into()),
            version: Some("1.0".into()),
        };
        cache.upsert(path.clone(), 1000, &[info]);

        // Same mtime → fresh
        assert!(cache.fresh_entry(&path, 1000).is_some());
        // Newer mtime → stale
        assert!(cache.fresh_entry(&path, 2000).is_none());
    }

    #[test]
    fn retain_paths_drops_missing_entries() {
        use std::collections::HashSet;
        let mut cache = PluginScanCache::default();
        cache.upsert(PathBuf::from("/a"), 1, &[]);
        cache.upsert(PathBuf::from("/b"), 1, &[]);
        cache.upsert(PathBuf::from("/c"), 1, &[]);
        let mut live = HashSet::new();
        live.insert(PathBuf::from("/a"));
        live.insert(PathBuf::from("/c"));
        cache.retain_paths(&live);
        assert_eq!(cache.entry_count(), 2);
        assert!(cache.entries.contains_key(&PathBuf::from("/a")));
        assert!(cache.entries.contains_key(&PathBuf::from("/c")));
        assert!(!cache.entries.contains_key(&PathBuf::from("/b")));
    }
}
