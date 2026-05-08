//! VST3 host-side callback tracing.
//!
//! Activated by setting the `MOONLITT_VST3_TRACE=1` environment variable.
//! Writes one line per plugin→host callback to stderr with a relative
//! timestamp and thread id. Used to diagnose what a plugin asks the host
//! for during initialization and processing — invaluable for figuring out
//! why a sampler-style plugin (Keyscape, Kontakt) stays silent.
//!
//! Format:
//! ```text
//! [VST3] +12.345ms tid=0x16b39f000 HostApp::getName -> "Moonlitt"
//! [VST3] +12.347ms tid=0x16b39f000 ComponentHandler::performEdit id=42 value=0.123
//! [VST3] +12.412ms tid=0x16b39f000 CP[comp->ctrl] notify msg="patch.load"
//! ```
//!
//! Off-mode is zero overhead beyond a single relaxed atomic load.

use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static ENABLED: OnceLock<bool> = OnceLock::new();

pub(crate) fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var("MOONLITT_VST3_TRACE")
            .map(|v| v != "0" && !v.is_empty())
            .unwrap_or(false)
    })
}

fn elapsed_ms() -> f64 {
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_secs_f64() * 1000.0
}

fn thread_id() -> String {
    let id = std::thread::current().id();
    format!("{id:?}")
}

/// Emit one trace line. Caller pre-formats the message body.
pub(crate) fn emit(body: &str) {
    if !enabled() {
        return;
    }
    eprintln!(
        "[VST3] +{:.3}ms tid={} {body}",
        elapsed_ms(),
        thread_id()
    );
}

/// Convert a 16-byte interface ID to a human name when known, otherwise hex.
pub(crate) fn iid_name(iid: &[u8; 16]) -> String {
    // VST3 IIDs are stored as TUID (16 bytes). Common ones we care about:
    let known: &[(&str, [u8; 16])] = &[
        ("IPluginBase", *b"\x22\x88\x83\x6F\xEE\xE6\x44\x9D\xB2\x4F\x14\x69\xB8\x8E\x46\x16"),
        // The byte arrays below are placeholders — proper TUIDs would need
        // platform-aware byte ordering. We log raw hex always; iid_name
        // returns hex unless a recognized prefix matches.
    ];
    for (name, bytes) in known {
        if bytes == iid {
            return (*name).to_string();
        }
    }
    hex16(iid)
}

fn hex16(bytes: &[u8; 16]) -> String {
    let mut s = String::with_capacity(35);
    for (i, b) in bytes.iter().enumerate() {
        if i == 4 || i == 6 || i == 8 || i == 10 {
            s.push('-');
        }
        s.push_str(&format!("{:02X}", b));
    }
    s
}
