// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! Display state structs for VU meters and oscilloscopes.

use std::time::Instant;

/// Number of samples to display in oscilloscope (downsampled from player buffer)
pub const SCOPE_DISPLAY_SAMPLES: usize = 256;

/// VU meter state with smoothed decay for visual appeal.
pub struct VuMeter {
    pub levels: [f32; 3],
    pub peaks: [f32; 3],
    peak_hold: [Instant; 3],
}

impl VuMeter {
    /// Creates meters with all levels at zero.
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            levels: [0.0; 3],
            peaks: [0.0; 3],
            peak_hold: [now; 3],
        }
    }

    /// Update meters with new envelope values, applying smoothing.
    pub fn update(&mut self, envelope: [u8; 3]) {
        const ATTACK_RATE: f32 = 0.7;
        const DECAY_RATE: f32 = 0.92;
        const PEAK_HOLD_MS: u128 = 500;

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
}

/// Per-voice envelope scope buffers.
pub struct VoiceScopes {
    pub samples: [Vec<f32>; 3],
}

impl VoiceScopes {
    /// Creates scope buffers initialized to zero.
    pub fn new() -> Self {
        Self {
            samples: std::array::from_fn(|_| vec![0.0; SCOPE_DISPLAY_SAMPLES]),
        }
    }

    /// Downsample from player envelope buffers to display resolution.
    pub fn update(&mut self, raw_samples: &[Vec<f32>; 3]) {
        for (display, raw) in self.samples.iter_mut().zip(raw_samples.iter()) {
            if raw.is_empty() {
                continue;
            }
            let step = raw.len() / SCOPE_DISPLAY_SAMPLES;
            if step == 0 {
                continue;
            }
            for (i, sample) in display.iter_mut().enumerate() {
                *sample = raw.get(i * step).copied().unwrap_or(0.0);
            }
        }
    }
}
