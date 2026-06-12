//! Spectrasonics (Keyscape / Omnisphere / Trilian) patch-library access.
//!
//! Spectrasonics plug-ins don't expose their patch browser through any
//! VST3 interface — IUnitInfo reports 2048 placeholder slots and the
//! program-change parameters are bare MIDI slots. The factory library
//! lives on disk instead, in STEAM `.db` containers with a plaintext
//! index, and a patch is loaded by *state assembly*: take any captured
//! plug-in state as a wrapper, replace part 0's `<SynthEngine>` subtree
//! with the library patch's, and hand the result to `set_state`.
//!
//! Verified against Keyscape (455 factory patches): the assembled state
//! loads, streams samples, and round-trips the new patch name. See
//! `tests/keyscape_patch_library.rs` for the hardware-gated proof.
//!
//! ## STEAM `.db` container layout
//!
//! ```text
//! <FileSystem>\n
//! <FILE name="Archive.zip" offset="0" size="128624"/>\n
//! <DIR name="Custom Graphics">\n
//!   <FILE name="Background.png" offset="128624" size="10259"/>\n
//! </DIR>\n
//! ...
//! </FileSystem>\n
//! <raw file bytes, at (index_end + offset)>
//! ```
//!
//! Offsets are relative to the first byte after the `</FileSystem>`
//! line. Patch files use per-product extensions: `.prt_key` (Keyscape),
//! `.prt_omn` (Omnisphere), `.prt_trl` (Trilian).
//!
//! ## Plug-in state framing
//!
//! `Vst3Plugin::get_state` wraps both VST3 state stores in our `MLST`
//! container ([`crate::state_format::ChunkedState`]). The component
//! chunk is Spectrasonics' own framing:
//!
//! ```text
//! offset 0  : magic   = 0x3B9AC9FF u32 le
//! offset 4  : 0u32
//! offset 8  : version = 1u32 le
//! offset 12 : 0u32
//! offset 16 : xml_len = u64 le      (includes the trailing NUL)
//! offset 24 : XML document <SynthMaster>…</SynthMaster>\n\0
//! offset .. : trailer (4 zero bytes, preserved verbatim)
//! ```

use crate::{Error, Result};

const FILESYSTEM_CLOSE: &[u8] = b"</FileSystem>";
const SPECTRA_MAGIC: u32 = 0x3B9A_C9FF;
const SPECTRA_HEADER_LEN: usize = 24;

/// One file inside a STEAM `.db` container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbEntry {
    /// Slash-joined directory path inside the container, e.g.
    /// `"Keyboards/Clavinets/Hohner Clavinet C/Clavinet C - Brite Rhythm.prt_key"`.
    pub path: String,
    /// Byte offset relative to the container's data section.
    pub offset: u64,
    /// File size in bytes.
    pub size: u64,
}

/// Parsed `.db` index: the entry list plus the absolute byte offset
/// where the data section starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbIndex {
    pub entries: Vec<DbEntry>,
    pub data_base: usize,
}

/// Parse the plaintext `<FileSystem>` index at the head of a STEAM
/// `.db` container.
pub fn parse_db_index(db: &[u8]) -> Result<DbIndex> {
    let close = find_subslice(db, FILESYSTEM_CLOSE)
        .ok_or_else(|| Error::Other("missing </FileSystem> index terminator".into()))?;
    let index_end = close + FILESYSTEM_CLOSE.len();
    let mut data_base = index_end;
    while matches!(db.get(data_base), Some(b'\n') | Some(b'\r')) {
        data_base += 1;
    }

    let index_text = String::from_utf8_lossy(&db[..index_end]);
    let mut stack: Vec<String> = Vec::new();
    let mut entries = Vec::new();
    for line in index_text.lines() {
        let line = line.trim();
        if let Some(name) = attr_value(line, "<DIR ", "name") {
            stack.push(name);
        } else if line.starts_with("</DIR>") {
            stack.pop();
        } else if line.starts_with("<FILE ") {
            let name = attr_value(line, "<FILE ", "name")
                .ok_or_else(|| Error::Other("FILE entry without name".into()))?;
            let offset = attr_value(line, "<FILE ", "offset")
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| Error::Other("FILE entry without numeric offset".into()))?;
            let size = attr_value(line, "<FILE ", "size")
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| Error::Other("FILE entry without numeric size".into()))?;
            let path = if stack.is_empty() {
                name
            } else {
                format!("{}/{}", stack.join("/"), name)
            };
            entries.push(DbEntry { path, offset, size });
        }
    }
    Ok(DbIndex { entries, data_base })
}

/// Borrow one entry's bytes out of a `.db` container.
pub fn read_db_entry<'a>(db: &'a [u8], index: &DbIndex, entry: &DbEntry) -> Result<&'a [u8]> {
    let start = index
        .data_base
        .checked_add(entry.offset as usize)
        .ok_or_else(|| Error::Other("entry offset overflow".into()))?;
    let end = start
        .checked_add(entry.size as usize)
        .filter(|&e| e <= db.len())
        .ok_or_else(|| Error::Other(format!("entry {} out of bounds", entry.path)))?;
    Ok(&db[start..end])
}

/// Replace part 0's `<SynthEngine>` subtree in a captured plug-in state
/// with the one from a STEAM library patch file (`.prt_key` /
/// `.prt_omn` / `.prt_trl`), producing a state blob that loads the
/// library patch when handed to `set_state`.
///
/// `state` accepts both our `MLST` container and a raw Spectrasonics
/// component blob; the output mirrors the input form (controller chunk
/// preserved verbatim).
pub fn splice_library_patch(state: &[u8], patch_file: &[u8]) -> Result<Vec<u8>> {
    if let Some(chunked) = crate::state_format::ChunkedState::parse(state) {
        let component = splice_component(&chunked.component, patch_file)?;
        return Ok(crate::state_format::ChunkedState {
            component,
            controller: chunked.controller,
        }
        .to_bytes());
    }
    splice_component(state, patch_file)
}

fn splice_component(component: &[u8], patch_file: &[u8]) -> Result<Vec<u8>> {
    if component.len() < SPECTRA_HEADER_LEN {
        return Err(Error::Other("component blob too short".into()));
    }
    let magic = u32::from_le_bytes(component[0..4].try_into().unwrap());
    if magic != SPECTRA_MAGIC {
        return Err(Error::Other(format!(
            "not a Spectrasonics state (magic {magic:#x})"
        )));
    }
    let xml_len = u64::from_le_bytes(component[16..24].try_into().unwrap()) as usize;
    let xml_end = SPECTRA_HEADER_LEN
        .checked_add(xml_len)
        .filter(|&e| e <= component.len())
        .ok_or_else(|| Error::Other("xml_len out of bounds".into()))?;
    let xml = &component[SPECTRA_HEADER_LEN..xml_end];
    let trailer = &component[xml_end..];

    let part_start = find_subslice(xml, b"<SynthSubEngine")
        .ok_or_else(|| Error::Other("state has no <SynthSubEngine> part slot".into()))?;
    let (engine_start, engine_end) = element_span(xml, b"SynthEngine", part_start)
        .ok_or_else(|| Error::Other("part 0 has no <SynthEngine> subtree".into()))?;
    let (patch_engine_start, patch_engine_end) = element_span(patch_file, b"SynthEngine", 0)
        .ok_or_else(|| Error::Other("patch file has no <SynthEngine> subtree".into()))?;

    let mut new_xml =
        Vec::with_capacity(xml.len() - (engine_end - engine_start) + patch_engine_end
            - patch_engine_start);
    new_xml.extend_from_slice(&xml[..engine_start]);
    new_xml.extend_from_slice(&patch_file[patch_engine_start..patch_engine_end]);
    new_xml.extend_from_slice(&xml[engine_end..]);

    let mut out = Vec::with_capacity(SPECTRA_HEADER_LEN + new_xml.len() + trailer.len());
    out.extend_from_slice(&component[..16]);
    out.extend_from_slice(&(new_xml.len() as u64).to_le_bytes());
    out.extend_from_slice(&new_xml);
    out.extend_from_slice(trailer);
    Ok(out)
}

/// Where a library patch's bytes live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchSource {
    /// Inside a STEAM `.db` container.
    Db {
        db_path: std::path::PathBuf,
        entry: DbEntry,
    },
    /// A loose patch file (User patches).
    File(std::path::PathBuf),
}

/// One browsable patch in a product's Settings Library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryPatch {
    /// Display name — the file stem, e.g. `"Clavinet C - Brite Rhythm"`.
    pub name: String,
    /// Slash-joined category path, e.g. `"Keyboards/Clavinets/Hohner Clavinet C"`.
    pub category: String,
    /// Library this patch belongs to — the `.db` stem (`"Keyscape Library"`)
    /// or `"User"` for loose files.
    pub library: String,
    pub source: PatchSource,
}

/// Patch extension a given Spectrasonics product loads. Products only
/// read their own format — e.g. the `Keyscape Creative` library that
/// ships inside `STEAM/Keyscape` is 1271 `.prt_omn` files usable only
/// from Omnisphere; Keyscape's own browser hides them.
pub fn product_patch_extension(product_name: &str) -> Option<&'static str> {
    match product_name.to_ascii_lowercase().as_str() {
        "keyscape" => Some("prt_key"),
        "omnisphere" => Some("prt_omn"),
        "trilian" => Some("prt_trl"),
        _ => None,
    }
}

fn has_extension(name: &str, ext: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(&format!(".{ext}"))
}

fn split_patch_path(path: &str) -> (String, String) {
    let (dir, file) = match path.rfind('/') {
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("", path),
    };
    let name = file.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(file);
    (dir.to_string(), name.to_string())
}

/// Enumerate every patch with extension `ext` (no leading dot — see
/// [`product_patch_extension`]) under a STEAM product directory:
/// factory `.db` containers plus loose User patch files. Returns
/// patches sorted by category then name.
pub fn scan_patch_library(product_dir: &std::path::Path, ext: &str) -> Result<Vec<LibraryPatch>> {
    let patches_dir = product_dir.join("Settings Library").join("Patches");
    let mut out = Vec::new();

    let factory = patches_dir.join("Factory");
    if let Ok(read) = std::fs::read_dir(&factory) {
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("db") {
                continue;
            }
            let library = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Factory")
                .to_string();
            let bytes = std::fs::read(&path)
                .map_err(|e| Error::Other(format!("read {}: {e}", path.display())))?;
            let index = parse_db_index(&bytes)?;
            for db_entry in &index.entries {
                if !has_extension(&db_entry.path, ext) {
                    continue;
                }
                let (category, name) = split_patch_path(&db_entry.path);
                out.push(LibraryPatch {
                    name,
                    category,
                    library: library.clone(),
                    source: PatchSource::Db {
                        db_path: path.clone(),
                        entry: db_entry.clone(),
                    },
                });
            }
        }
    }

    let user = patches_dir.join("User");
    let mut stack = vec![user.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !has_extension(file_name, ext) {
                continue;
            }
            let rel = path
                .strip_prefix(&user)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let (category, name) = split_patch_path(&rel);
            out.push(LibraryPatch {
                name,
                category,
                library: "User".to_string(),
                source: PatchSource::File(path),
            });
        }
    }

    out.sort_by(|a, b| {
        (&a.library, &a.category, &a.name).cmp(&(&b.library, &b.category, &b.name))
    });
    Ok(out)
}

/// Read a patch's raw bytes regardless of where it lives.
pub fn load_patch_bytes(patch: &LibraryPatch) -> Result<Vec<u8>> {
    match &patch.source {
        PatchSource::Db { db_path, entry } => {
            let bytes = std::fs::read(db_path)
                .map_err(|e| Error::Other(format!("read {}: {e}", db_path.display())))?;
            let index = parse_db_index(&bytes)?;
            read_db_entry(&bytes, &index, entry).map(|b| b.to_vec())
        }
        PatchSource::File(path) => {
            std::fs::read(path).map_err(|e| Error::Other(format!("read {}: {e}", path.display())))
        }
    }
}

/// Existing STEAM root directories on this machine, across the
/// standard install locations.
pub fn steam_roots() -> Vec<std::path::PathBuf> {
    let candidates = [
        dirs_home().map(|h| h.join("Library/Application Support/Spectrasonics/STEAM")),
        Some(std::path::PathBuf::from(
            "/Users/Shared/Spectrasonics/STEAM",
        )),
        Some(std::path::PathBuf::from(
            "/Library/Application Support/Spectrasonics/STEAM",
        )),
    ];
    candidates
        .into_iter()
        .flatten()
        .filter(|p| p.is_dir())
        .collect()
}

/// Every product directory under every STEAM root (`…/STEAM/Keyscape`,
/// `…/STEAM/Omnisphere`, …). Cross-product libraries make scanning all
/// of them necessary: `Keyscape Creative` (1271 Omnisphere patches)
/// ships inside the *Keyscape* product directory.
pub fn steam_product_dirs() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for root in steam_roots() {
        let Ok(read) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.push(path);
            }
        }
    }
    out
}

/// Locate the STEAM product directory for a plug-in, by display name
/// (`"Keyscape"` → `…/STEAM/Keyscape`). Returns `None` when the
/// product isn't installed.
pub fn steam_product_dir(plugin_name: &str) -> Option<std::path::PathBuf> {
    steam_roots()
        .into_iter()
        .map(|root| root.join(plugin_name))
        .find(|dir| dir.is_dir())
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

/// Byte span `[start, end)` of the first `<tag …>…</tag>` element at or
/// after `from`, tracking nesting depth so an inner element with the
/// same tag name doesn't terminate the span early. Returns `None` when
/// the element is absent or unbalanced.
fn element_span(doc: &[u8], tag: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut open = Vec::with_capacity(tag.len() + 1);
    open.push(b'<');
    open.extend_from_slice(tag);
    let mut close = Vec::with_capacity(tag.len() + 2);
    close.extend_from_slice(b"</");
    close.extend_from_slice(tag);

    // First occurrence at/after `from` whose token ends at a tag-name
    // boundary — skips longer tag names sharing the prefix (e.g.
    // `<SynthEngineExtra>` when searching for `<SynthEngine>`).
    fn find_tag_token(doc: &[u8], needle: &[u8], mut from: usize) -> Option<usize> {
        loop {
            let i = find_subslice_from(doc, needle, from)?;
            match doc.get(i + needle.len()) {
                Some(c) if c.is_ascii_alphanumeric() || *c == b'_' => from = i + 1,
                _ => return Some(i),
            }
        }
    }

    let mut pos = from;
    let mut depth = 0usize;
    let mut span_start = None;
    while pos < doc.len() {
        let next_open = find_tag_token(doc, &open, pos);
        let next_close = find_tag_token(doc, &close, pos);
        let open_comes_first = match (next_open, next_close) {
            (None, None) => return None,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some(o), Some(c)) => o < c,
        };
        if open_comes_first {
            let o = next_open.expect("checked above");
            if span_start.is_none() {
                span_start = Some(o);
            }
            depth += 1;
            pos = o + open.len();
        } else {
            let c = next_close.expect("checked above");
            if depth == 0 {
                return None; // close before any open
            }
            depth -= 1;
            let elem_end = find_subslice_from(doc, b">", c)? + 1;
            if depth == 0 {
                return Some((span_start?, elem_end));
            }
            pos = c + close.len();
        }
    }
    None
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    find_subslice_from(haystack, needle, 0)
}

fn find_subslice_from(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from > haystack.len() || needle.is_empty() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

/// Pull `attr="value"` off an XML-ish line that starts with `prefix`.
fn attr_value(line: &str, prefix: &str, attr: &str) -> Option<String> {
    if !line.starts_with(prefix) {
        return None;
    }
    let needle = format!("{attr}=\"");
    let start = line.find(&needle)? + needle.len();
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_db_index -------------------------------------------------

    fn synthetic_db() -> Vec<u8> {
        let index = "<FileSystem>\n\
            <FILE name=\"Archive.zip\" offset=\"0\" size=\"4\"/>\n\
            <DIR name=\"Keyboards\">\n\
            <DIR name=\"Clavinets\">\n\
            <FILE name=\"Clav A.prt_key\" offset=\"4\" size=\"7\"/>\n\
            </DIR>\n\
            <FILE name=\"Piano.prt_key\" offset=\"11\" size=\"5\"/>\n\
            </DIR>\n\
            </FileSystem>\n";
        let mut db = index.as_bytes().to_vec();
        db.extend_from_slice(b"ZIP!");
        db.extend_from_slice(b"clav-a!");
        db.extend_from_slice(b"piano");
        db
    }

    #[test]
    fn db_index_walks_dir_tree_into_slash_paths() {
        let db = synthetic_db();
        let index = parse_db_index(&db).expect("parse");
        let paths: Vec<&str> = index.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "Archive.zip",
                "Keyboards/Clavinets/Clav A.prt_key",
                "Keyboards/Piano.prt_key",
            ]
        );
    }

    #[test]
    fn db_entry_bytes_resolve_relative_to_data_base() {
        let db = synthetic_db();
        let index = parse_db_index(&db).expect("parse");
        let clav = &index.entries[1];
        assert_eq!(read_db_entry(&db, &index, clav).expect("read"), b"clav-a!");
        let piano = &index.entries[2];
        assert_eq!(read_db_entry(&db, &index, piano).expect("read"), b"piano");
    }

    #[test]
    fn db_entry_out_of_bounds_is_an_error_not_a_panic() {
        let db = synthetic_db();
        let index = parse_db_index(&db).expect("parse");
        let bogus = DbEntry {
            path: "x".into(),
            offset: 0,
            size: u64::MAX,
        };
        assert!(read_db_entry(&db, &index, &bogus).is_err());
    }

    #[test]
    fn db_without_index_terminator_is_an_error() {
        assert!(parse_db_index(b"<FileSystem>\n<FILE ...").is_err());
    }

    // --- element_span ----------------------------------------------------

    #[test]
    fn element_span_tracks_nested_same_name_tags() {
        let doc = b"<A><SynthEngine x=\"1\"><SynthEngine/></SynthEngine><B/></A>";
        // NOTE: self-closing same-name tags aren't a thing in Spectrasonics
        // docs; nesting is via full open/close pairs. Use a full pair here.
        let doc2 = b"<A><SynthEngine x=\"1\"><SynthEngine></SynthEngine></SynthEngine><B/></A>";
        let _ = doc;
        let (s, e) = element_span(doc2, b"SynthEngine", 0).expect("span");
        assert_eq!(&doc2[s..e], &b"<SynthEngine x=\"1\"><SynthEngine></SynthEngine></SynthEngine>"[..]);
    }

    #[test]
    fn element_span_ignores_longer_tag_names_sharing_the_prefix() {
        // <SynthEngineExtra> must not be mistaken for <SynthEngine>.
        let doc = b"<SynthEngineExtra></SynthEngineExtra><SynthEngine>x</SynthEngine>";
        let (s, e) = element_span(doc, b"SynthEngine", 0).expect("span");
        assert_eq!(&doc[s..e], &b"<SynthEngine>x</SynthEngine>"[..]);
    }

    #[test]
    fn element_span_absent_returns_none() {
        assert!(element_span(b"<Other/>", b"SynthEngine", 0).is_none());
    }

    // --- splice_library_patch ---------------------------------------------

    fn wrap_component(xml: &[u8]) -> Vec<u8> {
        let mut c = Vec::new();
        c.extend_from_slice(&SPECTRA_MAGIC.to_le_bytes());
        c.extend_from_slice(&0u32.to_le_bytes());
        c.extend_from_slice(&1u32.to_le_bytes());
        c.extend_from_slice(&0u32.to_le_bytes());
        c.extend_from_slice(&(xml.len() as u64).to_le_bytes());
        c.extend_from_slice(xml);
        c.extend_from_slice(&[0, 0, 0, 0]); // trailer
        c
    }

    fn synthetic_state_xml() -> &'static [u8] {
        b"<SynthMaster>\n\
          <MasterBlock gain=\"1\"/>\n\
          <SynthSubEngine>\n<SynthEngine old=\"yes\"><OLD/></SynthEngine>\n</SynthSubEngine>\n\
          <SynthSubEngine>\n<SynthEngine empty=\"1\"></SynthEngine>\n</SynthSubEngine>\n\
          </SynthMaster>\n\0"
    }

    const PATCH: &[u8] =
        b"<KeyscapePart>\n<SynthEngine new=\"yes\"><NEW/></SynthEngine>\n</KeyscapePart>\n";

    #[test]
    fn splice_replaces_only_part0_engine() {
        let component = wrap_component(synthetic_state_xml());
        let out = splice_component(&component, PATCH).expect("splice");

        let xml_len = u64::from_le_bytes(out[16..24].try_into().unwrap()) as usize;
        let xml = &out[24..24 + xml_len];
        let text = String::from_utf8_lossy(xml);
        assert!(text.contains("<SynthEngine new=\"yes\"><NEW/></SynthEngine>"), "{text}");
        assert!(!text.contains("old=\"yes\""), "part0 engine should be gone: {text}");
        assert!(text.contains("empty=\"1\""), "part1 must stay untouched: {text}");
        assert!(text.contains("<MasterBlock gain=\"1\"/>"), "master block must survive");
        // trailer preserved
        assert_eq!(&out[24 + xml_len..], &[0, 0, 0, 0]);
    }

    #[test]
    fn splice_roundtrips_mlst_container_and_preserves_controller() {
        let component = wrap_component(synthetic_state_xml());
        let chunked = crate::state_format::ChunkedState {
            component,
            controller: b"CTRL".to_vec(),
        };
        let out = splice_library_patch(&chunked.to_bytes(), PATCH).expect("splice");
        let parsed = crate::state_format::ChunkedState::parse(&out).expect("still MLST");
        assert_eq!(parsed.controller, b"CTRL");
        assert!(String::from_utf8_lossy(&parsed.component).contains("new=\"yes\""));
    }

    #[test]
    fn splice_rejects_non_spectrasonics_state() {
        let err = splice_library_patch(b"\x00\x01\x02\x03 not spectra", PATCH);
        assert!(err.is_err());
    }

    #[test]
    fn splice_rejects_patch_without_engine() {
        let component = wrap_component(synthetic_state_xml());
        assert!(splice_component(&component, b"<KeyscapePart></KeyscapePart>").is_err());
    }

    // --- scan_patch_library -------------------------------------------------

    /// A product's browser must only list its own patch format. The
    /// real-world case: `STEAM/Keyscape/...Factory/Keyscape Creative.db`
    /// holds 1271 `.prt_omn` (Omnisphere-only) patches — the Keyscape
    /// plug-in can't load them and its own browser hides them.
    #[test]
    fn scan_filters_by_product_extension() {
        let root = std::env::temp_dir().join(format!(
            "moonlitt-spectra-ext-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let patches = root.join("Settings Library/Patches");
        std::fs::create_dir_all(patches.join("Factory")).unwrap();
        std::fs::create_dir_all(patches.join("User")).unwrap();
        let mixed_db = {
            let index = "<FileSystem>\n\
                <FILE name=\"A Piano.prt_key\" offset=\"0\" size=\"1\"/>\n\
                <FILE name=\"A Guitar.prt_omn\" offset=\"1\" size=\"1\"/>\n\
                </FileSystem>\n";
            let mut db = index.as_bytes().to_vec();
            db.extend_from_slice(b"kg");
            db
        };
        std::fs::write(patches.join("Factory/Mixed.db"), mixed_db).unwrap();
        std::fs::write(patches.join("User/Mine.prt_omn"), b"x").unwrap();

        let keyscape_view = scan_patch_library(&root, "prt_key").expect("scan key");
        let omni_view = scan_patch_library(&root, "prt_omn").expect("scan omn");
        std::fs::remove_dir_all(&root).ok();

        let names = |v: &[LibraryPatch]| v.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&keyscape_view), vec!["A Piano"]);
        assert_eq!(names(&omni_view), vec!["A Guitar", "Mine"]);
    }

    #[test]
    fn product_extensions_map_the_family() {
        assert_eq!(product_patch_extension("Keyscape"), Some("prt_key"));
        assert_eq!(product_patch_extension("omnisphere"), Some("prt_omn"));
        assert_eq!(product_patch_extension("Trilian"), Some("prt_trl"));
        assert_eq!(product_patch_extension("Surge"), None);
    }

    #[test]
    fn scan_finds_factory_db_patches_and_loose_user_patches() {
        let root = std::env::temp_dir().join(format!(
            "moonlitt-spectra-scan-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let patches = root.join("Settings Library/Patches");
        std::fs::create_dir_all(patches.join("Factory")).unwrap();
        std::fs::create_dir_all(patches.join("User/My Sounds")).unwrap();
        std::fs::write(patches.join("Factory/Test Library.db"), synthetic_db()).unwrap();
        std::fs::write(patches.join("User/My Sounds/Custom Clav.prt_key"), b"x").unwrap();

        let found = scan_patch_library(&root, "prt_key").expect("scan");
        std::fs::remove_dir_all(&root).ok();

        let summary: Vec<(String, String, String)> = found
            .iter()
            .map(|p| (p.library.clone(), p.category.clone(), p.name.clone()))
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "Test Library".into(),
                    "Keyboards".into(),
                    "Piano".into()
                ),
                (
                    "Test Library".into(),
                    "Keyboards/Clavinets".into(),
                    "Clav A".into()
                ),
                ("User".into(), "My Sounds".into(), "Custom Clav".into()),
            ]
        );
        // Archive.zip must not appear — only patch extensions count.
        assert!(found.iter().all(|p| !p.name.contains("Archive")));
    }

    #[test]
    fn load_patch_bytes_resolves_both_sources() {
        let root = std::env::temp_dir().join(format!(
            "moonlitt-spectra-load-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let patches = root.join("Settings Library/Patches");
        std::fs::create_dir_all(patches.join("Factory")).unwrap();
        std::fs::create_dir_all(patches.join("User")).unwrap();
        std::fs::write(patches.join("Factory/Lib.db"), synthetic_db()).unwrap();
        std::fs::write(patches.join("User/Mine.prt_key"), b"loose bytes").unwrap();

        let found = scan_patch_library(&root, "prt_key").expect("scan");
        let clav = found.iter().find(|p| p.name == "Clav A").expect("db patch");
        let mine = found.iter().find(|p| p.name == "Mine").expect("user patch");
        let clav_bytes = load_patch_bytes(clav).expect("db bytes");
        let mine_bytes = load_patch_bytes(mine).expect("file bytes");
        std::fs::remove_dir_all(&root).ok();

        assert_eq!(clav_bytes, b"clav-a!");
        assert_eq!(mine_bytes, b"loose bytes");
    }
}
