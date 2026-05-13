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
    extract_spectrasonics(state)
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
    if !find_subslice(state, b"SynthMaster").is_some() {
        return None;
    }

    // Prefer ENTRYDESCR name (current/user-chosen) over origPatchName
    // (the patch's original library name). They usually match; when
    // they differ the user changed something.
    if let Some(name) = find_xml_attr(state, b"<ENTRYDESCR", b"name") {
        if !name.is_empty() && name != "default" {
            return Some(name);
        }
    }
    if let Some(name) = find_xml_attr(state, b"<SYNTHENG", b"origPatchName") {
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

/// Locate `<tag ... attr="value"` in `haystack` and return the value as
/// a UTF-8 string. Lenient about whitespace between attributes (handles
/// the double-space pattern Spectrasonics uses) and stops at the first
/// unescaped `"`. Returns `None` if either the tag or the attribute
/// isn't found.
fn find_xml_attr(haystack: &[u8], tag: &[u8], attr: &[u8]) -> Option<String> {
    let tag_pos = find_subslice(haystack, tag)?;
    // Scan from just after the tag for `attr="`.
    let scan_from = tag_pos + tag.len();
    let mut needle = Vec::with_capacity(attr.len() + 2);
    needle.extend_from_slice(attr);
    needle.extend_from_slice(b"=\"");
    let local = &haystack[scan_from..];
    let attr_pos = find_subslice(local, &needle)?;
    let value_start = scan_from + attr_pos + needle.len();
    // End of attribute = first `"` after value_start.
    let end_rel = haystack[value_start..]
        .iter()
        .position(|&b| b == b'"')?;
    let value = &haystack[value_start..value_start + end_rel];
    Some(String::from_utf8_lossy(value).into_owned())
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
            .map(|p| {
                p.join("crates/moonlitt-vst3/tests/fixtures/keyscape-default.mlstate")
            });
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
        let v = find_xml_attr(xml, b"<SYNTHENG", b"origPatchName");
        assert_eq!(v.as_deref(), Some("Bright Steinway"));
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
