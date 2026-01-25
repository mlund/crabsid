// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

use crate::memory::C64Memory;
use crate::sid_file::SidFile;
use mos6502::cpu::CPU;
use mos6502::instruction::Nmos6502;
use mos6502::memory::Bus;
use mos6502::registers::StackPointer;
use resid::{ChipModel, SamplingMethod};
use std::sync::{Arc, Mutex};

const PAL_CLOCK_HZ: u32 = 985_248;
const NTSC_CLOCK_HZ: u32 = 1_022_727;
const PAL_FRAME_CYCLES: u32 = 19_656;
const NTSC_FRAME_CYCLES: u32 = 17_045;

/// Ring buffer size for oscilloscope display (~23ms at 44.1kHz)
const SCOPE_BUFFER_SIZE: usize = 1024;
/// Envelope sampling divisor (sample envelope every N audio samples)
const ENVELOPE_SAMPLE_DIVISOR: usize = 4;

pub struct Player {
    cpu: CPU<C64Memory, Nmos6502>,
    play_address: u16,
    init_address: u16,
    load_address: u16,
    sid_data: Vec<u8>,
    cycles_per_frame: u32,
    cycles_per_sample: f64,
    cycle_accumulator: f64,
    frame_cycle_count: u32,
    paused: bool,
    /// Per-voice envelope history for channel scopes
    envelope_history: [Box<[f32; SCOPE_BUFFER_SIZE]>; 3],
    envelope_write_pos: usize,
    envelope_sample_counter: usize,
}

impl Player {
    pub fn new(
        sid_file: &SidFile,
        song: u16,
        sample_rate: u32,
        chip_override: Option<u16>,
    ) -> Self {
        let is_pal = sid_file.is_pal();
        let clock_hz = if is_pal { PAL_CLOCK_HZ } else { NTSC_CLOCK_HZ };
        let cycles_per_frame = if is_pal {
            PAL_FRAME_CYCLES
        } else {
            NTSC_FRAME_CYCLES
        };

        let chip_model = match chip_override {
            Some(8580) => ChipModel::Mos8580,
            None if (sid_file.flags >> 4) & 0x03 == 2 => ChipModel::Mos8580,
            Some(_) | None => ChipModel::Mos6581,
        };

        let mut memory = C64Memory::new(chip_model);

        memory
            .sid
            .set_sampling_parameters(SamplingMethod::Fast, clock_hz, sample_rate);

        memory.load(sid_file.load_address, &sid_file.data);

        let mut cpu = CPU::new(memory, Nmos6502);

        // SID tunes expect to be called via JSR and return via RTS.
        // We simulate this by placing RTS at $0000 and pushing $FFFF on stack.
        // When the tune's RTS pops $FFFF, PC wraps to $0000 and hits our RTS,
        // which we detect as the signal that the routine has completed.
        cpu.memory.set_byte(0x0000, 0x60);
        cpu.memory.set_byte(0x01FF, 0xFF);
        cpu.memory.set_byte(0x01FE, 0xFF);
        cpu.registers.stack_pointer = StackPointer(0xFD);
        // SID files use 1-indexed songs, 6502 accumulator is 0-indexed
        #[allow(clippy::cast_possible_truncation)]
        let song_index = song.saturating_sub(1) as u8;
        cpu.registers.accumulator = song_index;
        cpu.registers.program_counter = sid_file.init_address;

        for _ in 0..1_000_000 {
            if cpu.registers.program_counter == 0x0000 {
                break;
            }
            cpu.single_step();
        }

        Self {
            cpu,
            play_address: sid_file.play_address,
            init_address: sid_file.init_address,
            load_address: sid_file.load_address,
            sid_data: sid_file.data.clone(),
            cycles_per_frame,
            cycles_per_sample: f64::from(clock_hz) / f64::from(sample_rate),
            cycle_accumulator: 0.0,
            frame_cycle_count: 0,
            paused: false,
            envelope_history: [
                Box::new([0.0; SCOPE_BUFFER_SIZE]),
                Box::new([0.0; SCOPE_BUFFER_SIZE]),
                Box::new([0.0; SCOPE_BUFFER_SIZE]),
            ],
            envelope_write_pos: 0,
            envelope_sample_counter: 0,
        }
    }

    pub fn fill_buffer(&mut self, buffer: &mut [f32]) {
        if self.paused {
            buffer.fill(0.0);
            return;
        }

        for sample in buffer.iter_mut() {
            self.cycle_accumulator += self.cycles_per_sample;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let cycles_to_run = self.cycle_accumulator as u32;
            self.cycle_accumulator -= f64::from(cycles_to_run);

            for _ in 0..cycles_to_run {
                if self.frame_cycle_count >= self.cycles_per_frame {
                    self.frame_cycle_count = 0;
                    self.call_play();
                }

                self.cpu.memory.sid.clock();
                self.frame_cycle_count += 1;
            }

            *sample = f32::from(self.cpu.memory.sid.output()) / 32768.0;

            // Store envelope history at reduced rate (envelopes change slower than audio)
            self.envelope_sample_counter += 1;
            if self.envelope_sample_counter >= ENVELOPE_SAMPLE_DIVISOR {
                self.envelope_sample_counter = 0;
                let state = self.cpu.memory.sid.read_state();
                for (i, &env) in state.envelope_counter.iter().enumerate() {
                    self.envelope_history[i][self.envelope_write_pos] = f32::from(env) / 255.0;
                }
                self.envelope_write_pos = (self.envelope_write_pos + 1) % SCOPE_BUFFER_SIZE;
            }
        }
    }

    /// Returns envelope history for each voice, ordered oldest to newest
    pub fn envelope_samples(&self) -> [Vec<f32>; 3] {
        std::array::from_fn(|i| {
            let mut samples = Vec::with_capacity(SCOPE_BUFFER_SIZE);
            samples.extend_from_slice(&self.envelope_history[i][self.envelope_write_pos..]);
            samples.extend_from_slice(&self.envelope_history[i][..self.envelope_write_pos]);
            samples
        })
    }

    pub const fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    pub const fn is_paused(&self) -> bool {
        self.paused
    }

    /// Reinitialize for a different song number (1-indexed).
    /// Reloads SID data, resets CPU state, and runs the init routine.
    pub fn load_song(&mut self, song: u16) {
        // Reload the SID data to reset any modified memory
        self.cpu.memory.load(self.load_address, &self.sid_data);

        // Reset all internal SID state (envelope counters, oscillators, filters)
        self.cpu.memory.sid.reset();

        // Set up CPU for init routine
        self.cpu.memory.set_byte(0x0000, 0x60);
        self.cpu.memory.set_byte(0x01FF, 0xFF);
        self.cpu.memory.set_byte(0x01FE, 0xFF);
        self.cpu.registers.stack_pointer = StackPointer(0xFD);
        #[allow(clippy::cast_possible_truncation)]
        let song_index = song.saturating_sub(1) as u8;
        self.cpu.registers.accumulator = song_index;
        self.cpu.registers.program_counter = self.init_address;

        // Run init routine
        for _ in 0..1_000_000 {
            if self.cpu.registers.program_counter == 0x0000 {
                break;
            }
            self.cpu.single_step();
        }

        // Reset playback state
        self.cycle_accumulator = 0.0;
        self.frame_cycle_count = 0;
        self.paused = false;
    }

    /// Returns envelope levels (0-255) for all three SID voices.
    /// Unlike hardware where only ENV3 ($D41C) is readable, emulation
    /// gives us direct access to all voice envelopes via internal state.
    pub fn voice_levels(&self) -> [u8; 3] {
        let state = self.cpu.memory.sid.read_state();
        state.envelope_counter
    }

    fn call_play(&mut self) {
        // play_address == 0 means the tune uses IRQ-driven playback
        if self.play_address == 0 {
            return;
        }

        // Reset stack for each call to handle tunes that don't balance the stack
        self.cpu.memory.set_byte(0x01FF, 0xFF);
        self.cpu.memory.set_byte(0x01FE, 0xFF);
        self.cpu.registers.stack_pointer = StackPointer(0xFD);
        self.cpu.registers.program_counter = self.play_address;

        for _ in 0..100_000 {
            if self.cpu.registers.program_counter == 0x0000 {
                break;
            }
            self.cpu.single_step();
        }
    }
}

pub type SharedPlayer = Arc<Mutex<Player>>;

pub fn create_shared_player(
    sid_file: &SidFile,
    song: u16,
    sample_rate: u32,
    chip_override: Option<u16>,
) -> SharedPlayer {
    Arc::new(Mutex::new(Player::new(
        sid_file,
        song,
        sample_rate,
        chip_override,
    )))
}
