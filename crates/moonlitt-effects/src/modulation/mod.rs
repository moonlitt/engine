pub mod lfo;
pub mod delay_line;

#[cfg(feature = "delay")]
pub mod delay;

#[cfg(feature = "chorus")]
pub mod chorus;

#[cfg(feature = "flanger")]
pub mod flanger;

#[cfg(feature = "phaser")]
pub mod phaser;

#[cfg(feature = "tremolo")]
pub mod tremolo;
