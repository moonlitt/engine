use crate::audio_thread::AudioThread;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub(crate) struct AudioOutput {
    stream: cpal::Stream,
}

impl AudioOutput {
    pub fn new(mut audio_thread: AudioThread) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no audio output device")?;

        let sample_rate = audio_thread.mixer.sample_rate();
        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        // Move AudioThread directly into the cpal closure.
        // The closure owns it exclusively — no sharing needed, no mutex required.
        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    audio_thread.process(data);
                },
                |err| eprintln!("audio stream error: {err}"),
                None,
            )
            .map_err(|e| e.to_string())?;

        Ok(Self { stream })
    }

    pub fn start(&self) -> Result<(), String> {
        self.stream.play().map_err(|e| e.to_string())
    }

    pub fn pause(&self) -> Result<(), String> {
        self.stream.pause().map_err(|e| e.to_string())
    }
}
