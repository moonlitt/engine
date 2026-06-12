//! SF2 sample pool — loads and indexes all samples from an SF2 file.

use soundfont::raw::GeneratorType;
use soundfont::SoundFont2;
use std::sync::Arc;

/// Information about a single sample, ready for voice playback.
#[derive(Debug, Clone)]
pub struct SampleInfo {
    pub name: String,
    /// Root key (MIDI note at which sample plays at original pitch).
    pub root_key: u8,
    /// Pitch correction in cents.
    pub pitch_correction: i8,
    /// Original sample rate.
    pub sample_rate: u32,
    /// Sample data (i16 converted to f32, normalized to -1.0..1.0).
    pub data: Arc<[f32]>,
    /// Loop start (in samples from data start).
    pub loop_start: u32,
    /// Loop end (in samples from data start).
    pub loop_end: u32,
}

impl SampleInfo {
    pub fn len(&self) -> usize {
        self.data.len()
    }
}

/// Holds all samples and preset/instrument mappings from an SF2 file.
pub struct SamplePool {
    samples: Vec<SampleInfo>,
    sf2: SoundFont2,
}

impl SamplePool {
    /// Load from an SF2 file path.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let sf2 = SoundFont2::load(&mut file).map_err(|e| format!("{e:?}"))?;

        // Read raw sample data from file
        let raw_data = Self::read_sample_data(&mut file, &sf2)?;

        // Build sample info for each sample header
        let samples: Vec<SampleInfo> = sf2
            .sample_headers
            .iter()
            .filter(|h| h.start < h.end && h.end <= raw_data.len() as u32)
            .map(|h| {
                let start = h.start as usize;
                let end = h.end as usize;
                let data: Vec<f32> = raw_data[start..end]
                    .iter()
                    .map(|&s| s as f32 / 32768.0)
                    .collect();

                SampleInfo {
                    name: h.name.clone(),
                    root_key: h.origpitch,
                    pitch_correction: h.pitchadj,
                    sample_rate: h.sample_rate,
                    data: data.into(),
                    loop_start: h.loop_start.saturating_sub(h.start),
                    loop_end: h.loop_end.saturating_sub(h.start),
                }
            })
            .collect();

        Ok(Self { samples, sf2 })
    }

    fn read_sample_data(file: &mut std::fs::File, sf2: &SoundFont2) -> Result<Vec<i16>, String> {
        use std::io::{Read, Seek, SeekFrom};

        let smpl = sf2.sample_data.smpl.ok_or("No sample data in SF2")?;

        file.seek(SeekFrom::Start(smpl.offset))
            .map_err(|e| e.to_string())?;

        let num_samples = smpl.len as usize / 2;
        let mut data = vec![0i16; num_samples];

        let byte_slice = unsafe {
            std::slice::from_raw_parts_mut(data.as_mut_ptr() as *mut u8, num_samples * 2)
        };
        file.read_exact(byte_slice).map_err(|e| e.to_string())?;

        // Handle endianness
        if cfg!(target_endian = "big") {
            for s in data.iter_mut() {
                *s = i16::from_le(*s);
            }
        }

        Ok(data)
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    pub fn preset_count(&self) -> usize {
        self.sf2.presets.len()
    }

    /// Find the sample for a given preset (bank + program), MIDI note, and velocity.
    /// Follows the SF2 preset → instrument → sample zone chain.
    pub fn find_sample(
        &self,
        bank: u16,
        program: u8,
        note: u8,
        velocity: u8,
    ) -> Option<SampleInfo> {
        // Find preset by bank + program
        let preset = self
            .sf2
            .presets
            .iter()
            .find(|p| p.header.bank == bank && p.header.preset == program as u16)?;

        // Search preset zones for matching key/velocity range
        for pzone in &preset.zones {
            if !zone_matches(&pzone.gen_list, note, velocity) {
                continue;
            }

            // Find instrument reference
            let inst_id = find_gen_u16(&pzone.gen_list, GeneratorType::Instrument);
            let inst_id = match inst_id {
                Some(id) => id as usize,
                None => continue,
            };

            let instrument = match self.sf2.instruments.get(inst_id) {
                Some(i) => i,
                None => continue,
            };

            // Search instrument zones
            for izone in &instrument.zones {
                if !zone_matches(&izone.gen_list, note, velocity) {
                    continue;
                }

                // Find sample reference
                let sample_id = find_gen_u16(&izone.gen_list, GeneratorType::SampleID);
                let sample_id = match sample_id {
                    Some(id) => id as usize,
                    None => continue,
                };

                if let Some(sample) = self.samples.get(sample_id) {
                    let mut result = sample.clone();
                    // Check for overriding root key
                    if let Some(key) =
                        find_gen_i16(&izone.gen_list, GeneratorType::OverridingRootKey)
                    {
                        if (0..=127).contains(&key) {
                            result.root_key = key as u8;
                        }
                    }
                    return Some(result);
                }
            }
        }

        None
    }
}

/// Check if a zone's key/velocity range matches the given note and velocity.
fn zone_matches(gen_list: &[soundfont::raw::Generator], note: u8, velocity: u8) -> bool {
    for gen in gen_list {
        if matches_gen_type(&gen.ty, GeneratorType::KeyRange) {
            if let Some(range) = gen.amount.as_range() {
                if note < range.low || note > range.high {
                    return false;
                }
            }
        }
        if matches_gen_type(&gen.ty, GeneratorType::VelRange) {
            if let Some(range) = gen.amount.as_range() {
                if velocity < range.low || velocity > range.high {
                    return false;
                }
            }
        }
    }
    true
}

fn matches_gen_type(sf: &soundfont::SfEnum<GeneratorType, u16>, target: GeneratorType) -> bool {
    match sf {
        soundfont::SfEnum::Value(v) => *v == target,
        _ => false,
    }
}

fn find_gen_u16(gen_list: &[soundfont::raw::Generator], target: GeneratorType) -> Option<u16> {
    gen_list
        .iter()
        .find(|g| matches_gen_type(&g.ty, target))
        .and_then(|g| g.amount.as_u16().copied())
}

fn find_gen_i16(gen_list: &[soundfont::raw::Generator], target: GeneratorType) -> Option<i16> {
    gen_list
        .iter()
        .find(|g| matches_gen_type(&g.ty, target))
        .and_then(|g| g.amount.as_i16().copied())
}
