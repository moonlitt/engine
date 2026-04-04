/// Musical note value for tempo-synced LFO rates.
///
/// 17 divisions from 1/32 to 4 bars, including triplet and dotted variants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoteValue {
    ThirtySecond,
    SixteenthTriplet,
    Sixteenth,
    DottedSixteenth,
    EighthTriplet,
    Eighth,
    DottedEighth,
    QuarterTriplet,
    Quarter,
    DottedQuarter,
    HalfTriplet,
    Half,
    DottedHalf,
    WholeTriplet,
    Whole,
    TwoBar,
    FourBar,
}

impl NoteValue {
    /// Duration in milliseconds at the given BPM.
    ///
    /// Quarter note = one beat = 60000 / bpm ms.
    pub fn to_ms(self, bpm: f64) -> f64 {
        let beat_ms = 60_000.0 / bpm;
        let multiplier = match self {
            Self::ThirtySecond => 0.125,
            Self::SixteenthTriplet => 0.25 * (2.0 / 3.0),
            Self::Sixteenth => 0.25,
            Self::DottedSixteenth => 0.25 * 1.5,
            Self::EighthTriplet => 0.5 * (2.0 / 3.0),
            Self::Eighth => 0.5,
            Self::DottedEighth => 0.5 * 1.5,
            Self::QuarterTriplet => 2.0 / 3.0,
            Self::Quarter => 1.0,
            Self::DottedQuarter => 1.5,
            Self::HalfTriplet => 2.0 * (2.0 / 3.0),
            Self::Half => 2.0,
            Self::DottedHalf => 2.0 * 1.5,
            Self::WholeTriplet => 4.0 * (2.0 / 3.0),
            Self::Whole => 4.0,
            Self::TwoBar => 8.0,
            Self::FourBar => 16.0,
        };
        beat_ms * multiplier
    }

    /// Frequency in Hz at the given BPM.
    pub fn to_hz(self, bpm: f64) -> f64 {
        1000.0 / self.to_ms(bpm)
    }

    /// Convert a parameter index (0..=16) to a `NoteValue`.
    ///
    /// Out-of-range values are clamped to `FourBar`.
    pub fn from_index(i: u32) -> Self {
        match i {
            0 => Self::ThirtySecond,
            1 => Self::SixteenthTriplet,
            2 => Self::Sixteenth,
            3 => Self::DottedSixteenth,
            4 => Self::EighthTriplet,
            5 => Self::Eighth,
            6 => Self::DottedEighth,
            7 => Self::QuarterTriplet,
            8 => Self::Quarter,
            9 => Self::DottedQuarter,
            10 => Self::HalfTriplet,
            11 => Self::Half,
            12 => Self::DottedHalf,
            13 => Self::WholeTriplet,
            14 => Self::Whole,
            15 => Self::TwoBar,
            _ => Self::FourBar,
        }
    }
}

/// LFO waveform shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LfoShape {
    Sine,
    Triangle,
    Saw,
    Square,
    SampleAndHold,
}

impl LfoShape {
    /// Convert a parameter index (0..=4) to an `LfoShape`.
    ///
    /// Out-of-range values are clamped to `SampleAndHold`.
    pub fn from_index(i: u32) -> Self {
        match i {
            0 => Self::Sine,
            1 => Self::Triangle,
            2 => Self::Saw,
            3 => Self::Square,
            _ => Self::SampleAndHold,
        }
    }
}

/// Low-frequency oscillator with 5 waveform shapes and tempo sync.
///
/// Output range is always \[-1, 1\]. Used as a modulation source by
/// chorus, flanger, phaser, tremolo, and delay effects.
pub struct Lfo {
    phase: f64,
    sample_rate: f64,
    shape: LfoShape,
    rng_state: u64,
    sh_value: f64,
}

impl Lfo {
    /// Create a new LFO with the given sample rate, defaulting to `Sine`.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            phase: 0.0,
            sample_rate: f64::from(sample_rate),
            shape: LfoShape::Sine,
            // Non-zero seed for xorshift64
            rng_state: 0x5EED_CAFE_BABE_D00D,
            sh_value: 0.0,
        }
    }

    /// Change the waveform shape.
    pub fn set_shape(&mut self, shape: LfoShape) {
        self.shape = shape;
    }

    /// Reset the oscillator phase to zero.
    pub fn reset_phase(&mut self) {
        self.phase = 0.0;
    }

    /// Set the oscillator phase to a specific value in \[0, 1).
    ///
    /// Useful for creating phase-offset LFO pairs (e.g., stereo auto-pan
    /// with the right channel at 0.5 = 180° out of phase).
    pub fn set_phase(&mut self, phase: f64) {
        self.phase = phase.rem_euclid(1.0);
    }

    /// Advance one sample at `freq_hz` and return a value in \[-1, 1\].
    pub fn next(&mut self, freq_hz: f64) -> f64 {
        let value = self.evaluate();
        self.advance(freq_hz);
        value
    }

    /// Tempo-synced version — derives frequency from BPM and note value.
    pub fn next_synced(&mut self, bpm: f64, note: NoteValue) -> f64 {
        self.next(note.to_hz(bpm))
    }

    /// Evaluate the current phase without advancing.
    fn evaluate(&self) -> f64 {
        match self.shape {
            LfoShape::Sine => (self.phase * std::f64::consts::TAU).sin(),
            LfoShape::Triangle => {
                if self.phase < 0.5 {
                    4.0 * self.phase - 1.0
                } else {
                    3.0 - 4.0 * self.phase
                }
            }
            LfoShape::Saw => 2.0 * self.phase - 1.0,
            LfoShape::Square => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoShape::SampleAndHold => self.sh_value,
        }
    }

    /// Advance phase by one sample, handling wrap and S&H trigger.
    fn advance(&mut self, freq_hz: f64) {
        let prev_phase = self.phase;
        self.phase += freq_hz / self.sample_rate;

        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
            self.update_sample_and_hold();
        } else if self.phase < prev_phase {
            // Wrap detected via underflow (shouldn't normally happen, but guard)
            self.update_sample_and_hold();
        }
    }

    /// Generate a new S&H value using xorshift64.
    fn update_sample_and_hold(&mut self) {
        if self.shape == LfoShape::SampleAndHold {
            self.rng_state = xorshift64(self.rng_state);
            // Map u64 to -1..1
            self.sh_value = (self.rng_state as f64) / (u64::MAX as f64) * 2.0 - 1.0;
        }
    }
}

/// xorshift64 PRNG — fast, non-allocating, deterministic.
fn xorshift64(mut state: u64) -> u64 {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_output_range() {
        let mut lfo = Lfo::new(44100);
        for _ in 0..44100 {
            let v = lfo.next(1.0);
            assert!(v >= -1.0 && v <= 1.0, "Sine out of range: {v}");
        }
    }

    #[test]
    fn sine_completes_one_cycle() {
        let mut lfo = Lfo::new(44100);
        let mut crossed_zero_positive = 0;
        let mut prev = lfo.next(1.0);
        for _ in 1..44100 {
            let v = lfo.next(1.0);
            if prev <= 0.0 && v > 0.0 {
                crossed_zero_positive += 1;
            }
            prev = v;
        }
        assert_eq!(
            crossed_zero_positive, 1,
            "1 Hz sine should cross zero upward once per second"
        );
    }

    #[test]
    fn triangle_output_range() {
        let mut lfo = Lfo::new(44100);
        lfo.set_shape(LfoShape::Triangle);
        for _ in 0..44100 {
            let v = lfo.next(2.0);
            assert!(v >= -1.0 && v <= 1.0, "Triangle out of range: {v}");
        }
    }

    #[test]
    fn saw_ramps_up() {
        let mut lfo = Lfo::new(1000);
        lfo.set_shape(LfoShape::Saw);
        let v0 = lfo.next(1.0);
        let v1 = lfo.next(1.0);
        assert!(v1 > v0, "Saw should ramp up: {v0} -> {v1}");
    }

    #[test]
    fn square_is_binary() {
        let mut lfo = Lfo::new(44100);
        lfo.set_shape(LfoShape::Square);
        for _ in 0..44100 {
            let v = lfo.next(1.0);
            assert!(v == 1.0 || v == -1.0, "Square should be ±1, got {v}");
        }
    }

    #[test]
    fn sample_and_hold_holds() {
        let mut lfo = Lfo::new(100);
        lfo.set_shape(LfoShape::SampleAndHold);
        let v0 = lfo.next(1.0); // freq=1Hz, period=100 samples
        let v1 = lfo.next(1.0);
        assert_eq!(v0, v1, "S&H should hold value within same cycle");
    }

    #[test]
    fn note_value_quarter_at_120bpm() {
        let ms = NoteValue::Quarter.to_ms(120.0);
        assert!(
            (ms - 500.0).abs() < 0.01,
            "1/4 @ 120 BPM = 500ms, got {ms}"
        );
    }

    #[test]
    fn note_value_eighth_at_120bpm() {
        let ms = NoteValue::Eighth.to_ms(120.0);
        assert!(
            (ms - 250.0).abs() < 0.01,
            "1/8 @ 120 BPM = 250ms, got {ms}"
        );
    }

    #[test]
    fn note_value_dotted_eighth_at_120bpm() {
        let ms = NoteValue::DottedEighth.to_ms(120.0);
        assert!(
            (ms - 375.0).abs() < 0.01,
            "1/8. @ 120 BPM = 375ms, got {ms}"
        );
    }

    #[test]
    fn note_value_triplet_at_120bpm() {
        let ms = NoteValue::EighthTriplet.to_ms(120.0);
        assert!(
            (ms - 166.667).abs() < 0.1,
            "1/8T @ 120 BPM = 166.67ms, got {ms}"
        );
    }

    #[test]
    fn tempo_sync_matches_free() {
        let mut lfo = Lfo::new(44100);
        let synced = lfo.next_synced(120.0, NoteValue::Quarter);
        let mut lfo2 = Lfo::new(44100);
        let free = lfo2.next(2.0); // Quarter @ 120 BPM = 2 Hz
        assert_eq!(synced, free);
    }

    #[test]
    fn reset_phase_resets() {
        let mut lfo = Lfo::new(44100);
        for _ in 0..1000 {
            lfo.next(10.0);
        }
        lfo.reset_phase();
        let mut lfo2 = Lfo::new(44100);
        assert_eq!(lfo.next(10.0), lfo2.next(10.0));
    }
}
