//! Best-effort patch-name extraction from VST3 state blobs.
//!
//! VST3 has no standard "current patch name" API. Plug-ins put it
//! wherever they want — most commonly inside their state blob. For
//! Spectrasonics' lineup (Keyscape, Omnisphere, Trilian, Stylus RMX),
//! the state blob contains plain-text XML attributes:
//!
//! ```xml
//! <SYNTHENG ... origLibName="Keyscape Library"
//!                origPatchName="LA Custom C7 - Natural" ... >
//! ```
//!
//! Other vendors store patch names in their own opaque containers; if
//! a future plug-in needs support we add another parser here. The
//! caller treats `None` as "unknown patch" and falls back to a
//! user-supplied label or just the plug-in name.

/// Try every parser in this module and return the first hit. The order
/// is "least false-positive risk first" — generic byte sniffing comes
/// last.
pub fn extract_patch_name(state: &[u8]) -> Option<String> {
    extract_spectrasonics(state).or_else(|| extract_pianoteq(state))
}

/// Pull the current preset name out of a Pianoteq VST3 state.
///
/// Pianoteq wraps a VST2-era fxChunk (`VstW`/`CcnK`) whose payload
/// carries a `tdtM` metadata chunk: a few length fields, then the
/// preset name as a NUL-terminated string, then the vendor string
/// `Modartt` and a description. We require both `tdtM` and `Modartt`
/// so random binaries can't fake a match.
fn extract_pianoteq(state: &[u8]) -> Option<String> {
    find_subslice(state, b"Modartt")?;
    let tag = find_subslice(state, b"tdtM")?;
    let scan = &state[tag + 4..];
    let scan = &scan[..scan.len().min(96)];
    // First printable ASCII run of a plausible name length.
    let start = scan.iter().position(|&b| (0x20..0x7f).contains(&b))?;
    let end = scan[start..]
        .iter()
        .position(|&b| !(0x20..0x7f).contains(&b))
        .map(|e| start + e)
        .unwrap_or(scan.len());
    let name = std::str::from_utf8(&scan[start..end]).ok()?.trim();
    (name.len() >= 3).then(|| name.to_string())
}

/// Pull `origPatchName="..."` (with a fallback to the first
/// `<ENTRYDESCR name="...">` attribute) out of a Spectrasonics
/// XML-style state blob.
///
/// `origPatchName` is preferred because Spectrasonics writes the
/// *original library* patch name there even when the user has tweaked
/// it. `<ENTRYDESCR name="">` matches the current display name and is
/// a better signal of what the user actually has loaded.
fn extract_spectrasonics(state: &[u8]) -> Option<String> {
    // Cheap pre-check: bail if the blob doesn't look like Spectrasonics
    // XML at all. Avoids scanning megabytes for nothing on every load.
    find_subslice(state, b"SynthMaster")?;

    // Prefer ENTRYDESCR name (current/user-chosen) over origPatchName
    // (the patch's original library name). They usually match; when
    // they differ the user changed something. A state holds several
    // ENTRYDESCRs — the master descriptor (often "default") plus one
    // per part — so walk all of them, not just the first.
    if let Some(name) = find_all_xml_attr(state, b"<ENTRYDESCR", b"name")
        .into_iter()
        .find(|n| !n.is_empty() && n != "default")
    {
        return Some(name);
    }
    if let Some(name) = find_all_xml_attr(state, b"<SYNTHENG", b"origPatchName")
        .into_iter()
        .find(|n| !n.is_empty())
    {
        return Some(name);
    }
    None
}

/// Locate `<tag ... attr="value"` in `haystack` and return the value as
/// a UTF-8 string. Lenient about whitespace between attributes (handles
/// the double-space pattern Spectrasonics uses) and stops at the first
/// unescaped `"`. Returns `None` if either the tag or the attribute
/// isn't found.
/// Every occurrence of `<tag … attr="value"` in document order. The
/// attribute is searched only up to the tag's closing `>` so one tag's
/// attributes never bleed into the next match.
fn find_all_xml_attr(haystack: &[u8], tag: &[u8], attr: &[u8]) -> Vec<String> {
    let mut needle = Vec::with_capacity(attr.len() + 2);
    needle.extend_from_slice(attr);
    needle.extend_from_slice(b"=\"");

    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(rel) = find_subslice(&haystack[pos..], tag) {
        let tag_pos = pos + rel;
        let scan_from = tag_pos + tag.len();
        let tag_end = haystack[scan_from..]
            .iter()
            .position(|&b| b == b'>')
            .map(|e| scan_from + e)
            .unwrap_or(haystack.len());
        if let Some(attr_rel) = find_subslice(&haystack[scan_from..tag_end], &needle) {
            let value_start = scan_from + attr_rel + needle.len();
            if let Some(end_rel) = haystack[value_start..].iter().position(|&b| b == b'"') {
                out.push(String::from_utf8_lossy(&haystack[value_start..value_start + end_rel]).into_owned());
            }
        }
        pos = scan_from;
    }
    out
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_keyscape_patch_name_from_real_fixture() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent() // src-tauri
            .and_then(|p| p.parent()) // desktop
            .and_then(|p| p.parent()) // packages
            .and_then(|p| p.parent()) // repo root
            .map(|p| p.join("crates/moonlitt-vst3/tests/fixtures/keyscape-default.mlstate"));
        let Some(path) = path else {
            return;
        };
        if !path.exists() {
            return;
        }
        let bytes = std::fs::read(&path).unwrap();
        let name = extract_patch_name(&bytes).expect("should extract patch name");
        assert_eq!(name, "LA Custom C7 - Natural");
    }

    #[test]
    fn extracts_pianoteq_preset_name() {
        // Shape of a Pianoteq VST3 state: VstW/fxChunk framing, then a
        // `tdtM` metadata chunk holding the preset name, vendor
        // "Modartt" and a description.
        let mut state = b"MLST....VstW....CcnK....FBCh....Pt9q....".to_vec();
        state.extend_from_slice(b"tdtM");
        state.extend_from_slice(&[0u8; 8]);
        state.extend_from_slice(b"NY Steinway D Classical\0\0\0");
        state.extend_from_slice(b"Modartt\0This preset offers...");
        assert_eq!(
            extract_patch_name(&state).as_deref(),
            Some("NY Steinway D Classical")
        );
    }

    #[test]
    fn pianoteq_parser_needs_both_markers() {
        // "tdtM" alone (random binary collision) must not produce junk.
        let state = b"....tdtM\0\0\0\0Garbage Name\0....";
        assert!(extract_patch_name(state).is_none());
    }

    /// Real-plugin check: the heuristic parser must survive an actual
    /// Pianoteq state, not just the synthetic fixture. Skips when
    /// Pianoteq isn't installed.
    #[test]
    fn extracts_name_from_real_pianoteq_state() {
        let Some(path) = moonlitt_vst3::Vst3Host::new(48_000, 512)
            .ok()
            .and_then(|h| h.scan().ok())
            .and_then(|ps| ps.into_iter().find(|p| p.name.starts_with("Pianoteq")))
            .map(|p| p.path)
        else {
            eprintln!("Pianoteq not installed — skipping");
            return;
        };
        let host = moonlitt_vst3::Vst3Host::new(48_000, 512).unwrap();
        let plugin = host.load_from_path(&path).expect("load Pianoteq");
        let state = plugin.get_state().expect("get_state");
        let name = extract_patch_name(&state);
        assert!(
            name.as_deref().is_some_and(|n| n.len() >= 3),
            "no preset name parsed from real Pianoteq state: {name:?}"
        );
        eprintln!("Pianoteq preset name: {name:?}");
    }

    #[test]
    fn returns_none_for_random_bytes() {
        let bytes = vec![0xCD; 4096];
        assert!(extract_patch_name(&bytes).is_none());
    }

    #[test]
    fn returns_none_for_non_spectrasonics_xml() {
        let xml = b"<vst3:state><some other plugin/></vst3:state>";
        assert!(extract_patch_name(xml).is_none());
    }

    #[test]
    fn finds_attr_with_extra_whitespace() {
        let xml = b"<SYNTHENG  Vers=\"29\"  origPatchName=\"Bright Steinway\"  more=\"x\" >";
        let v = find_all_xml_attr(xml, b"<SYNTHENG", b"origPatchName");
        assert_eq!(v.first().map(String::as_str), Some("Bright Steinway"));
    }

    #[test]
    fn entrydescr_name_takes_precedence_over_syntheng() {
        let xml = br#"<SynthMaster vers="1.5">
            <ENTRYDESCR name="User Custom Patch" library="x">
            <SYNTHENG origPatchName="Original Name">
        "#;
        assert_eq!(
            extract_patch_name(xml).as_deref(),
            Some("User Custom Patch")
        );
    }

    #[test]
    fn skips_default_master_descriptor_and_reads_part_descriptor() {
        // Shape of a state assembled from a library patch: the master
        // ENTRYDESCR stays "default", origPatchName is empty (library
        // files never carry it), and the real name lives on part 0's own
        // ENTRYDESCR further in.
        let xml = br#"<SynthMaster vers="1.5">
            <ENTRYDESCR name="default" library="">
            <SYNTHENG origPatchName="">
            <SynthSubEngine><ENTRYDESCR name="Clavinet C - Brite Rhythm" library="Keyscape Library"></SynthSubEngine>
        "#;
        assert_eq!(
            extract_patch_name(xml).as_deref(),
            Some("Clavinet C - Brite Rhythm")
        );
    }

    #[test]
    fn falls_through_to_syntheng_when_entrydescr_says_default() {
        // Spectrasonics writes name="default" when the user hasn't picked
        // a patch yet — that's not useful, fall through to origPatchName.
        let xml = br#"<SynthMaster vers="1.5">
            <ENTRYDESCR name="default" library="Omnisphere Library">
            <SYNTHENG origPatchName="LA Custom C7 Grand">
        "#;
        assert_eq!(
            extract_patch_name(xml).as_deref(),
            Some("LA Custom C7 Grand")
        );
    }
}
