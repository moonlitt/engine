use moonlitt_session::AudioThread;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub(crate) struct AudioOutput {
    stream: cpal::Stream,
}

impl AudioOutput {
    /// Check that an audio output device exists and negotiate a compatible
    /// stream config. Call this BEFORE consuming resources to fail fast.
    pub fn pre_check(desired_rate: u32) -> Result<(), String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no audio output device")?;
        Self::negotiate_config(&device, desired_rate)?;
        Ok(())
    }

    /// Create a new audio output stream. The `AudioThread` is moved into the
    /// cpal callback closure — the caller should call `pre_check` first to
    /// avoid losing resources on predictable failures.
    pub fn new(mut audio_thread: AudioThread) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no audio output device")?;

        let desired_rate = audio_thread.mixer.sample_rate();
        let config = Self::negotiate_config(&device, desired_rate)?;

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
            .map_err(|e| format!("failed to build audio stream: {e}"))?;

        Ok(Self { stream })
    }

    /// Query the device for a compatible output config.
    fn negotiate_config(
        device: &cpal::Device,
        desired_rate: u32,
    ) -> Result<cpal::StreamConfig, String> {
        match device.supported_output_configs() {
            Ok(mut configs) => {
                // Find a config that supports our sample rate and stereo F32
                configs
                    .find(|c| {
                        c.channels() >= 2
                            && c.min_sample_rate().0 <= desired_rate
                            && c.max_sample_rate().0 >= desired_rate
                            && c.sample_format() == cpal::SampleFormat::F32
                    })
                    .map(|c| c.with_sample_rate(cpal::SampleRate(desired_rate)).config())
                    .or_else(|| {
                        // Fallback: use device default config
                        device.default_output_config().ok().map(|c| c.config())
                    })
                    .ok_or_else(|| {
                        format!(
                            "audio device does not support compatible config \
                             (wanted: stereo F32 @ {}Hz)",
                            desired_rate
                        )
                    })
            }
            Err(_) => {
                // Cannot query supported configs — try device default
                device
                    .default_output_config()
                    .map(|c| c.config())
                    .map_err(|e| format!("failed to get default audio config: {e}"))
            }
        }
    }

    pub fn start(&self) -> Result<(), String> {
        self.stream.play().map_err(|e| e.to_string())
    }

    pub fn pause(&self) -> Result<(), String> {
        self.stream.pause().map_err(|e| e.to_string())
    }
}
