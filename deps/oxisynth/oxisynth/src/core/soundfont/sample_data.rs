use std::{
    io::{self, Read, Seek, SeekFrom},
    sync::Arc,
};

use soundfont::raw::SampleChunk;

/// Sample data stored as 24-bit values in the upper 24 bits of i32.
/// When sm24 chunk is available: i32 = (smpl_i16 << 8) | sm24_u8
/// When sm24 is absent: i32 = smpl_i16 << 8 (preserves amplitude range)
///
/// DSP code divides by 256.0 to recover the original i16-equivalent amplitude.
#[derive(Debug, Clone)]
pub(crate) struct SampleData {
    data: Arc<[i32]>,
    /// Raw smpl bytes kept for SF3 Vorbis decompression (compressed data round-trip)
    #[cfg(feature = "sf3")]
    raw_bytes: Option<Arc<[u8]>>,
}

impl SampleData {
    #[cfg_attr(not(feature = "sf3"), allow(dead_code))]
    pub fn new(data: Arc<[i32]>) -> Self {
        Self {
            data,
            #[cfg(feature = "sf3")]
            raw_bytes: None,
        }
    }

    pub fn load<F: Read + Seek>(
        file: &mut F,
        smpl: &SampleChunk,
        sm24: Option<&SampleChunk>,
    ) -> io::Result<Self> {
        let sample_pos = smpl.offset;
        let sample_size = smpl.len as usize;
        let num_samples = sample_size / 2;

        // Read 16-bit sample data
        if let Err(err) = file.seek(SeekFrom::Start(sample_pos)) {
            log::error!("Failed to seek position in data file: {err}");
            return Err(err);
        }

        let mut smpl_data = vec![0i16; num_samples];

        {
            let byte_slice = crate::unsafe_stuff::slice_i16_to_u8_mut(&mut smpl_data);

            if let Err(err) = file.read_exact(byte_slice) {
                log::error!("Failed to read sample data: {err}");
                return Err(err);
            }
        }

        // Sample is in LittleEndian so if we are on BigEndian flip the bits around
        if cfg!(target_endian = "big") {
            for n in smpl_data.iter_mut() {
                *n = n.to_le();
            }
        }

        // Keep raw smpl bytes for SF3 Vorbis decompression (compressed data round-trip)
        #[cfg(feature = "sf3")]
        let raw_smpl_bytes: Vec<u8> =
            crate::unsafe_stuff::slice_i16_to_u8(&smpl_data).to_vec();

        // Try to read sm24 chunk (lower 8 bits of 24-bit samples)
        // Per SF2 spec: sm24 must be exactly half the size of smpl (+1 if odd)
        let sm24_bytes = if let Some(sm24_chunk) = sm24 {
            let expected_size = (num_samples + 1) & !1; // round up to even
            if sm24_chunk.len as usize >= num_samples
                && sm24_chunk.len as usize <= expected_size
            {
                if let Err(err) = file.seek(SeekFrom::Start(sm24_chunk.offset)) {
                    log::warn!("Failed to seek sm24 chunk, falling back to 16-bit: {err}");
                    None
                } else {
                    let mut bytes = vec![0u8; num_samples];
                    match file.read_exact(&mut bytes) {
                        Ok(()) => {
                            log::info!("Loaded sm24 chunk: 24-bit sample resolution enabled");
                            Some(bytes)
                        }
                        Err(err) => {
                            log::warn!(
                                "Failed to read sm24 data, falling back to 16-bit: {err}"
                            );
                            None
                        }
                    }
                }
            } else {
                log::warn!(
                    "sm24 chunk size mismatch (expected ~{}, got {}), ignoring sm24",
                    num_samples,
                    sm24_chunk.len
                );
                None
            }
        } else {
            None
        };

        // Merge into i32: upper 24 bits of i32
        let data: Vec<i32> = if let Some(lo_bytes) = sm24_bytes {
            smpl_data
                .iter()
                .zip(lo_bytes.iter())
                .map(|(&hi16, &lo8)| ((hi16 as i32) << 8) | (lo8 as i32))
                .collect()
        } else {
            smpl_data.iter().map(|&s| (s as i32) << 8).collect()
        };

        Ok(Self {
            data: data.into(),
            #[cfg(feature = "sf3")]
            raw_bytes: Some(raw_smpl_bytes.into()),
        })
    }

    /// Returns raw smpl bytes for SF3 Vorbis decompression.
    /// For SF3, the smpl chunk contains compressed Ogg data that must be
    /// accessed as raw bytes, not as converted i32 samples.
    #[cfg(feature = "sf3")]
    pub fn as_byte_slice(&self) -> &[u8] {
        if let Some(ref raw) = self.raw_bytes {
            raw
        } else {
            // Fallback for SampleData::new() (post-decode data, not compressed)
            &[]
        }
    }
}

impl std::ops::Deref for SampleData {
    type Target = [i32];

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
