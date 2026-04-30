// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! Audio output via cpal. The default device's preferred sample rate and
//! channel layout are used so the player's resampler matches the device exactly.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SampleRate, SizedSample, Stream, StreamConfig};
use std::error::Error;

/// Configuration probed from the default output device, used to construct
/// the player at the right sample rate before the audio stream is started.
pub struct AudioOutput {
    device: cpal::Device,
    config: StreamConfig,
    sample_format: SampleFormat,
    /// Device-native sample rate; pass to `Player::new`.
    pub sample_rate: u32,
}

impl AudioOutput {
    /// Probes the system default output device.
    pub fn probe() -> Result<Self, Box<dyn Error>> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no default audio output device")?;
        let supported = device.default_output_config()?;
        let sample_rate: SampleRate = supported.sample_rate();
        let config = StreamConfig {
            channels: supported.channels(),
            sample_rate,
            // Default lets the platform pick a buffer size it actually supports
            // (CoreAudio in particular dislikes Fixed sizes).
            buffer_size: cpal::BufferSize::Default,
        };
        Ok(Self {
            device,
            config,
            sample_format: supported.sample_format(),
            sample_rate,
        })
    }

    /// Starts the audio stream. The callback fills a mono `&mut [f32]` buffer;
    /// samples are duplicated across channels for the device.
    pub fn start<F>(self, fill: F) -> Result<Stream, Box<dyn Error>>
    where
        F: FnMut(&mut [f32]) + Send + 'static,
    {
        let stream = match self.sample_format {
            SampleFormat::F32 => build_stream::<f32, _>(&self.device, &self.config, fill)?,
            SampleFormat::I16 => build_stream::<i16, _>(&self.device, &self.config, fill)?,
            SampleFormat::U16 => build_stream::<u16, _>(&self.device, &self.config, fill)?,
            other => return Err(format!("unsupported audio sample format: {other:?}").into()),
        };
        stream.play()?;
        Ok(stream)
    }
}

fn build_stream<T, F>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut fill: F,
) -> Result<Stream, cpal::BuildStreamError>
where
    T: SizedSample + FromSample<f32>,
    F: FnMut(&mut [f32]) + Send + 'static,
{
    let channels = config.channels as usize;
    let mut mono = Vec::<f32>::new();
    device.build_output_stream(
        config,
        move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
            let frames = output.len() / channels;
            mono.resize(frames, 0.0);
            fill(&mut mono);
            for (frame_idx, frame) in output.chunks_mut(channels).enumerate() {
                let s: T = T::from_sample(mono[frame_idx]);
                for sample in frame {
                    *sample = s;
                }
            }
        },
        |err| eprintln!("audio stream error: {err}"),
        None,
    )
}
