pub mod delay;
pub mod delay_line;
pub mod lfo;
pub mod tremolo;

pub use delay::StereoDelay;
pub use delay_line::FractionalDelayLine;
pub use lfo::{Lfo, LfoShape, NoteValue};
pub use tremolo::Tremolo;
