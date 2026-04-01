use std::error::Error;

pub type AudioCallback = Box<dyn FnMut(&mut [f32]) + Send>;

pub trait AudioHost: Send {
    fn start(&mut self, callback: AudioCallback) -> Result<(), Box<dyn Error>>;
    fn stop(&mut self);
    fn sample_rate(&self) -> u32;
    fn buffer_size(&self) -> u32;
}
