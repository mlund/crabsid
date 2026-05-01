// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! Emulated C64 memory map: RAM, banked KERNAL stub, SID chips, CIA1.

use crate::cia::Cia1;
use crate::sid_file::PRIMARY_SID_ADDRESS;
use mos6502::memory::Bus;
use residfp::{ChipModel, Sid};

const RAM_SIZE: usize = 65536;
const ZEROPAGE_AND_STACK_END: usize = 0x0200;
const SID_REGISTER_COUNT: u16 = 0x20;
const SID_RANGE_START: u16 = 0xD400;
const SID_RANGE_END: u16 = 0xD800;

// Banking control registers in zero page.
const BANK_DATA_DIR_ADDR: u16 = 0x0000;
const BANK_CONFIG_ADDR: u16 = 0x0001;
/// Default `$00` data direction (LORAM/HIRAM/CHAREN bits + tape lines as outputs).
const BANK_DATA_DIR_DEFAULT: u8 = 0x2F;
/// Default `$01`: KERNAL + BASIC + I/O all banked in.
const BANK_DEFAULT: u8 = 0x37;
/// HIRAM bit gates KERNAL ROM at `$E000-$FFFF`. LORAM (BASIC) and CHAREN (CHARROM)
/// are deliberately ignored — RSID files driven by CIA1 IRQs only need the KERNAL
/// IRQ trampoline; pulling extra ROMs in would require redistributable ROM images.
const BANK_HIRAM_BIT: u8 = 1 << 1;

// KERNAL stub region.
const KERNAL_BASE: u16 = 0xE000;
const KERNAL_SIZE: usize = 0x2000;

// Stub-internal addresses where we patch specific opcodes/vectors.
const KERNAL_IRQ_ENTRY: u16 = 0xFF48;
const KERNAL_IRQ_EXIT: u16 = 0xFF8E;
const HW_IRQ_VECTOR_LO: u16 = 0xFFFE;
const HW_IRQ_VECTOR_HI: u16 = 0xFFFF;
const HW_NMI_VECTOR_LO: u16 = 0xFFFA;
const HW_NMI_VECTOR_HI: u16 = 0xFFFB;

/// User-installed IRQ vector in RAM. Tunes overwrite this in init to point at their
/// handler; default points at the stub's RTI so pre-init IRQs are silent.
const USER_IRQ_VECTOR_LO: u16 = 0x0314;
const USER_IRQ_VECTOR_HI: u16 = 0x0315;

const OPCODE_JMP_INDIRECT: u8 = 0x6C;
const OPCODE_RTI: u8 = 0x40;
/// Default fill byte for the KERNAL stub. RTS keeps stray calls from corrupting
/// the stack; only the IRQ exit address is RTI.
const OPCODE_RTS: u8 = 0x60;

// CIA1 register window (mirrored across $DC00..=$DCFF).
const CIA1_BASE: u16 = 0xDC00;
const CIA1_END: u16 = 0xDD00;

// Out-of-scope hardware: VIC raster IRQ and CIA2 NMI. Reads return floating bus,
// writes are recorded so the player can refuse to play the file.
const VIC_IRQ_FLAG: u16 = 0xD019;
const VIC_IRQ_MASK: u16 = 0xD01A;
const CIA2_ICR: u16 = 0xDD0D;
const CIA2_CRA: u16 = 0xDD0E;
const FLOATING_BUS: u8 = 0xFF;

const KERNAL_STUB: [u8; KERNAL_SIZE] = build_kernel_stub();

const fn build_kernel_stub() -> [u8; KERNAL_SIZE] {
    let mut stub = [OPCODE_RTS; KERNAL_SIZE];
    let [entry_lo, entry_hi] = KERNAL_IRQ_ENTRY.to_le_bytes();
    let [user_lo, user_hi] = USER_IRQ_VECTOR_LO.to_le_bytes();

    // Hardware IRQ + NMI vectors → KERNAL IRQ entry. We don't service NMI
    // separately, but the vector cannot be 0 or random ROM byte.
    stub[(HW_IRQ_VECTOR_LO - KERNAL_BASE) as usize] = entry_lo;
    stub[(HW_IRQ_VECTOR_HI - KERNAL_BASE) as usize] = entry_hi;
    stub[(HW_NMI_VECTOR_LO - KERNAL_BASE) as usize] = entry_lo;
    stub[(HW_NMI_VECTOR_HI - KERNAL_BASE) as usize] = entry_hi;

    // $FF48: JMP ($0314) — trampoline through user IRQ vector.
    let entry = (KERNAL_IRQ_ENTRY - KERNAL_BASE) as usize;
    stub[entry] = OPCODE_JMP_INDIRECT;
    stub[entry + 1] = user_lo;
    stub[entry + 2] = user_hi;

    // $FF8E: RTI — exit point for handlers that JMP here to return.
    stub[(KERNAL_IRQ_EXIT - KERNAL_BASE) as usize] = OPCODE_RTI;

    stub
}

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

    fn contains(&self, addr: u16) -> bool {
        addr >= self.base_address && addr < self.base_address + SID_REGISTER_COUNT
    }
}

/// Emulated C64 memory map.
///
/// Hosts 64KB RAM, 1-3 SID chips, CIA1 (Timer A + ICR), an 8KB KERNAL stub, and
/// `$00/$01` HIRAM banking. CIA2 / VIC raster IRQ are out of scope: writes to
/// their interrupt-control registers are recorded in `unsupported_hardware` so
/// the player can refuse the file rather than play it silently broken.
pub struct C64Memory {
    ram: Box<[u8]>,
    pub sids: Vec<SidChip>,
    cia1: Cia1,
    bank_config: u8,
    unsupported_hardware: Vec<u16>,
    /// Whether CIA1's IRQ line is routed to the CPU. Off by default so PSID
    /// tunes (frame or CIA-poll) never have mos6502 service spurious IRQs;
    /// `Player` flips it on for RSID-style tunes that expect vector dispatch.
    cia_irq_routed: bool,
}

impl C64Memory {
    /// Creates memory with a single SID chip at `$D400` and C64 default banking.
    pub fn new(chip_model: ChipModel) -> Self {
        let mut mem = Self {
            ram: vec![0; RAM_SIZE].into_boxed_slice(),
            sids: vec![SidChip::new(chip_model, PRIMARY_SID_ADDRESS)],
            cia1: Cia1::new(),
            bank_config: BANK_DEFAULT,
            unsupported_hardware: Vec::new(),
            cia_irq_routed: false,
        };
        mem.init_zeropage_defaults();
        mem
    }

    /// Configures SID chips from `(base_address, chip_model)` pairs.
    /// First entry should always be `$D400` for the primary SID.
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

    /// Resets zero page + stack and re-arms banking, CIA1, and the IRQ vector.
    /// Called between songs so init starts from a clean machine state.
    pub fn clear_zeropage_and_stack(&mut self) {
        self.ram[..ZEROPAGE_AND_STACK_END].fill(0);
        self.bank_config = BANK_DEFAULT;
        self.cia1 = Cia1::new();
        self.unsupported_hardware.clear();
        self.init_zeropage_defaults();
    }

    fn init_zeropage_defaults(&mut self) {
        let [exit_lo, exit_hi] = KERNAL_IRQ_EXIT.to_le_bytes();
        self.ram[BANK_DATA_DIR_ADDR as usize] = BANK_DATA_DIR_DEFAULT;
        self.ram[USER_IRQ_VECTOR_LO as usize] = exit_lo;
        self.ram[USER_IRQ_VECTOR_HI as usize] = exit_hi;
    }

    /// Replace the chip model for a specific SID (by index).
    pub fn set_chip_model(&mut self, index: usize, chip_model: ChipModel) {
        if let Some(sid_chip) = self.sids.get_mut(index) {
            let base = sid_chip.base_address;
            *sid_chip = SidChip::new(chip_model, base);
        }
    }

    /// Advances per-cycle peripherals (CIA1) by `cycles`.
    pub fn tick(&mut self, cycles: u64) {
        self.cia1.tick(cycles);
    }

    /// PSID-CIA poll: did Timer A underflow since last call? Clears the flag.
    pub fn cia1_take_timer_a_underflow(&mut self) -> bool {
        self.cia1.take_timer_a_underflow()
    }

    /// Toggles whether CIA1's IRQ line drives `Bus::irq_pending`. Only RSID-style
    /// tunes want this — PSID-CIA polls the underflow flag directly via
    /// `cia1_take_timer_a_underflow` and would be disrupted by mos6502 servicing.
    pub fn set_cia_irq_routed(&mut self, routed: bool) {
        self.cia_irq_routed = routed;
    }

    /// Returns the addresses where init wrote to out-of-scope hardware (VIC raster
    /// IRQ or CIA2 NMI), if any. Player aborts loading on a non-empty list.
    pub fn unsupported_hardware(&self) -> &[u16] {
        &self.unsupported_hardware
    }

    /// Direct RAM read used by tests to verify CPU side effects.
    #[cfg(test)]
    pub fn ram_byte(&self, addr: u16) -> u8 {
        self.ram[addr as usize]
    }

    fn kernal_banked_in(&self) -> bool {
        self.bank_config & BANK_HIRAM_BIT != 0
    }
}

impl Bus for C64Memory {
    fn get_byte(&mut self, addr: u16) -> u8 {
        if addr == BANK_CONFIG_ADDR {
            return self.bank_config;
        }
        if addr >= KERNAL_BASE && self.kernal_banked_in() {
            return KERNAL_STUB[(addr - KERNAL_BASE) as usize];
        }
        if (CIA1_BASE..CIA1_END).contains(&addr) {
            #[allow(clippy::cast_possible_truncation)]
            return self.cia1.read_register((addr & 0x0F) as u8);
        }
        // VIC IRQ + CIA2 reads return floating-bus so polling tunes don't loop forever.
        if matches!(addr, VIC_IRQ_FLAG | VIC_IRQ_MASK | CIA2_ICR | CIA2_CRA) {
            return FLOATING_BUS;
        }
        if (SID_RANGE_START..SID_RANGE_END).contains(&addr) {
            for sid_chip in &mut self.sids {
                if sid_chip.contains(addr) {
                    #[allow(clippy::cast_possible_truncation)]
                    return sid_chip.sid.read((addr - sid_chip.base_address) as u8);
                }
            }
        }
        self.ram[addr as usize]
    }

    fn set_byte(&mut self, addr: u16, val: u8) {
        if addr == BANK_CONFIG_ADDR {
            self.bank_config = val;
            return;
        }
        if (CIA1_BASE..CIA1_END).contains(&addr) {
            #[allow(clippy::cast_possible_truncation)]
            self.cia1.write_register((addr & 0x0F) as u8, val);
            return;
        }
        if matches!(addr, VIC_IRQ_FLAG | VIC_IRQ_MASK | CIA2_ICR | CIA2_CRA) {
            // Record the trap, then fall through to RAM so the value is observable.
            self.unsupported_hardware.push(addr);
            self.ram[addr as usize] = val;
            return;
        }
        if (SID_RANGE_START..SID_RANGE_END).contains(&addr) {
            for sid_chip in &mut self.sids {
                if sid_chip.contains(addr) {
                    #[allow(clippy::cast_possible_truncation)]
                    sid_chip
                        .sid
                        .write((addr - sid_chip.base_address) as u8, val);
                    return;
                }
            }
        }
        self.ram[addr as usize] = val;
    }

    fn irq_pending(&mut self) -> bool {
        self.cia_irq_routed && self.cia1.irq_line()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> C64Memory {
        C64Memory::new(ChipModel::Mos6581)
    }

    #[test]
    fn kernal_banked_in_by_default() {
        let mut mem = fresh();
        // $FFFE/$FFFF are the IRQ vector — KERNAL stub points them at $FF48.
        let lo = mem.get_byte(HW_IRQ_VECTOR_LO);
        let hi = mem.get_byte(HW_IRQ_VECTOR_HI);
        assert_eq!(u16::from_le_bytes([lo, hi]), KERNAL_IRQ_ENTRY);
    }

    #[test]
    fn kernal_banked_out_when_hiram_clear() {
        let mut mem = fresh();
        mem.set_byte(BANK_CONFIG_ADDR, BANK_DEFAULT & !BANK_HIRAM_BIT);
        mem.ram[HW_IRQ_VECTOR_LO as usize] = 0xAB;
        assert_eq!(mem.get_byte(HW_IRQ_VECTOR_LO), 0xAB);
    }

    #[test]
    fn kernal_irq_entry_is_jmp_indirect() {
        let mut mem = fresh();
        assert_eq!(mem.get_byte(KERNAL_IRQ_ENTRY), OPCODE_JMP_INDIRECT);
        assert_eq!(
            mem.get_byte(KERNAL_IRQ_ENTRY + 1),
            (USER_IRQ_VECTOR_LO & 0xFF) as u8
        );
        assert_eq!(
            mem.get_byte(KERNAL_IRQ_ENTRY + 2),
            (USER_IRQ_VECTOR_LO >> 8) as u8
        );
    }

    #[test]
    fn kernal_default_fill_is_rts() {
        let mut mem = fresh();
        // An arbitrary byte in the stub that isn't a patched address.
        assert_eq!(mem.get_byte(0xE100), OPCODE_RTS);
    }

    use crate::cia::{CRA_FORCE_LOAD, CRA_START, ICR_FILL_BIT, ICR_TIMER_A};
    const CRA_ARM: u8 = CRA_START | CRA_FORCE_LOAD;
    const ICR_ENABLE_TIMER_A: u8 = ICR_FILL_BIT | ICR_TIMER_A;

    #[test]
    fn cia1_register_routing() {
        let mut mem = fresh();
        // Latch $1234, force-load + start, ICR mask Timer A.
        mem.set_byte(0xDC04, 0x34);
        mem.set_byte(0xDC05, 0x12);
        mem.set_byte(0xDC0D, ICR_ENABLE_TIMER_A);
        mem.set_byte(0xDC0E, CRA_ARM);
        // After force-load, counter holds $1234. Read low byte.
        assert_eq!(mem.get_byte(0xDC04), 0x34);
        assert_eq!(mem.get_byte(0xDC05), 0x12);
    }

    #[test]
    fn irq_pending_propagates_from_cia1_when_routed() {
        let mut mem = fresh();
        mem.set_cia_irq_routed(true);
        mem.set_byte(0xDC04, 5);
        mem.set_byte(0xDC05, 0);
        mem.set_byte(0xDC0D, ICR_ENABLE_TIMER_A);
        mem.set_byte(0xDC0E, CRA_ARM);
        assert!(!mem.irq_pending());
        mem.tick(6);
        assert!(mem.irq_pending());
    }

    #[test]
    fn irq_pending_suppressed_when_not_routed() {
        let mut mem = fresh();
        // Default routing is off — PSID/PSID-CIA mode.
        mem.set_byte(0xDC04, 5);
        mem.set_byte(0xDC05, 0);
        mem.set_byte(0xDC0D, ICR_ENABLE_TIMER_A);
        mem.set_byte(0xDC0E, CRA_ARM);
        mem.tick(6);
        assert!(
            !mem.irq_pending(),
            "PSID modes must not surface CIA IRQs to the CPU"
        );
    }

    #[test]
    fn unsupported_hardware_flag_trips_on_vic_irq_mask() {
        let mut mem = fresh();
        assert!(mem.unsupported_hardware().is_empty());
        mem.set_byte(VIC_IRQ_MASK, 0x01);
        assert_eq!(mem.unsupported_hardware(), &[VIC_IRQ_MASK]);
    }

    #[test]
    fn unsupported_hardware_flag_trips_on_cia2_icr() {
        let mut mem = fresh();
        mem.set_byte(CIA2_ICR, 0x81);
        assert_eq!(mem.unsupported_hardware(), &[CIA2_ICR]);
    }

    #[test]
    fn vic_irq_reads_return_floating_bus() {
        let mut mem = fresh();
        // Even after a write (which goes through to RAM), reads return $FF.
        mem.set_byte(VIC_IRQ_MASK, 0x42);
        assert_eq!(mem.get_byte(VIC_IRQ_MASK), FLOATING_BUS);
    }
}
