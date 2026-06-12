//! Chunked plug-in state container.
//!
//! VST3 plug-ins have TWO independent state stores:
//!
//!   - `IComponent::getState`        — processor / DSP-side state (params, internal buffers)
//!   - `IEditController::getState`   — controller / UI-side state (patch IDs, browser state)
//!
//! Capturing only the component blob loses controller-resident data such as
//! Spectrasonics patch selection — set_state then "succeeds" but the plug-in
//! produces silence. This module defines a tiny self-describing container
//! that holds both blobs side-by-side.
//!
//! Layout (little-endian throughout):
//!
//!   offset 0   : magic         = b"MLST"           (4 bytes)
//!   offset 4   : version       = 1u32              (4 bytes)
//!   offset 8   : component_len = u64               (8 bytes)
//!   offset 16  : component_blob                    (component_len bytes)
//!   offset ... : controller_len = u64              (8 bytes)
//!   offset ... : controller_blob                   (controller_len bytes)
//!
//! Backward compatibility: blobs that don't start with `MLST` are treated as
//! legacy component-only state by the caller.

const MAGIC: &[u8; 4] = b"MLST";
const VERSION: u32 = 1;
const HEADER_LEN: usize = 4 + 4 + 8; // magic + version + component_len

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkedState {
    pub component: Vec<u8>,
    pub controller: Vec<u8>,
}

impl ChunkedState {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(HEADER_LEN + self.component.len() + 8 + self.controller.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.component.len() as u64).to_le_bytes());
        out.extend_from_slice(&self.component);
        out.extend_from_slice(&(self.controller.len() as u64).to_le_bytes());
        out.extend_from_slice(&self.controller);
        out
    }

    /// Parse a chunked state blob. Returns `None` if the blob does not start
    /// with our magic — callers should treat that as a legacy component-only
    /// blob.
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < HEADER_LEN || &data[..4] != MAGIC {
            return None;
        }
        let version = u32::from_le_bytes(data[4..8].try_into().ok()?);
        if version != VERSION {
            return None;
        }
        let comp_len = u64::from_le_bytes(data[8..16].try_into().ok()?) as usize;
        let comp_end = HEADER_LEN.checked_add(comp_len)?;
        if data.len() < comp_end.checked_add(8)? {
            return None;
        }
        let component = data[HEADER_LEN..comp_end].to_vec();
        let ctrl_len = u64::from_le_bytes(data[comp_end..comp_end + 8].try_into().ok()?) as usize;
        let ctrl_end = comp_end.checked_add(8)?.checked_add(ctrl_len)?;
        if data.len() < ctrl_end {
            return None;
        }
        let controller = data[comp_end + 8..ctrl_end].to_vec();
        Some(Self {
            component,
            controller,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_both_chunks() {
        let original = ChunkedState {
            component: vec![1, 2, 3, 4, 5],
            controller: vec![10, 20, 30],
        };
        let bytes = original.to_bytes();
        let parsed = ChunkedState::parse(&bytes).expect("must parse own output");
        assert_eq!(parsed, original);
    }

    #[test]
    fn roundtrip_handles_empty_controller() {
        let original = ChunkedState {
            component: vec![0xAA; 64],
            controller: Vec::new(),
        };
        let bytes = original.to_bytes();
        let parsed = ChunkedState::parse(&bytes).unwrap();
        assert_eq!(parsed.component, original.component);
        assert!(parsed.controller.is_empty());
    }

    #[test]
    fn roundtrip_handles_empty_component() {
        let original = ChunkedState {
            component: Vec::new(),
            controller: vec![0xBB; 32],
        };
        let bytes = original.to_bytes();
        let parsed = ChunkedState::parse(&bytes).unwrap();
        assert!(parsed.component.is_empty());
        assert_eq!(parsed.controller, original.controller);
    }

    #[test]
    fn parse_rejects_legacy_single_blob() {
        // 250 KB of arbitrary bytes that don't begin with MLST.
        let legacy = vec![0xCDu8; 250_000];
        assert!(
            ChunkedState::parse(&legacy).is_none(),
            "legacy single-blob fixtures must NOT be parsed as chunked"
        );
    }

    #[test]
    fn parse_rejects_truncated_blob() {
        let full = ChunkedState {
            component: vec![1; 100],
            controller: vec![2; 50],
        }
        .to_bytes();
        // Cut off mid-controller — parser must refuse.
        let truncated = &full[..full.len() - 10];
        assert!(ChunkedState::parse(truncated).is_none());
    }

    #[test]
    fn parse_rejects_wrong_magic() {
        let mut bytes = ChunkedState {
            component: vec![1],
            controller: vec![2],
        }
        .to_bytes();
        bytes[0] = b'X';
        assert!(ChunkedState::parse(&bytes).is_none());
    }

    #[test]
    fn parse_rejects_future_version() {
        let mut bytes = ChunkedState {
            component: vec![1],
            controller: vec![2],
        }
        .to_bytes();
        // Bump version to 999 — should reject (forward incompatibility is
        // safer than silently misinterpreting).
        bytes[4] = 0xE7;
        bytes[5] = 0x03;
        assert!(ChunkedState::parse(&bytes).is_none());
    }
}
