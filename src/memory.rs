// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

use mos6502::memory::Bus;
use resid::{ChipModel, Sid};

const RAM_SIZE: usize = 65536;
const SID_BASE: u16 = 0xD400;
const SID_END: u16 = 0xD41C;

/// Emulated C64 memory map with SID chip at $D400-$D41C.
///
/// Provides 64KB RAM with memory-mapped I/O for the SID sound chip.
/// All other I/O areas (VIC, CIA, etc.) are treated as plain RAM since
/// SID playback only requires the sound chip.
pub struct C64Memory {
    /// 64KB RAM, heap-allocated to avoid stack overflow
    ram: Box<[u8]>,
    /// SID sound chip mapped at $D400
    pub sid: Sid,
}

impl C64Memory {
    /// Creates memory with zeroed RAM and a SID chip of the specified model.
    pub fn new(chip_model: ChipModel) -> Self {
        Self {
            ram: vec![0; RAM_SIZE].into_boxed_slice(),
            sid: Sid::new(chip_model),
        }
    }

    /// Loads binary data into RAM at the specified address.
    pub fn load(&mut self, address: u16, data: &[u8]) {
        let start = address as usize;
        let end = (start + data.len()).min(RAM_SIZE);
        self.ram[start..end].copy_from_slice(&data[..end - start]);
    }

    /// Clears zero page ($0000-$00FF) and stack ($0100-$01FF).
    pub fn clear_zeropage_and_stack(&mut self) {
        self.ram[0x0000..0x0200].fill(0);
    }

    /// Replace the SID chip with a new instance of the specified model
    pub fn set_chip_model(&mut self, chip_model: ChipModel) {
        self.sid = Sid::new(chip_model);
    }
}

impl Bus for C64Memory {
    fn get_byte(&mut self, addr: u16) -> u8 {
        match addr {
            // SID register range is 0x00-0x1C, fits in u8
            #[allow(clippy::cast_possible_truncation)]
            SID_BASE..=SID_END => self.sid.read((addr - SID_BASE) as u8),
            _ => self.ram[addr as usize],
        }
    }

    fn set_byte(&mut self, addr: u16, val: u8) {
        match addr {
            #[allow(clippy::cast_possible_truncation)]
            SID_BASE..=SID_END => self.sid.write((addr - SID_BASE) as u8, val),
            _ => self.ram[addr as usize] = val,
        }
    }
}
