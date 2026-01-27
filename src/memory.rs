// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

use mos6502::memory::Bus;
use resid::{ChipModel, Sid};

const RAM_SIZE: usize = 65536;
const SID_REGISTER_COUNT: u16 = 0x20;

/// A SID chip with its base memory address.
pub struct SidChip {
    pub sid: Sid,
    pub base_address: u16,
}

impl SidChip {
    pub fn new(chip_model: ChipModel, base_address: u16) -> Self {
        Self {
            sid: Sid::new(chip_model),
            base_address,
        }
    }

    /// Returns true if the address falls within this SID's register range.
    fn contains(&self, addr: u16) -> bool {
        addr >= self.base_address && addr < self.base_address + SID_REGISTER_COUNT
    }
}

/// Emulated C64 memory map with 1-3 SID chips.
///
/// Provides 64KB RAM with memory-mapped I/O for SID sound chips.
/// Primary SID at $D400, optional second/third at configurable addresses.
/// All other I/O areas (VIC, CIA, etc.) are treated as plain RAM since
/// SID playback only requires the sound chips.
pub struct C64Memory {
    /// 64KB RAM, heap-allocated to avoid stack overflow
    ram: Box<[u8]>,
    /// SID sound chips (1-3), each at their configured address
    pub sids: Vec<SidChip>,
}

impl C64Memory {
    /// Creates memory with zeroed RAM and a single SID chip at $D400.
    pub fn new(chip_model: ChipModel) -> Self {
        Self {
            ram: vec![0; RAM_SIZE].into_boxed_slice(),
            sids: vec![SidChip::new(chip_model, 0xD400)],
        }
    }

    /// Configures SID chips from (base_address, chip_model) pairs.
    /// First entry should always be $D400 for the primary SID.
    pub fn configure_sids(&mut self, configs: &[(u16, ChipModel)]) {
        self.sids = configs
            .iter()
            .map(|&(addr, model)| SidChip::new(model, addr))
            .collect();
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

    /// Replace the chip model for a specific SID (by index).
    pub fn set_chip_model(&mut self, index: usize, chip_model: ChipModel) {
        if let Some(sid_chip) = self.sids.get_mut(index) {
            let base = sid_chip.base_address;
            *sid_chip = SidChip::new(chip_model, base);
        }
    }
}

impl Bus for C64Memory {
    fn get_byte(&mut self, addr: u16) -> u8 {
        for sid_chip in &mut self.sids {
            if sid_chip.contains(addr) {
                #[allow(clippy::cast_possible_truncation)]
                return sid_chip.sid.read((addr - sid_chip.base_address) as u8);
            }
        }
        self.ram[addr as usize]
    }

    fn set_byte(&mut self, addr: u16, val: u8) {
        for sid_chip in &mut self.sids {
            if sid_chip.contains(addr) {
                #[allow(clippy::cast_possible_truncation)]
                sid_chip
                    .sid
                    .write((addr - sid_chip.base_address) as u8, val);
                return;
            }
        }
        self.ram[addr as usize] = val;
    }
}
