//! Spectrasonics patch-library browsing for the desktop UI.
//!
//! Bridges `moonlitt_vst3::spectrasonics` to the UI: resolve a loaded
//! .vst3 to its STEAM product library, cache the scan (the factory
//! `.db` containers are >100 MB and re-reading them per keystroke would
//! hurt), and expose stable patch ids the load command can resolve.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use moonlitt_vst3::spectrasonics::{self, LibraryPatch};
use serde::Serialize;

/// One row in the UI's patch browser. `id` indexes into the cached scan
/// for this plug-in path — stable until the library rescans.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PatchView {
    pub id: usize,
    pub name: String,
    pub category: String,
    pub library: String,
}

static SCANS: OnceLock<Mutex<HashMap<String, Arc<Vec<LibraryPatch>>>>> = OnceLock::new();

fn scans() -> &'static Mutex<HashMap<String, Arc<Vec<LibraryPatch>>>> {
    SCANS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Product name a .vst3 path maps to in the STEAM tree:
/// `/…/Keyscape.vst3` → `Keyscape`.
fn product_name(plugin_path: &str) -> Option<String> {
    std::path::Path::new(plugin_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

fn scan_for(plugin_path: &str) -> Result<Arc<Vec<LibraryPatch>>, String> {
    let product = product_name(plugin_path)
        .ok_or_else(|| format!("无法从 {plugin_path} 解析插件名"))?;
    let dir = spectrasonics::steam_product_dir(&product)
        .ok_or_else(|| format!("未找到 {product} 的 STEAM 音色库（仅 Spectrasonics 系插件支持）"))?;
    let patches = spectrasonics::scan_patch_library(&dir)
        .map_err(|e| format!("扫描 {product} 音色库失败: {e}"))?;
    let arc = Arc::new(patches);
    scans()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(plugin_path.to_string(), arc.clone());
    Ok(arc)
}

/// Enumerate the browsable patches for a plug-in (rescans each call so
/// freshly-saved User patches show up; the result is cached for
/// subsequent [`patch_at`] lookups).
pub fn list(plugin_path: &str) -> Result<Vec<PatchView>, String> {
    let patches = scan_for(plugin_path)?;
    Ok(patches
        .iter()
        .enumerate()
        .map(|(id, p)| PatchView {
            id,
            name: p.name.clone(),
            category: p.category.clone(),
            library: p.library.clone(),
        })
        .collect())
}

/// Resolve a patch id from the last scan for this plug-in path.
pub fn patch_at(plugin_path: &str, id: usize) -> Result<LibraryPatch, String> {
    let cached = scans()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(plugin_path)
        .cloned();
    let patches = match cached {
        Some(p) => p,
        None => scan_for(plugin_path)?,
    };
    patches
        .get(id)
        .cloned()
        .ok_or_else(|| format!("音色 #{id} 不存在（库共 {} 个）", patches.len()))
}

/// Read a patch's raw bytes.
pub fn patch_bytes(patch: &LibraryPatch) -> Result<Vec<u8>, String> {
    spectrasonics::load_patch_bytes(patch).map_err(|e| format!("读取音色失败: {e}"))
}

/// Assemble a state that loads `patch_file` into part 0 of `state`.
pub fn assemble_state(state: &[u8], patch_file: &[u8]) -> Result<Vec<u8>, String> {
    spectrasonics::splice_library_patch(state, patch_file).map_err(|e| format!("拼装音色状态失败: {e}"))
}
