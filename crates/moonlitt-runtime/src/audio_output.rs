use crate::audio_thread::AudioThread;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

pub(crate) struct AudioOutput {
    stream: cpal::Stream,
    // AudioThread is behind Arc<Mutex> for start/stop control.
    // The audio callback uses try_lock() to avoid blocking.
    _audio_thread: Arc<Mutex<AudioThread>>,
}

impl AudioOutput {
    pub fn new(audio_thread: AudioThread) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no audio output device")?;

        let sample_rate = audio_thread.engine.sample_rate();
        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let audio_thread = Arc::new(Mutex::new(audio_thread));
        let thread_ref = audio_thread.clone();

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut at) = thread_ref.try_lock() {
                        at.process(data);
                    } else {
                        data.fill(0.0); // silence if locked
                    }
                },
                |err| eprintln!("audio stream error: {err}"),
                None,
            )
            .map_err(|e| e.to_string())?;

        Ok(Self {
            stream,
            _audio_thread: audio_thread,
        })
    }

    pub fn start(&self) -> Result<(), String> {
        self.stream.play().map_err(|e| e.to_string())
    }

    pub fn pause(&self) -> Result<(), String> {
        self.stream.pause().map_err(|e| e.to_string())
    }
}
