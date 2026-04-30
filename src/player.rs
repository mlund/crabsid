// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

use crate::memory::C64Memory;
use crate::sid_file::SidFile;
use mos6502::cpu::CPU;
use mos6502::instruction::Nmos6502;
use mos6502::memory::Bus;
use mos6502::registers::StackPointer;
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
const STACK_SENTINEL: u8 = 0xFF;
const STACK_POINTER_INIT: u8 = 0xFD;
const RTS_OPCODE: u8 = 0x60;

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
}

/// Errors that can occur while initializing or running SID routines.
#[derive(Debug, PartialEq, Eq)]
pub enum PlayerError {
    /// The init routine never returned before the step limit.
    InitTimeout { steps: u32, address: u16 },
    /// The play routine never returned before the step limit.
    PlayTimeout { steps: u32, address: u16 },
}

impl fmt::Display for PlayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InitTimeout { steps, address } => {
                write!(
                    f,
                    "SID init routine at ${address:04X} exceeded {steps} steps \
                    (may require CIA/interrupt emulation)"
                )
            }
            Self::PlayTimeout { steps, address } => {
                write!(
                    f,
                    "SID play routine at ${address:04X} exceeded {steps} steps"
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
        chip_override: Option<u16>,
        sampling_method: SamplingMethod,
    ) -> PlayerResult<Self> {
        let (clock_hz, cycles_per_frame) = timing_from_file(sid_file);
        let chip_models = select_chip_models(sid_file, chip_override);

        let mut cpu = bootstrap_cpu(
            sid_file,
            &chip_models,
            sample_rate,
            clock_hz,
            song,
            sampling_method,
        );

        run_init(&mut cpu, sid_file.init_address)?;

        let voice_count = chip_models.len() * 3;
        let envelope_history = (0..voice_count)
            .map(|_| Box::new([0.0; SCOPE_BUFFER_SIZE]))
            .collect();

        Ok(Self {
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
        })
    }

    /// Fills the buffer with audio samples, advancing emulation accordingly.
    ///
    /// Each sample triggers the appropriate number of CPU/SID clock cycles
    /// to maintain cycle-accurate timing between the 1MHz system and audio rate.
    /// On error, auto-pauses and stores error message for TUI to display.
    pub fn fill_buffer(&mut self, buffer: &mut [f32]) {
        if self.paused || self.playback_error.is_some() {
            buffer.fill(0.0);
            return;
        }

        let sid_count = self.cpu.memory.sids.len();

        for sample in buffer.iter_mut() {
            self.cycle_accumulator += self.cycles_per_sample;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let cycles_to_run = self.cycle_accumulator as u32;
            self.cycle_accumulator -= f64::from(cycles_to_run);

            for _ in 0..cycles_to_run {
                if self.frame_cycle_count >= self.cycles_per_frame {
                    self.frame_cycle_count = 0;
                    if let Err(e) = self.call_play() {
                        self.playback_error = Some(e.to_string());
                        self.paused = true;
                        buffer.fill(0.0);
                        return;
                    }
                }

                for sid_chip in &mut self.cpu.memory.sids {
                    sid_chip.sid.clock();
                }
                self.frame_cycle_count += 1;
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
                history[self.envelope_write_pos..]
                    .iter()
                    .chain(&history[..self.envelope_write_pos])
                    .copied()
                    .collect()
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

        self.chip_models = select_chip_models(sid_file, None);
        let sid_configs = build_sid_configs(sid_file, &self.chip_models);
        self.cpu.memory.configure_sids(&sid_configs);

        for sid_chip in &mut self.cpu.memory.sids {
            sid_chip
                .sid
                .set_sampling_parameters(self.sampling_method, self.clock_hz, self.sample_rate)
                .unwrap();
        }

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

        run_init(&mut self.cpu, self.init_address)?;

        self.cycle_accumulator = 0.0;
        self.frame_cycle_count = 0;
        self.paused = false;
        self.playback_error = None;
        Ok(())
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

    /// Cycles the chip model for the specified SID (or first if index is None).
    /// Returns the new model for that SID.
    pub fn switch_chip_model(&mut self, sid_index: Option<usize>) -> ChipModel {
        let idx = sid_index.unwrap_or(0);
        let sid_count = self.cpu.memory.sids.len();
        if idx >= self.chip_models.len() || idx >= sid_count {
            return self
                .chip_models
                .first()
                .copied()
                .unwrap_or(ChipModel::Mos6581);
        }

        // Save current register state before replacing the chip
        let state = self.cpu.memory.sids[idx].sid.read_state();

        let new_model = match self.chip_models[idx] {
            ChipModel::Mos6581 => ChipModel::Mos8580,
            ChipModel::Mos8580 => ChipModel::Mos6581,
        };
        self.chip_models[idx] = new_model;

        self.cpu.memory.set_chip_model(idx, new_model);
        self.cpu.memory.sids[idx]
            .sid
            .set_sampling_parameters(self.sampling_method, self.clock_hz, self.sample_rate)
            .unwrap();

        // Restore writable registers (0x00-0x18) to maintain playback
        for (reg, &val) in state.sid_register[..0x19].iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            self.cpu.memory.sids[idx].sid.write(reg as u8, val);
        }

        new_model
    }

    /// Toggles between standard and EKV transistor model filter.
    ///
    /// The EKV filter provides more accurate 6581 emulation using physics-based
    /// MOS transistor modeling. Only affects 6581 chips; 8580 always uses standard.
    ///
    /// Returns `true` if now using EKV filter, `false` if using standard.
    pub fn toggle_ekv_filter(&mut self, sid_index: Option<usize>) -> bool {
        let idx = sid_index.unwrap_or(0);
        if idx >= self.cpu.memory.sids.len() {
            return false;
        }
        self.cpu.memory.sids[idx].sid.toggle_ekv_filter()
    }

    fn call_play(&mut self) -> PlayerResult<()> {
        // play_address == 0 means the tune uses IRQ-driven playback
        if self.play_address == 0 {
            return Ok(());
        }

        // Reset stack each call: some tunes don't balance JSR/RTS over their lifetime.
        self.cpu.memory.set_byte(STACK_HIGH, STACK_SENTINEL);
        self.cpu.memory.set_byte(STACK_LOW, STACK_SENTINEL);
        self.cpu.registers.stack_pointer = StackPointer(STACK_POINTER_INIT);
        self.cpu.registers.program_counter = self.play_address;

        run_play(&mut self.cpu, self.play_address)?;
        Ok(())
    }
}

fn timing_from_file(sid_file: &SidFile) -> (u32, u32) {
    if sid_file.is_pal() {
        (clock::PAL, PAL_FRAME_CYCLES)
    } else {
        (clock::NTSC, NTSC_FRAME_CYCLES)
    }
}

/// Selects chip models for all SIDs in the file.
fn select_chip_models(sid_file: &SidFile, chip_override: Option<u16>) -> Vec<ChipModel> {
    let sid_count = sid_file.sid_count();
    (0..sid_count)
        .map(|i| select_chip_model_for_sid(sid_file, i, chip_override))
        .collect()
}

fn select_chip_model_for_sid(
    sid_file: &SidFile,
    sid_index: usize,
    chip_override: Option<u16>,
) -> ChipModel {
    // CLI override wins; otherwise use the file's per-SID flag (bits 4-5/6-7/8-9, value 2 = 8580).
    match (chip_override, sid_file.chip_model_for_sid(sid_index)) {
        (Some(8580), _) | (None, Some(2)) => ChipModel::Mos8580,
        _ => ChipModel::Mos6581,
    }
}

/// Builds SID configuration pairs (address, model) from file metadata.
fn build_sid_configs(sid_file: &SidFile, chip_models: &[ChipModel]) -> Vec<(u16, ChipModel)> {
    let mut configs = vec![(0xD400, chip_models[0])];

    if let Some(addr) = sid_file.second_sid_address
        && chip_models.len() > 1
    {
        configs.push((addr, chip_models[1]));
    }

    if let Some(addr) = sid_file.third_sid_address
        && chip_models.len() > 2
    {
        configs.push((addr, chip_models[2]));
    }

    configs
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
    cpu.memory.set_byte(0x0000, RTS_OPCODE);
    cpu.memory.set_byte(STACK_HIGH, STACK_SENTINEL);
    cpu.memory.set_byte(STACK_LOW, STACK_SENTINEL);
    cpu.registers.stack_pointer = StackPointer(STACK_POINTER_INIT);
}

fn mix_sample(sum: i32, sid_count: usize) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let mixed = (sum as f32) / (sid_count as f32) / 32768.0;
    // Keep headroom to avoid int16 overflow in platform backends (DirectSound wraps on >1.0)
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

fn run_play(cpu: &mut CPU<C64Memory, Nmos6502>, play_address: u16) -> PlayerResult<()> {
    run_routine(
        cpu,
        100_000,
        PlayerError::PlayTimeout {
            steps: 100_000,
            address: play_address,
        },
    )
}

fn run_routine(
    cpu: &mut CPU<C64Memory, Nmos6502>,
    max_steps: u32,
    timeout_err: PlayerError,
) -> PlayerResult<()> {
    let mut steps = 0;
    while steps < max_steps {
        if cpu.registers.program_counter == 0x0000 {
            return Ok(());
        }
        cpu.single_step();
        steps += 1;
    }
    Err(timeout_err)
}

/// Thread-safe handle for sharing the player between audio and UI threads.
pub type SharedPlayer = Arc<Mutex<Player>>;

/// Creates a player wrapped for thread-safe sharing.
pub fn create_shared_player(
    sid_file: &SidFile,
    song: u16,
    sample_rate: u32,
    chip_override: Option<u16>,
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
            first_sid_mut!(player).write(reg, reg as u8);
        }
        let before = first_sid!(player).read_state();

        player.switch_chip_model(None);
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
}
