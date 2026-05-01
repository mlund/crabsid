// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! CIA1 (MOS 6526) emulation, scoped to what RSID files need.
//!
//! Real C64 CIAs have two timers, a 24-hour TOD clock, a serial port, and two
//! 8-bit I/O ports for keyboard / joysticks. We implement only Timer A and the
//! Interrupt Control Register — the rest of the chip is stubbed (writes
//! accepted; reads return `$FF` floating-bus). This is enough to drive the
//! ~80% of HVSC's RSID corpus that uses CIA1 Timer A IRQs for playback.

/// Register offsets within the CIA1 register file (mirrored every 16 bytes
/// across `$DC00..=$DCFF`).
const REG_TIMER_A_LO: u8 = 0x04;
const REG_TIMER_A_HI: u8 = 0x05;
const REG_ICR: u8 = 0x0D;
const REG_CRA: u8 = 0x0E;
/// CIA register file is 16 bytes; addresses above mirror.
const REG_MASK: u8 = 0x0F;

/// Control Register A bits. Exposed `pub(crate)` so test fixtures and other
/// modules can write CRA values without inlining hex literals.
pub(crate) const CRA_START: u8 = 1 << 0;
const CRA_MODE_ONESHOT: u8 = 1 << 3;
/// Force-load is a write-only strobe: copies latch into counter then clears itself.
pub(crate) const CRA_FORCE_LOAD: u8 = 1 << 4;

/// Interrupt Control Register source bits (only Timer A is acted on).
pub(crate) const ICR_TIMER_A: u8 = 1 << 0;
/// Set in the read value when any masked source is pending; mirrors the IRQ line.
const ICR_ANY: u8 = 1 << 7;
/// Bit 7 in writes selects set vs clear; bits 0-4 select which sources.
pub(crate) const ICR_FILL_BIT: u8 = 1 << 7;
const ICR_SOURCE_MASK: u8 = 0x1F;

/// Reads of unimplemented registers return floating-bus high.
const FLOATING_BUS: u8 = 0xFF;

/// Boot state of CIA1 Timer A on a real C64: KERNAL programs it for ~60Hz
/// (`$4025` = 16421 cycles). Many PSID-CIA tunes assume this is already running
/// and only adjust the latch — without re-issuing the start/force-load strobe —
/// so we must come out of reset already armed. The exact value matters less
/// than being non-zero and giving roughly VBI-rate firing for tunes that don't
/// reprogram the CIA at all.
const KERNAL_DEFAULT_LATCH: u16 = 0x4025;

/// MOS 6526 CIA1 — Timer A + Interrupt Control Register.
pub struct Cia1 {
    /// Decrementing 16-bit counter (per CPU cycle when `CRA_START` is set).
    counter_a: u16,
    /// Reloaded into `counter_a` on underflow (continuous) or force-load.
    latch_a: u16,
    /// Control register A; force-load strobe is acted on but not stored.
    cra: u8,
    /// Pending interrupt sources, bits 0-4.
    icr_pending: u8,
    /// Mask: bits 0-4 enable per-source IRQ assertion.
    icr_mask: u8,
}

impl Default for Cia1 {
    fn default() -> Self {
        // Boot already armed — see KERNAL_DEFAULT_LATCH note above.
        Self {
            counter_a: KERNAL_DEFAULT_LATCH,
            latch_a: KERNAL_DEFAULT_LATCH,
            cra: CRA_START,
            icr_pending: 0,
            icr_mask: 0,
        }
    }
}

impl Cia1 {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bus read of CIA1 register `reg` (low 4 bits significant; mirrors).
    pub fn read_register(&mut self, reg: u8) -> u8 {
        #[allow(clippy::cast_possible_truncation)]
        match reg & REG_MASK {
            REG_TIMER_A_LO => (self.counter_a & 0x00FF) as u8,
            REG_TIMER_A_HI => (self.counter_a >> 8) as u8,
            REG_ICR => {
                // Returning ICR atomically reads pending+IRQ line and clears both.
                let any = if self.irq_line() { ICR_ANY } else { 0 };
                let value = (self.icr_pending & ICR_SOURCE_MASK) | any;
                self.icr_pending = 0;
                value
            }
            REG_CRA => self.cra & !CRA_FORCE_LOAD, // strobe never reads back
            _ => FLOATING_BUS,
        }
    }

    /// Bus write of CIA1 register `reg`.
    pub fn write_register(&mut self, reg: u8, val: u8) {
        match reg & REG_MASK {
            REG_TIMER_A_LO => self.latch_a = (self.latch_a & 0xFF00) | u16::from(val),
            REG_TIMER_A_HI => self.latch_a = (self.latch_a & 0x00FF) | (u16::from(val) << 8),
            REG_ICR => {
                // Bit 7 = 1 → set the source bits in the mask; bit 7 = 0 → clear them.
                let sources = val & ICR_SOURCE_MASK;
                if val & ICR_FILL_BIT != 0 {
                    self.icr_mask |= sources;
                } else {
                    self.icr_mask &= !sources;
                }
            }
            REG_CRA => {
                if val & CRA_FORCE_LOAD != 0 {
                    self.counter_a = self.latch_a;
                }
                self.cra = val & !CRA_FORCE_LOAD;
            }
            _ => {} // other registers stub: accept but discard
        }
    }

    /// Advance Timer A by `cycles`. Sets ICR Timer A bit on each underflow;
    /// reloads from latch (continuous) or stops (one-shot).
    pub fn tick(&mut self, cycles: u64) {
        if self.cra & CRA_START == 0 {
            return;
        }
        // Pathological: latch=0 would underflow every cycle. One IRQ flag is enough.
        if self.latch_a == 0 {
            self.icr_pending |= ICR_TIMER_A;
            return;
        }
        let mut remaining = cycles;
        while remaining > 0 {
            let to_underflow = u64::from(self.counter_a) + 1;
            if remaining < to_underflow {
                #[allow(clippy::cast_possible_truncation)]
                {
                    self.counter_a -= remaining as u16;
                }
                return;
            }
            remaining -= to_underflow;
            self.icr_pending |= ICR_TIMER_A;
            if self.cra & CRA_MODE_ONESHOT != 0 {
                self.cra &= !CRA_START;
                self.counter_a = self.latch_a;
                return;
            }
            self.counter_a = self.latch_a;
        }
    }

    /// True when an enabled interrupt source is pending — drives `Bus::irq_pending`.
    pub fn irq_line(&self) -> bool {
        (self.icr_pending & self.icr_mask & ICR_SOURCE_MASK) != 0
    }

    /// Polls + clears the Timer A underflow flag without disturbing the mask.
    /// Used by PSID-CIA mode where the player drives `play()` directly instead
    /// of relying on the CPU to service an IRQ.
    pub fn take_timer_a_underflow(&mut self) -> bool {
        let fired = self.icr_pending & ICR_TIMER_A != 0;
        self.icr_pending &= !ICR_TIMER_A;
        fired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a CIA1 with Timer A primed: latch loaded, counter force-loaded,
    /// ICR mask enabled for Timer A, and timer started.
    macro_rules! cia_with_timer {
        ($latch:expr, mode = $mode:expr) => {{
            let mut cia = Cia1::new();
            cia.write_register(REG_TIMER_A_LO, ($latch & 0xFF) as u8);
            cia.write_register(REG_TIMER_A_HI, ($latch >> 8) as u8);
            cia.write_register(REG_ICR, ICR_FILL_BIT | ICR_TIMER_A);
            cia.write_register(REG_CRA, CRA_START | CRA_FORCE_LOAD | $mode);
            cia
        }};
    }

    #[test]
    fn tick_continuous_fires_periodically() {
        let mut cia = cia_with_timer!(10u16, mode = 0);
        assert!(!cia.irq_line());
        cia.tick(11); // counter: 10 → ... → underflow
        assert!(cia.irq_line());
        // Reading clears.
        cia.read_register(REG_ICR);
        assert!(!cia.irq_line());
        // Continues firing in continuous mode.
        cia.tick(11);
        assert!(cia.irq_line());
    }

    #[test]
    fn tick_oneshot_fires_once_then_stops() {
        let mut cia = cia_with_timer!(10u16, mode = CRA_MODE_ONESHOT);
        cia.tick(11);
        assert!(cia.irq_line());
        cia.read_register(REG_ICR);
        // After one-shot underflow CRA_START is cleared; no more firings.
        cia.tick(100);
        assert!(!cia.irq_line());
    }

    #[test]
    fn icr_read_clears_pending_and_irq_line() {
        let mut cia = cia_with_timer!(5u16, mode = 0);
        cia.tick(6);
        let value = cia.read_register(REG_ICR);
        assert_eq!(value & ICR_TIMER_A, ICR_TIMER_A);
        assert_eq!(value & ICR_ANY, ICR_ANY);
        assert_eq!(cia.read_register(REG_ICR), 0);
        assert!(!cia.irq_line());
    }

    #[test]
    fn mask_gates_irq_line() {
        // No mask: pending sets bit 0 of ICR but irq_line stays low.
        let mut cia = Cia1::new();
        cia.write_register(REG_TIMER_A_LO, 5);
        cia.write_register(REG_TIMER_A_HI, 0);
        cia.write_register(REG_CRA, CRA_START | CRA_FORCE_LOAD);
        cia.tick(6);
        assert!(!cia.irq_line());
        // Then enable mask: line goes high without further ticking.
        cia.write_register(REG_ICR, ICR_FILL_BIT | ICR_TIMER_A);
        assert!(cia.irq_line());
    }

    #[test]
    fn force_load_strobe_does_not_persist() {
        let mut cia = Cia1::new();
        cia.write_register(REG_CRA, CRA_START | CRA_FORCE_LOAD);
        // Force-load bit must read back as 0.
        assert_eq!(cia.read_register(REG_CRA) & CRA_FORCE_LOAD, 0);
        assert_eq!(cia.read_register(REG_CRA) & CRA_START, CRA_START);
    }
}
