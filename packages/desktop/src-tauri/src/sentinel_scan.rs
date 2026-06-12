//! Sentinel VST3 scanning — probe each bundle in an ISOLATED child
//! process so a crashing or hanging plug-in can't take the app down.
//!
//! The pattern every serious DAW uses (Logic's auval, Live's and
//! Bitwig's sentinel scanners): the parent never dlopens an unknown
//! bundle. It spawns itself with `MOONLITT_PROBE_VST3=<path>`; the
//! child probes, prints JSON on stdout and exits. Crash / non-zero /
//! timeout → the bundle goes on a persistent blacklist and is skipped
//! (with a log line) until the user re-enables it by deleting the
//! blacklist file or the bundle's mtime changes.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const PROBE_ENV: &str = "MOONLITT_PROBE_VST3";
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const BLACKLIST_FILE: &str = "plugin-blacklist.json";

/// What the probe child reports per discovered plug-in class. Loading
/// later goes by path (`load_from_path` re-reads the bundle factory),
/// so the wire format stays minimal.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbedPlugin {
    pub name: String,
    pub path: String,
    pub subcategories: Option<String>,
}

/// One quarantined bundle.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlacklistEntry {
    pub path: String,
    pub reason: String,
    /// Bundle mtime (ns) at quarantine time — if the bundle is updated
    /// (reinstalled), we give it another chance automatically.
    pub mtime_ns: u128,
}

/// Child-process entry point. Returns `true` when this process was a
/// probe worker (caller must exit instead of starting the app).
pub fn maybe_run_probe_worker() -> bool {
    let Ok(path) = std::env::var(PROBE_ENV) else {
        return false;
    };
    let result: Vec<ProbedPlugin> = match moonlitt_vst3::probe_path(Path::new(&path)) {
        Ok(infos) => infos
            .into_iter()
            .map(|i| ProbedPlugin {
                name: i.name,
                path: i.path.to_string_lossy().into_owned(),
                subcategories: i.subcategories,
            })
            .collect(),
        Err(e) => {
            eprintln!("probe failed: {e}");
            std::process::exit(2);
        }
    };
    println!("{}", serde_json::to_string(&result).unwrap_or_default());
    true
}

fn bundle_mtime_ns(path: &Path) -> u128 {
    fn newest(p: &Path, depth: usize) -> u128 {
        let own = std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        if depth == 0 || !p.is_dir() {
            return own;
        }
        let mut max = own;
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                max = max.max(newest(&e.path(), depth - 1));
            }
        }
        max
    }
    newest(path, 3)
}

fn blacklist_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;
    let dir = app.path().app_data_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(BLACKLIST_FILE))
}

pub fn load_blacklist(app: &tauri::AppHandle) -> Vec<BlacklistEntry> {
    blacklist_path(app)
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_blacklist(app: &tauri::AppHandle, list: &[BlacklistEntry]) {
    if let (Some(path), Ok(json)) = (blacklist_path(app), serde_json::to_vec_pretty(list)) {
        let _ = std::fs::write(path, json);
    }
}

/// Probe every candidate VST3 bundle through child processes. Bundles
/// on the blacklist are skipped unless their mtime changed since
/// quarantine (reinstall = second chance). Newly-misbehaving bundles
/// are added to the blacklist and reported in the returned log lines.
pub fn scan_vst3(app: &tauri::AppHandle) -> (Vec<ProbedPlugin>, Vec<String>) {
    let mut blacklist = load_blacklist(app);
    let mut found = Vec::new();
    let mut log = Vec::new();
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            log.push(format!("current_exe failed ({e}) — VST3 scan skipped"));
            return (found, log);
        }
    };

    for bundle in moonlitt_vst3::candidate_bundle_paths() {
        let mtime = bundle_mtime_ns(&bundle);
        let bundle_str = bundle.to_string_lossy().into_owned();
        if let Some(entry) = blacklist.iter().find(|b| b.path == bundle_str) {
            if entry.mtime_ns == mtime {
                log.push(format!(
                    "跳过隔离插件 {bundle_str}（{}）— 重装该插件可自动解除",
                    entry.reason
                ));
                continue;
            }
            // Bundle changed since quarantine — give it another chance.
            blacklist.retain(|b| b.path != bundle_str);
        }

        match probe_in_child(&exe, &bundle) {
            Ok(mut plugins) => found.append(&mut plugins),
            Err(reason) => {
                log.push(format!("隔离插件 {bundle_str}：{reason}"));
                blacklist.push(BlacklistEntry {
                    path: bundle_str,
                    reason,
                    mtime_ns: mtime,
                });
            }
        }
    }

    save_blacklist(app, &blacklist);
    (found, log)
}

fn probe_in_child(exe: &Path, bundle: &Path) -> Result<Vec<ProbedPlugin>, String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(exe)
        .env(PROBE_ENV, bundle)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn probe: {e}"))?;

    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    let _ = stdout.read_to_string(&mut out);
                }
                if status.success() {
                    return serde_json::from_str(out.trim())
                        .map_err(|e| format!("probe output unparseable: {e}"));
                }
                return Err(match status.code() {
                    Some(2) => "插件探测失败（加载或工厂枚举出错）".to_string(),
                    Some(c) => format!("探测进程异常退出（code {c}）"),
                    None => "探测进程崩溃（信号终止）".to_string(),
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("探测超时（>{}s，疑似挂起）", PROBE_TIMEOUT.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("try_wait: {e}")),
        }
    }
}
