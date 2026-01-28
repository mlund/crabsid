// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! Display state structs for VU meters and oscilloscopes.

use std::time::Instant;

/// Number of samples to display in oscilloscope (downsampled from player buffer)
pub const SCOPE_DISPLAY_SAMPLES: usize = 256;

const ATTACK_RATE: f32 = 0.7;
const DECAY_RATE: f32 = 0.92;
const PEAK_HOLD_MS: u128 = 500;

/// Blend factor for oscilloscope persistence (0.0 = instant, 1.0 = frozen)
const SCOPE_PERSISTENCE: f32 = 0.6;

/// VU meter state with smoothed decay for visual appeal.
/// Supports dynamic voice count (3/6/9 for 1/2/3 SIDs).
pub struct VuMeter {
    pub levels: Vec<f32>,
    pub peaks: Vec<f32>,
    peak_hold: Vec<Instant>,
}

impl VuMeter {
    /// Creates meters for the specified number of voices.
    pub fn with_voice_count(voice_count: usize) -> Self {
        let now = Instant::now();
        Self {
            levels: vec![0.0; voice_count],
            peaks: vec![0.0; voice_count],
            peak_hold: vec![now; voice_count],
        }
    }

    /// Update meters with new envelope values, applying smoothing.
    /// Resizes internal storage if voice count changes.
    pub fn update(&mut self, envelope: &[u8]) {
        self.resize_if_needed(envelope.len());

        let now = Instant::now();
        for (i, &env) in envelope.iter().enumerate() {
            let target = f32::from(env) / 255.0;

            // Fast attack, slow decay for classic VU behavior
            self.levels[i] = if target > self.levels[i] {
                (target - self.levels[i]).mul_add(ATTACK_RATE, self.levels[i])
            } else {
                self.levels[i] * DECAY_RATE
            };

            // Peak hold with decay
            if self.levels[i] >= self.peaks[i] {
                self.peaks[i] = self.levels[i];
                self.peak_hold[i] = now;
            } else if now.duration_since(self.peak_hold[i]).as_millis() > PEAK_HOLD_MS {
                self.peaks[i] *= 0.95;
            }
        }
    }

    fn resize_if_needed(&mut self, voice_count: usize) {
        if self.levels.len() != voice_count {
            let now = Instant::now();
            self.levels.resize(voice_count, 0.0);
            self.peaks.resize(voice_count, 0.0);
            self.peak_hold.resize(voice_count, now);
        }
    }

    /// Returns the number of voices being tracked.
    pub fn voice_count(&self) -> usize {
        self.levels.len()
    }
}

/// Per-voice envelope scope buffers.
/// Supports dynamic voice count (3/6/9 for 1/2/3 SIDs).
pub struct VoiceScopes {
    pub samples: Vec<Vec<f32>>,
}

impl VoiceScopes {
    /// Creates scope buffers for the specified number of voices.
    pub fn with_voice_count(voice_count: usize) -> Self {
        Self {
            samples: (0..voice_count)
                .map(|_| vec![0.0; SCOPE_DISPLAY_SAMPLES])
                .collect(),
        }
    }

    /// Downsample from player envelope buffers to display resolution.
    /// Applies persistence smoothing for easier reading.
    pub fn update(&mut self, raw_samples: &[Vec<f32>]) {
        self.resize_if_needed(raw_samples.len());

        for (display, raw) in self.samples.iter_mut().zip(raw_samples.iter()) {
            if raw.is_empty() {
                continue;
            }
            let step = raw.len() / SCOPE_DISPLAY_SAMPLES;
            if step == 0 {
                continue;
            }
            for (i, sample) in display.iter_mut().enumerate() {
                let new_val = raw.get(i * step).copied().unwrap_or(0.0);
                // Blend old and new for persistence effect
                *sample = sample.mul_add(SCOPE_PERSISTENCE, new_val * (1.0 - SCOPE_PERSISTENCE));
            }
        }
    }

    fn resize_if_needed(&mut self, voice_count: usize) {
        if self.samples.len() != voice_count {
            self.samples
                .resize_with(voice_count, || vec![0.0; SCOPE_DISPLAY_SAMPLES]);
        }
    }

    /// Returns the number of voices being tracked.
    pub fn voice_count(&self) -> usize {
        self.samples.len()
    }
}
