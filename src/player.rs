// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

use crate::memory::C64Memory;
use crate::sid_file::SidFile;
use mos6502::cpu::CPU;
use mos6502::instruction::Nmos6502;
use mos6502::memory::{Bus, IRQ_INTERRUPT_VECTOR_HI, IRQ_INTERRUPT_VECTOR_LO};
use mos6502::registers::{StackPointer, Status};
pub use residfp::SamplingMethod;
use residfp::{ChipModel, clock};
use std::sync::{Arc, Mutex};
use std::{error, fmt};
const PAL_FRAME_CYCLES: u32 = 19_656;
const NTSC_FRAME_CYCLES: u32 = 17_045;

/// Ring buffer size for oscilloscope display (~23ms at 44.1kHz)
const SCOPE_BUFFER_SIZE: usize = 1024;
/// Envelope sampling divisor (sample envelope every N audio samples)
const ENVELOPE_SAMPLE_DIVISOR: usize = 4;

/// Sentinel return address pushed onto the stack so JSR'd routines RTS to $0000,
/// where we plant an RTS opcode to halt single-stepping cleanly.
const STACK_HIGH: u16 = 0x01FF;
const STACK_LOW: u16 = 0x01FE;
const STACK_BASE: u16 = 0x0100;
const STACK_SENTINEL: u8 = 0xFF;
const STACK_POINTER_INIT: u8 = 0xFD;
const RTS_OPCODE: u8 = 0x60;
/// Where the CPU's PC parks when no init/play routine is running.
/// Init returns here (we exit `run_init`); PSID play routines RTS here too.
const SENTINEL_PC: u16 = 0x0000;
/// Cycle limit for a single play-routine call. Replaces the prior 100k step limit
/// (~3 cycles per instruction average → 300k cycles is the cycle equivalent).
const MAX_PLAY_CYCLES: u64 = 300_000;
/// Nominal 6502 IRQ entry cost (3 stack pushes + 2 vector reads).
const IRQ_ENTRY_CYCLES: u64 = 7;

/// How the play routine gets driven for the current tune.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DriveMode {
    /// PSID with `speed` bit clear: kick `play()` every PAL/NTSC frame.
    Frame,
    /// PSID with `speed` bit set: poll CIA1 Timer A; tune sets the latch.
    /// The "2x speed" tunes in HVSC pick this path.
    PsidCia,
    /// RSID (or PSID with `play_address == 0`): tune installs its own IRQ vector;
    /// mos6502 services CIA IRQ via the KERNAL stub.
    Interrupt,
}

/// SID music player combining 6502 CPU and SID chip emulation.
///
/// Executes the SID tune's play routine at the correct frame rate while
/// generating audio samples. Supports PAL/NTSC timing, both SID chip models,
/// and multi-SID tunes (2-3 SIDs for 6-9 voices).
pub struct Player {
    /// 6502 CPU with C64 memory map
    cpu: CPU<C64Memory, Nmos6502>,
    /// Address of the play routine called each frame
    play_address: u16,
    /// Address of the init routine for song setup
    init_address: u16,
    /// Memory address where tune data is loaded
    load_address: u16,
    /// Original tune data for reloading on song change
    sid_data: Vec<u8>,
    /// CPU cycles per video frame (PAL: 19656, NTSC: 17045)
    cycles_per_frame: u32,
    /// Fractional cycles to run per audio sample
    cycles_per_sample: f64,
    /// Accumulated fractional cycles between samples
    cycle_accumulator: f64,
    /// Cycles elapsed in current frame
    frame_cycle_count: u32,
    /// Playback paused state
    paused: bool,
    /// Per-voice envelope history for oscilloscope display (3 per SID)
    envelope_history: Vec<Box<[f32; SCOPE_BUFFER_SIZE]>>,
    /// Write position in envelope ring buffers
    envelope_write_pos: usize,
    /// Counter for downsampling envelope captures
    envelope_sample_counter: usize,
    /// Chip models for each SID (1-3 entries)
    chip_models: Vec<ChipModel>,
    /// System clock frequency (PAL or NTSC)
    clock_hz: u32,
    /// Audio output sample rate
    sample_rate: u32,
    /// Last playback error (auto-pauses on error)
    playback_error: Option<String>,
    /// Resampling method for SID audio output
    sampling_method: SamplingMethod,
    /// Cycles since the CPU last left `SENTINEL_PC`; reset when it returns. Catches
    /// runaway play routines and IRQ handlers stuck in a tight loop.
    cpu_busy_cycles: u64,
    /// Recomputed from `speed_flags` and the current song each time the song or
    /// file changes — speed bits are per-song so a "1x" track followed by a "2x"
    /// track in the same file must re-derive.
    drive_mode: DriveMode,
    /// Set once at file-load. Skips the speed-flag check for RSID-style tunes.
    is_interrupt_driven: bool,
    /// Cached `SidFile::speed` so `load_song` can re-derive the drive mode without
    /// holding a `SidFile` reference.
    speed_flags: u32,
    /// `$01` banking byte to apply before running the init routine. Mirrors
    /// libsidplayfp's psiddrv `iomap`: tunes loaded into KERNAL or BASIC ROM
    /// space need those ROMs banked out so the CPU executes the loaded code.
    init_iomap: u8,
    /// `$01` banking byte to apply before each play call. Re-applied on every
    /// kick so the play routine sees the right banking even if init wrote $01.
    play_iomap: u8,
    /// User preference for the EKV transistor-model filter. Persisted on the
    /// player so it survives `load_sid_file`, which rebuilds the SID chips.
    ekv_filter: bool,
}

/// Errors that can occur while initializing or running SID routines.
#[derive(Debug, PartialEq, Eq)]
pub enum PlayerError {
    /// The init routine never returned before the step limit.
    InitTimeout { steps: u32, address: u16 },
    /// Init wrote to hardware we don't emulate (VIC raster IRQ, CIA2 NMI).
    /// The tune may technically be playable but our minimal emulator can't drive it.
    UnsupportedHardware { addresses: Vec<u16> },
}

impl fmt::Display for PlayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InitTimeout { steps, address } => {
                write!(
                    f,
                    "SID init routine at ${address:04X} exceeded {steps} steps"
                )
            }
            Self::UnsupportedHardware { addresses } => {
                let list = addresses
                    .iter()
                    .map(|a| format!("${a:04X}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(
                    f,
                    "Tune writes to unsupported hardware ({list}) — likely needs \
                    VIC raster IRQ or CIA2 NMI emulation"
                )
            }
        }
    }
}

impl error::Error for PlayerError {}

type PlayerResult<T> = Result<T, PlayerError>;

impl Player {
    /// Creates a player for the given SID file and song number (1-indexed).
    ///
    /// Loads the tune into emulated memory, runs the init routine, and
    /// configures timing based on PAL/NTSC detection from the file header.
    ///
    /// The `sampling_method` parameter controls audio quality vs CPU usage:
    /// - `Fast`: Direct output (lowest quality, lowest CPU)
    /// - `Interpolate`: Linear interpolation (good quality, low CPU)
    /// - `ResampleFast`: FIR resampling without interpolation
    /// - `Resample`: FIR resampling with interpolation (highest quality)
    /// - `ResampleTwoPass`: Two-stage FIR resampling (high quality, efficient)
    pub fn new(
        sid_file: &SidFile,
        song: u16,
        sample_rate: u32,
        chip_override: Option<ChipModel>,
        sampling_method: SamplingMethod,
    ) -> PlayerResult<Self> {
        let (clock_hz, cycles_per_frame) = timing_from_file(sid_file);
        let chip_models = sid_file.preferred_chip_models(chip_override);

        let mut cpu = bootstrap_cpu(
            sid_file,
            &chip_models,
            sample_rate,
            clock_hz,
            song,
            sampling_method,
        );

        // Apply PSID-driver banking before init so tunes whose code lives in
        // BASIC or KERNAL ROM space (e.g. Cobra at $F100) execute from RAM.
        cpu.memory.set_bank_config(psid_iomap(sid_file.init_address));
        run_init(&mut cpu, sid_file.init_address)?;
        check_supported_hardware(&cpu)?;

        let voice_count = chip_models.len() * 3;
        let envelope_history = (0..voice_count)
            .map(|_| Box::new([0.0; SCOPE_BUFFER_SIZE]))
            .collect();

        let mut player = Self {
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
            envelope_history,
            envelope_write_pos: 0,
            envelope_sample_counter: 0,
            chip_models,
            clock_hz,
            sample_rate,
            playback_error: None,
            sampling_method,
            cpu_busy_cycles: 0,
            drive_mode: DriveMode::Frame,
            is_interrupt_driven: sid_file.is_interrupt_driven(),
            speed_flags: sid_file.speed,
            init_iomap: psid_iomap(sid_file.init_address),
            play_iomap: psid_iomap(sid_file.play_address),
            ekv_filter: false,
        };
        player.apply_drive_mode(song);
        Ok(player)
    }

    /// Fills the buffer with audio samples, advancing emulation accordingly.
    ///
    /// Cycle-accurate: every CPU instruction is single-stepped, and CIA/SID/audio
    /// timers all advance by the same `elapsed` cycle delta from `cpu.cycles`.
    /// On error, auto-pauses and stores error message for TUI to display.
    pub fn fill_buffer(&mut self, buffer: &mut [f32]) {
        if self.paused || self.playback_error.is_some() {
            buffer.fill(0.0);
            return;
        }

        let sid_count = self.cpu.memory.sids.len();

        for sample in buffer.iter_mut() {
            self.cycle_accumulator += self.cycles_per_sample;
            while self.cycle_accumulator >= 1.0 {
                if !self.advance_one_step() {
                    buffer.fill(0.0);
                    return;
                }
            }

            let sum: i32 = self
                .cpu
                .memory
                .sids
                .iter()
                .map(|s| i32::from(s.sid.output()))
                .sum();
            *sample = mix_sample(sum, sid_count);

            self.capture_envelope_history();
        }
    }

    /// Drives the emulator forward by one CPU instruction (or one idle cycle when no
    /// routine is running) and ticks all per-cycle peripherals by the same delta.
    /// Returns `false` to halt the buffer (runaway routine, etc.).
    fn advance_one_step(&mut self) -> bool {
        let at_sentinel = self.cpu.registers.program_counter == SENTINEL_PC;

        // Only RSID-style tunes route CIA IRQs through mos6502's vector. PSID
        // (frame or CIA-poll) keeps the CPU at the sentinel and the player
        // dispatches play() directly.
        let irq_at_sentinel = at_sentinel
            && self.drive_mode == DriveMode::Interrupt
            && self.cpu.memory.irq_pending()
            && !self
                .cpu
                .registers
                .status
                .contains(Status::PS_DISABLE_INTERRUPTS);

        let elapsed = if irq_at_sentinel {
            // Manually vector — single-stepping the RTS at $0000 would pop garbage.
            self.force_irq_service();
            IRQ_ENTRY_CYCLES
        } else if at_sentinel {
            1
        } else {
            let prev = self.cpu.cycles;
            self.cpu.single_step();
            self.cpu.cycles - prev
        };

        // Frame-mode tunes never observe CIA1, so skip the timer arithmetic entirely.
        if self.drive_mode != DriveMode::Frame {
            self.cpu.memory.tick(elapsed);
        }
        #[allow(clippy::cast_possible_truncation)]
        let elapsed_u32 = elapsed as u32;
        for sid_chip in &mut self.cpu.memory.sids {
            sid_chip.sid.clock_delta(elapsed_u32);
        }

        self.cycle_accumulator -= elapsed as f64;
        self.frame_cycle_count = self.frame_cycle_count.saturating_add(elapsed_u32);

        // PC away from the sentinel = a play routine or IRQ handler is running;
        // a too-long stretch trips the runaway-routine timeout.
        if self.cpu.registers.program_counter != SENTINEL_PC {
            self.cpu_busy_cycles = self.cpu_busy_cycles.saturating_add(elapsed);
            if self.cpu_busy_cycles > MAX_PLAY_CYCLES {
                self.playback_error = Some(format!(
                    "CPU stuck for {MAX_PLAY_CYCLES} cycles at PC=${:04X}",
                    self.cpu.registers.program_counter
                ));
                self.paused = true;
                return false;
            }
            return true;
        }
        self.cpu_busy_cycles = 0;

        if self.should_kick_play() {
            reset_stack_pointer(&mut self.cpu);
            // Re-apply banking on every kick: init may have left $01 elsewhere,
            // and libsidplayfp's psiddrv refreshes it before each play call.
            self.cpu.memory.set_bank_config(self.play_iomap);
            self.cpu.registers.program_counter = self.play_address;
        }

        true
    }

    /// True when the CPU is idle at the sentinel and the active drive-mode says
    /// it's time to invoke `play()`. RSID/Interrupt mode never kicks here — it
    /// services the IRQ via the CPU vector path instead.
    fn should_kick_play(&mut self) -> bool {
        if self.play_address == 0 {
            return false;
        }
        match self.drive_mode {
            DriveMode::Frame => {
                if self.frame_cycle_count >= self.cycles_per_frame {
                    self.frame_cycle_count -= self.cycles_per_frame;
                    true
                } else {
                    false
                }
            }
            DriveMode::PsidCia => self.cpu.memory.cia1_take_timer_a_underflow(),
            DriveMode::Interrupt => false,
        }
    }

    /// Mimics mos6502's `service_interrupt` without executing whatever opcode lives
    /// at the current PC. Used to wake the CPU from the sentinel when CIA1 fires.
    fn force_irq_service(&mut self) {
        let [pc_lo, pc_hi] = self.cpu.registers.program_counter.to_le_bytes();
        self.push_byte(pc_hi);
        self.push_byte(pc_lo);
        let status = self.cpu.registers.status & !Status::PS_BRK;
        self.push_byte(status.bits());
        self.cpu
            .registers
            .status
            .insert(Status::PS_DISABLE_INTERRUPTS);
        let lo = self.cpu.memory.get_byte(IRQ_INTERRUPT_VECTOR_LO);
        let hi = self.cpu.memory.get_byte(IRQ_INTERRUPT_VECTOR_HI);
        self.cpu.registers.program_counter = u16::from_le_bytes([lo, hi]);
    }

    fn push_byte(&mut self, val: u8) {
        let sp = self.cpu.registers.stack_pointer.0;
        self.cpu.memory.set_byte(STACK_BASE | u16::from(sp), val);
        self.cpu.registers.stack_pointer = StackPointer(sp.wrapping_sub(1));
    }

    /// Captures envelope history at reduced rate for oscilloscope display.
    fn capture_envelope_history(&mut self) {
        self.envelope_sample_counter += 1;
        if self.envelope_sample_counter < ENVELOPE_SAMPLE_DIVISOR {
            return;
        }
        self.envelope_sample_counter = 0;

        let mut voice_idx = 0;
        for sid_chip in &self.cpu.memory.sids {
            let state = sid_chip.sid.read_state();
            for &env in &state.envelope_counter {
                if voice_idx < self.envelope_history.len() {
                    self.envelope_history[voice_idx][self.envelope_write_pos] =
                        f32::from(env) / 255.0;
                }
                voice_idx += 1;
            }
        }
        self.envelope_write_pos = (self.envelope_write_pos + 1) % SCOPE_BUFFER_SIZE;
    }

    /// Returns envelope history for each voice, ordered oldest to newest.
    /// Returns 3 entries per SID (3/6/9 voices for 1/2/3 SIDs).
    pub fn envelope_samples(&self) -> Vec<Vec<f32>> {
        let voice_count = self.envelope_history.len();
        if self.paused {
            return vec![vec![0.0; SCOPE_BUFFER_SIZE]; voice_count];
        }
        self.envelope_history
            .iter()
            .map(|history| {
                // Older samples sit after the write head; chain them in front of the newer prefix.
                let (head, tail) = history.split_at(self.envelope_write_pos);
                tail.iter().chain(head).copied().collect()
            })
            .collect()
    }

    /// Toggles between playing and paused states.
    pub const fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    /// Returns whether playback is currently paused.
    pub const fn is_paused(&self) -> bool {
        self.paused
    }

    /// Takes and clears any pending playback error.
    pub fn take_error(&mut self) -> Option<String> {
        self.playback_error.take()
    }

    /// Loads a completely new SID file, replacing the current tune.
    pub fn load_sid_file(&mut self, sid_file: &SidFile, song: u16) -> PlayerResult<()> {
        (self.clock_hz, self.cycles_per_frame) = timing_from_file(sid_file);
        self.cycles_per_sample = f64::from(self.clock_hz) / f64::from(self.sample_rate);

        self.play_address = sid_file.play_address;
        self.init_address = sid_file.init_address;
        self.load_address = sid_file.load_address;
        self.sid_data = sid_file.data.clone();
        self.is_interrupt_driven = sid_file.is_interrupt_driven();
        self.speed_flags = sid_file.speed;
        self.init_iomap = psid_iomap(sid_file.init_address);
        self.play_iomap = psid_iomap(sid_file.play_address);

        self.chip_models = sid_file.preferred_chip_models(None);
        let sid_configs = build_sid_configs(sid_file, &self.chip_models);
        self.cpu.memory.configure_sids(&sid_configs);

        for sid_chip in &mut self.cpu.memory.sids {
            sid_chip
                .sid
                .set_sampling_parameters(self.sampling_method, self.clock_hz, self.sample_rate)
                .unwrap();
        }
        // configure_sids rebuilds chips with the default (standard) filter — re-apply preference.
        self.sync_ekv_filter();

        let voice_count = self.chip_models.len() * 3;
        self.envelope_history = (0..voice_count)
            .map(|_| Box::new([0.0; SCOPE_BUFFER_SIZE]))
            .collect();

        self.load_song(song)?;
        Ok(())
    }

    /// Reinitialize for a different song number (1-indexed).
    /// Reloads SID data, resets CPU state, and runs the init routine.
    pub fn load_song(&mut self, song: u16) -> PlayerResult<()> {
        self.cpu.memory.clear_zeropage_and_stack();
        self.cpu.memory.load(self.load_address, &self.sid_data);

        for sid_chip in &mut self.cpu.memory.sids {
            sid_chip.sid.reset();
        }

        self.cpu.registers.index_x = 0;
        self.cpu.registers.index_y = 0;
        self.cpu.registers.status = mos6502::registers::Status::empty();

        setup_stack_for_rts(&mut self.cpu);
        #[allow(clippy::cast_possible_truncation)]
        let song_index = song.saturating_sub(1) as u8;
        self.cpu.registers.accumulator = song_index;
        self.cpu.registers.program_counter = self.init_address;

        // Apply PSID-driver banking before init so tunes loaded into KERNAL or
        // BASIC ROM space (e.g. Cobra at $F100) execute from RAM, not the stub.
        self.cpu.memory.set_bank_config(self.init_iomap);
        run_init(&mut self.cpu, self.init_address)?;
        check_supported_hardware(&self.cpu)?;

        // Drive mode is per-song (PSID `speed` is a 32-bit-per-song bitmap), so
        // re-derive even for an in-file song change.
        self.apply_drive_mode(song);

        self.cycle_accumulator = 0.0;
        self.frame_cycle_count = 0;
        self.cpu_busy_cycles = 0;
        self.paused = false;
        self.playback_error = None;
        Ok(())
    }

    /// Picks the play-dispatch mode for `song` and pushes the bus IRQ-routing flag
    /// in lockstep so PSID modes never see mos6502 service a CIA IRQ.
    fn apply_drive_mode(&mut self, song: u16) {
        self.drive_mode = if self.is_interrupt_driven {
            DriveMode::Interrupt
        } else if cia_timing_for_song(self.speed_flags, song) {
            DriveMode::PsidCia
        } else {
            DriveMode::Frame
        };
        self.cpu
            .memory
            .set_cia_irq_routed(self.drive_mode == DriveMode::Interrupt);
    }

    /// Returns envelope levels (0-255) for all SID voices.
    /// Returns 3 entries per SID (3/6/9 voices for 1/2/3 SIDs).
    /// Unlike hardware where only ENV3 ($D41C) is readable, emulation
    /// gives us direct access to all voice envelopes via internal state.
    pub fn voice_levels(&self) -> Vec<u8> {
        let voice_count = self.cpu.memory.sids.len() * 3;
        if self.paused {
            return vec![0; voice_count];
        }
        self.cpu
            .memory
            .sids
            .iter()
            .flat_map(|s| s.sid.read_state().envelope_counter)
            .collect()
    }

    /// Returns the chip models for all SIDs.
    pub fn chip_models(&self) -> &[ChipModel] {
        &self.chip_models
    }

    /// Returns the number of SID chips.
    pub fn sid_count(&self) -> usize {
        self.chip_models.len()
    }

    /// Cycles the chip model for the specified SID (0-indexed).
    /// Returns the new model, or `None` if `sid_index` is out of range.
    pub fn switch_chip_model(&mut self, sid_index: usize) -> Option<ChipModel> {
        if sid_index >= self.chip_models.len() {
            return None;
        }

        // Save current register state before replacing the chip.
        let state = self.cpu.memory.sids[sid_index].sid.read_state();

        let new_model = match self.chip_models[sid_index] {
            ChipModel::Mos6581 => ChipModel::Mos8580,
            ChipModel::Mos8580 => ChipModel::Mos6581,
        };
        self.chip_models[sid_index] = new_model;

        self.cpu.memory.set_chip_model(sid_index, new_model);
        self.cpu.memory.sids[sid_index]
            .sid
            .set_sampling_parameters(self.sampling_method, self.clock_hz, self.sample_rate)
            .unwrap();

        // Restore writable registers (0x00-0x18) to maintain playback.
        for (reg, &val) in state.sid_register[..0x19].iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            self.cpu.memory.sids[sid_index].sid.write(reg as u8, val);
        }

        Some(new_model)
    }

    /// Sets the EKV transistor-model filter preference for all SIDs.
    ///
    /// The preference is stored on the player and re-applied each time a new
    /// SID file is loaded (which rebuilds the chips and would otherwise reset
    /// the filter to the standard spline model). Only affects 6581 chips;
    /// 8580 always uses the standard filter.
    pub fn set_ekv_filter(&mut self, enable: bool) {
        self.ekv_filter = enable;
        self.sync_ekv_filter();
    }

    /// Forces every SID chip's EKV state to match `self.ekv_filter`.
    /// The underlying library only exposes a toggle, so we read the current
    /// state per chip and flip if it disagrees with the preference.
    fn sync_ekv_filter(&mut self) {
        for chip in &mut self.cpu.memory.sids {
            if chip.sid.is_ekv_filter_enabled() != self.ekv_filter {
                chip.sid.toggle_ekv_filter();
            }
        }
    }
}

/// PSID-driver `$01` value to apply for an entry-point address. Mirrors
/// libsidplayfp's `iomap`: tunes whose code lives in BASIC or KERNAL ROM space
/// need those ROMs banked out so the CPU executes the loaded RAM instead.
/// I/O at `$D000-$DFFF` stays mapped except when the tune itself sits there.
const fn psid_iomap(addr: u16) -> u8 {
    match addr {
        0x0000..=0x9FFF => 0x37, // C64 default: BASIC + KERNAL + I/O all in
        0xA000..=0xBFFF => 0x36, // BASIC out (tune lives in BASIC ROM space)
        0xC000..=0xCFFF => 0x35, // BASIC + KERNAL out
        0xD000..=0xDFFF => 0x34, // BASIC + KERNAL + I/O out (tune in I/O area)
        0xE000..=0xFFFF => 0x35, // BASIC + KERNAL out, I/O in
    }
}

/// PSID `speed` field is a per-song bitmap (bit n = song n+1, 1 = CIA timer).
fn cia_timing_for_song(speed_flags: u32, song: u16) -> bool {
    if song == 0 || song > 32 {
        return false;
    }
    (speed_flags >> (song - 1)) & 1 != 0
}

fn timing_from_file(sid_file: &SidFile) -> (u32, u32) {
    if sid_file.is_pal() {
        (clock::PAL, PAL_FRAME_CYCLES)
    } else {
        (clock::NTSC, NTSC_FRAME_CYCLES)
    }
}

fn build_sid_configs(sid_file: &SidFile, chip_models: &[ChipModel]) -> Vec<(u16, ChipModel)> {
    sid_file
        .sid_addresses()
        .into_iter()
        .zip(chip_models.iter().copied())
        .collect()
}

fn bootstrap_cpu(
    sid_file: &SidFile,
    chip_models: &[ChipModel],
    sample_rate: u32,
    clock_hz: u32,
    song: u16,
    sampling_method: SamplingMethod,
) -> CPU<C64Memory, Nmos6502> {
    let mut memory = C64Memory::new(chip_models[0]);

    let sid_configs = build_sid_configs(sid_file, chip_models);
    memory.configure_sids(&sid_configs);

    for sid_chip in &mut memory.sids {
        sid_chip
            .sid
            .set_sampling_parameters(sampling_method, clock_hz, sample_rate)
            .unwrap();
    }

    memory.load(sid_file.load_address, &sid_file.data);

    let mut cpu = CPU::new(memory, Nmos6502);
    setup_stack_for_rts(&mut cpu);

    #[allow(clippy::cast_possible_truncation)]
    let song_index = song.saturating_sub(1) as u8;
    cpu.registers.accumulator = song_index;
    cpu.registers.program_counter = sid_file.init_address;
    cpu
}

fn setup_stack_for_rts(cpu: &mut CPU<C64Memory, Nmos6502>) {
    // Tunes expect JSR/RTS pairing; plant an RTS at $0000 and a $FFFF return on the stack.
    cpu.memory.set_byte(SENTINEL_PC, RTS_OPCODE);
    reset_stack_pointer(cpu);
}

fn reset_stack_pointer(cpu: &mut CPU<C64Memory, Nmos6502>) {
    cpu.memory.set_byte(STACK_HIGH, STACK_SENTINEL);
    cpu.memory.set_byte(STACK_LOW, STACK_SENTINEL);
    cpu.registers.stack_pointer = StackPointer(STACK_POINTER_INIT);
}

fn mix_sample(sum: i32, sid_count: usize) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let mixed = (sum as f32) / (sid_count as f32) / 32768.0;
    // Keep a small headroom: backends running at i16/u16 will wrap if a sample
    // converts to exactly i16::MAX, and downstream limiters dislike a hot 1.0.
    mixed.clamp(-0.999_5, 0.999_5)
}

fn run_init(cpu: &mut CPU<C64Memory, Nmos6502>, init_address: u16) -> PlayerResult<()> {
    run_routine(
        cpu,
        1_000_000,
        PlayerError::InitTimeout {
            steps: 1_000_000,
            address: init_address,
        },
    )
}

/// Runs the CPU synchronously until PC reaches `SENTINEL_PC` or the step limit is hit.
/// Used only for the init routine; in-flight `fill_buffer` uses cycle-accurate stepping.
fn run_routine(
    cpu: &mut CPU<C64Memory, Nmos6502>,
    max_steps: u32,
    timeout_err: PlayerError,
) -> PlayerResult<()> {
    let mut steps = 0;
    while steps < max_steps {
        if cpu.registers.program_counter == SENTINEL_PC {
            return Ok(());
        }
        cpu.single_step();
        steps += 1;
    }
    Err(timeout_err)
}

/// Refuses tunes whose init touched VIC raster IRQ or CIA2 NMI registers.
fn check_supported_hardware(cpu: &CPU<C64Memory, Nmos6502>) -> PlayerResult<()> {
    let traps = cpu.memory.unsupported_hardware();
    if traps.is_empty() {
        Ok(())
    } else {
        Err(PlayerError::UnsupportedHardware {
            addresses: traps.to_vec(),
        })
    }
}

/// Thread-safe handle for sharing the player between audio and UI threads.
pub type SharedPlayer = Arc<Mutex<Player>>;

/// Creates a player wrapped for thread-safe sharing.
pub fn create_shared_player(
    sid_file: &SidFile,
    song: u16,
    sample_rate: u32,
    chip_override: Option<ChipModel>,
    sampling_method: SamplingMethod,
) -> PlayerResult<SharedPlayer> {
    Player::new(sid_file, song, sample_rate, chip_override, sampling_method)
        .map(|p| Arc::new(Mutex::new(p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! fill_history {
        ($player:expr, $voice:expr, $offset:expr) => {
            for i in 0..SCOPE_BUFFER_SIZE {
                $player.envelope_history[$voice][i] = i as f32 + $offset;
            }
        };
    }

    macro_rules! assert_sid_registers_eq {
        ($a:expr, $b:expr, $range:expr) => {
            for reg in $range {
                assert_eq!(
                    $a.sid_register[reg], $b.sid_register[reg],
                    "register {reg:02X} mismatch"
                );
            }
        };
    }

    macro_rules! first_sid {
        ($player:expr) => {
            &$player.cpu.memory.sids[0].sid
        };
    }

    macro_rules! first_sid_mut {
        ($player:expr) => {
            &mut $player.cpu.memory.sids[0].sid
        };
    }

    macro_rules! test_sid {
        () => {
            SidFile {
                magic: "PSID".to_string(),
                version: 2,
                data_offset: 0x7c,
                load_address: 0x1000,
                init_address: 0x1000,
                play_address: 0x1003,
                songs: 1,
                start_song: 1,
                speed: 0,
                name: String::new(),
                author: String::new(),
                released: String::new(),
                flags: 0,
                data: vec![0x60, 0x60, 0x60],
                md5: String::new(),
                second_sid_address: None,
                third_sid_address: None,
            }
        };
    }

    fn load_fixture(name: &str) -> SidFile {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name);
        SidFile::load(path).expect("load fixture sid")
    }

    #[test]
    fn envelope_samples_rotate_oldest_first() {
        let sid = test_sid!();
        let mut player =
            Player::new(&sid, 1, 44_100, None, SamplingMethod::Fast).expect("player init");

        fill_history!(player, 0, 0.0);
        fill_history!(player, 1, 1000.0);
        fill_history!(player, 2, 2000.0);
        player.envelope_write_pos = 3;

        let samples = player.envelope_samples();
        assert_eq!(samples[0][0], 3.0);
        assert_eq!(samples[0][1], 4.0);
        assert_eq!(samples[0].last().copied().unwrap(), 2.0);
        assert_eq!(samples[1][0], 1003.0);
        assert_eq!(samples[2][0], 2003.0);
    }

    #[test]
    fn switch_chip_preserves_sid_registers() {
        let sid = test_sid!();
        let mut player =
            Player::new(&sid, 1, 44_100, None, SamplingMethod::Fast).expect("player init");

        for reg in 0..=0x18 {
            first_sid_mut!(player).write(reg, reg);
        }
        let before = first_sid!(player).read_state();

        player.switch_chip_model(0);
        let after = first_sid!(player).read_state();

        assert_sid_registers_eq!(before, after, 0..=0x18);
    }

    #[test]
    fn mix_sample_limits_output() {
        assert_eq!(mix_sample(0, 1), 0.0);
        assert!(mix_sample(i32::MAX, 1) <= 1.0);
        assert!(mix_sample(i32::MIN, 1) >= -1.0);
        let clipped = mix_sample(40_000, 1);
        assert!(clipped < 0.999_6);
    }

    #[test]
    fn glitch_fixture_stays_within_i16_range() {
        let sid = load_fixture("Glitch.sid");
        let mut player = Player::new(&sid, sid.start_song, 44_100, None, SamplingMethod::Fast)
            .expect("player init");

        let mut buffer = vec![0.0f32; 1024];
        let mut max_abs = 0.0f32;
        let mut max_i16 = i16::MIN;
        let mut min_i16 = i16::MAX;

        for _ in 0..64 {
            player.fill_buffer(&mut buffer);
            for &s in &buffer {
                let scaled = (s * i16::MAX as f32) as i16;
                max_i16 = max_i16.max(scaled);
                min_i16 = min_i16.min(scaled);
                max_abs = max_abs.max(s.abs());
            }
        }

        assert!(max_abs <= 0.9996, "mix exceeded headroom: {max_abs}");
        assert!(max_i16 < i16::MAX, "scaled samples hit i16::MAX");
        assert!(min_i16 > i16::MIN, "scaled samples hit i16::MIN");
    }

    /// PSID with the speed bit set: player polls CIA1 Timer A underflow and
    /// drives `play()` itself. Verifies the "2x speed" tune family no longer
    /// runs at 50Hz when the tune asked for a faster rate.
    #[test]
    fn psid_cia_fixture_play_is_polled_at_cia_rate() {
        let sid = SidFile::psid_cia_test_fixture();
        assert!(sid.uses_cia_timing(1));
        let mut player =
            Player::new(&sid, 1, 44_100, None, SamplingMethod::Fast).expect("PSID-CIA player init");

        const COUNTER_ADDR: u16 = 0x0400;
        assert_eq!(player.cpu.memory.ram_byte(COUNTER_ADDR), 0);

        let mut buffer = vec![0.0f32; 1024];
        player.fill_buffer(&mut buffer);

        assert!(
            player.cpu.memory.ram_byte(COUNTER_ADDR) > 0,
            "play() was not called by the CIA-poll path"
        );
    }

    #[test]
    fn psid_iomap_banks_out_kernal_for_high_init() {
        assert_eq!(psid_iomap(0x1000), 0x37, "low memory: defaults stay");
        assert_eq!(psid_iomap(0xA000), 0x36, "BASIC ROM area: BASIC out");
        assert_eq!(psid_iomap(0xC000), 0x35, "between ROMs: both ROMs out");
        assert_eq!(psid_iomap(0xD400), 0x34, "I/O area: I/O out");
        assert_eq!(psid_iomap(0xF900), 0x35, "KERNAL ROM area: KERNAL out, I/O in");
    }

    /// Regression for tunes loaded into the KERNAL ROM area (e.g. Daglish's
    /// Cobra at $F100). The KERNAL stub returns RTS for any address it covers,
    /// so without `iomap` banking the init routine would return immediately
    /// without writing any SID register.
    #[test]
    fn psid_kernal_area_fixture_runs_init_from_ram() {
        let sid = SidFile::psid_kernal_area_fixture();
        let player =
            Player::new(&sid, 1, 44_100, None, SamplingMethod::Fast).expect("player init");

        // Init wrote $0F to $D418; if KERNAL was still banked in, init would
        // have hit the stub's RTS at $F000 and the SID would still read 0.
        let state = first_sid!(player).read_state();
        assert_eq!(state.sid_register[0x18], 0x0F, "init didn't reach SID volume");
    }

    /// CIA Timer A drives the IRQ; the handler increments `$0400`. After a single
    /// audio buffer fill, the counter must be non-zero — proves both that init's
    /// IRQ-vector install survived and that mos6502 is fetching `$FFFE/$FFFF`
    /// through the KERNAL stub.
    #[test]
    fn rsid_fixture_handler_runs_via_cia_irq() {
        let sid = SidFile::rsid_test_fixture();
        assert!(sid.is_interrupt_driven(), "fixture must be RSID");

        let mut player =
            Player::new(&sid, 1, 44_100, None, SamplingMethod::Fast).expect("RSID player init");

        const COUNTER_ADDR: u16 = 0x0400;
        assert_eq!(player.cpu.memory.ram_byte(COUNTER_ADDR), 0);

        // ~22000 CPU cycles per fill, latch=1024, so the handler should fire ~20×.
        let mut buffer = vec![0.0f32; 1024];
        player.fill_buffer(&mut buffer);

        assert!(
            player.cpu.memory.ram_byte(COUNTER_ADDR) > 0,
            "CIA1 IRQ handler did not run during fill_buffer"
        );
    }
}
