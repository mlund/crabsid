// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

use mos6502::memory::Bus;
use resid::{ChipModel, Sid};

const RAM_SIZE: usize = 65536;
const SID_BASE: u16 = 0xD400;
const SID_END: u16 = 0xD41C;

pub struct C64Memory {
    ram: Box<[u8]>,
    pub sid: Sid,
}

impl C64Memory {
    pub fn new(chip_model: ChipModel) -> Self {
        Self {
            ram: vec![0; RAM_SIZE].into_boxed_slice(),
            sid: Sid::new(chip_model),
        }
    }

    pub fn load(&mut self, address: u16, data: &[u8]) {
        let start = address as usize;
        let end = (start + data.len()).min(RAM_SIZE);
        self.ram[start..end].copy_from_slice(&data[..end - start]);
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
