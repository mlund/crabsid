// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

use md5::{Digest, Md5};
use residfp::ChipModel;
use std::fs;
use std::io;
use std::path::Path;

/// Test-only mini-assembler shared by the synthetic SID fixtures.
#[cfg(test)]
mod fixture_asm {
    pub const OP_SEI: u8 = 0x78;
    pub const OP_CLI: u8 = 0x58;
    pub const OP_LDA_IMM: u8 = 0xA9;
    pub const OP_LDA_ABS: u8 = 0xAD;
    pub const OP_STA_ABS: u8 = 0x8D;
    pub const OP_INC_ABS: u8 = 0xEE;
    pub const OP_RTI: u8 = 0x40;
    pub const OP_RTS: u8 = 0x60;

    // CIA1 register addresses our fixture init code writes to.
    pub const CIA1_TIMER_A_LO: u16 = 0xDC04;
    pub const CIA1_TIMER_A_HI: u16 = 0xDC05;
    pub const CIA1_ICR: u16 = 0xDC0D;
    pub const CIA1_CRA: u16 = 0xDC0E;
    pub const USER_IRQ_VECTOR_LO: u16 = 0x0314;
    pub const USER_IRQ_VECTOR_HI: u16 = 0x0315;

    pub const fn lda_imm(val: u8) -> [u8; 2] {
        [OP_LDA_IMM, val]
    }
    pub const fn sta_abs(addr: u16) -> [u8; 3] {
        let [lo, hi] = addr.to_le_bytes();
        [OP_STA_ABS, lo, hi]
    }
    pub const fn lda_abs(addr: u16) -> [u8; 3] {
        let [lo, hi] = addr.to_le_bytes();
        [OP_LDA_ABS, lo, hi]
    }
    pub const fn inc_abs(addr: u16) -> [u8; 3] {
        let [lo, hi] = addr.to_le_bytes();
        [OP_INC_ABS, lo, hi]
    }
}

/// Primary SID register base address on the C64 ($D400).
pub const PRIMARY_SID_ADDRESS: u16 = 0xD400;

// PSID/RSID header field offsets (big-endian format)
const HEADER_MIN_SIZE: usize = 0x76;
const OFFSET_VERSION: usize = 0x04;
const OFFSET_DATA: usize = 0x06;
const OFFSET_LOAD: usize = 0x08;
const OFFSET_INIT: usize = 0x0A;
const OFFSET_PLAY: usize = 0x0C;
const OFFSET_SONGS: usize = 0x0E;
const OFFSET_START: usize = 0x10;
const OFFSET_SPEED: usize = 0x12;
const OFFSET_NAME: usize = 0x16;
const OFFSET_AUTHOR: usize = 0x36;
const OFFSET_RELEASED: usize = 0x56;
const OFFSET_FLAGS: usize = 0x76;
const OFFSET_SECOND_SID: usize = 0x7A;
const OFFSET_THIRD_SID: usize = 0x7B;

/// Parsed PSID/RSID file containing a C64 SID tune.
///
/// The PSID format stores 6502 machine code along with metadata
/// (title, author, release info) and playback parameters.
#[derive(Debug)]
pub struct SidFile {
    /// File format identifier ("PSID" or "RSID"). Read by [`SidFile::is_interrupt_driven`].
    #[allow(dead_code)] // exercised only via a #[cfg(test)] caller in production builds
    pub magic: String,
    /// PSID version (1, 2, 3, or 4)
    pub version: u16,
    /// Offset to binary data in original file
    #[allow(dead_code)] // Parsed for completeness
    pub data_offset: u16,
    /// C64 memory address where data is loaded
    pub load_address: u16,
    /// Entry point for song initialization
    pub init_address: u16,
    /// Entry point called each frame during playback
    pub play_address: u16,
    /// Number of songs in the file
    pub songs: u16,
    /// Default song to play (1-indexed)
    pub start_song: u16,
    /// Per-song timing flags (bit set = CIA, clear = VBI)
    pub speed: u32,
    /// Song title from file header
    pub name: String,
    /// Composer/artist name
    pub author: String,
    /// Release year and publisher
    pub released: String,
    /// v2+ flags: video standard, SID model, etc.
    pub flags: u16,
    /// 6502 machine code and data
    pub data: Vec<u8>,
    /// MD5 hash of original file (for Songlengths lookup)
    pub md5: String,
    /// v3+ second SID address (e.g., $D420, $D500)
    pub second_sid_address: Option<u16>,
    /// v3+ third SID address
    pub third_sid_address: Option<u16>,
}

impl SidFile {
    /// Loads and parses a PSID/RSID file from disk.
    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        Self::parse(&bytes)
    }

    /// Parses PSID/RSID data from a byte slice.
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < HEADER_MIN_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "File too small"));
        }

        // Compute MD5 hash of original file for Songlengths lookup
        let md5 = format!("{:x}", Md5::digest(bytes));

        let magic = String::from_utf8_lossy(&bytes[0..4]).to_string();
        if magic != "PSID" && magic != "RSID" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid magic: {magic}"),
            ));
        }

        let version = read_u16_be(&bytes[OFFSET_VERSION..]);
        let data_offset = read_u16_be(&bytes[OFFSET_DATA..]);
        let mut load_address = read_u16_be(&bytes[OFFSET_LOAD..]);
        let init_address = read_u16_be(&bytes[OFFSET_INIT..]);
        let play_address = read_u16_be(&bytes[OFFSET_PLAY..]);
        let songs = read_u16_be(&bytes[OFFSET_SONGS..]);
        let start_song = read_u16_be(&bytes[OFFSET_START..]);
        let speed = read_u32_be(&bytes[OFFSET_SPEED..]);

        let name = read_string(&bytes[OFFSET_NAME..OFFSET_AUTHOR]);
        let author = read_string(&bytes[OFFSET_AUTHOR..OFFSET_RELEASED]);
        let released = read_string(&bytes[OFFSET_RELEASED..OFFSET_FLAGS]);

        let flags = if version >= 2 && bytes.len() > OFFSET_FLAGS + 1 {
            read_u16_be(&bytes[OFFSET_FLAGS..])
        } else {
            0
        };

        // v3+ multi-SID addresses (byte encodes high nybble of $Dxx0)
        let (second_sid_address, third_sid_address) =
            if version >= 3 && bytes.len() > OFFSET_THIRD_SID {
                (
                    parse_sid_address(bytes[OFFSET_SECOND_SID]),
                    parse_sid_address(bytes[OFFSET_THIRD_SID]),
                )
            } else {
                (None, None)
            };

        let Some(data_slice) = bytes.get(data_offset as usize..) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Data offset beyond file",
            ));
        };
        let mut data = data_slice.to_vec();

        // PSID spec: load_address == 0 means the actual address is stored
        // in the first two bytes of the data section (little-endian C64 format)
        if load_address == 0 && data.len() >= 2 {
            load_address = u16::from_le_bytes([data[0], data[1]]);
            data.drain(..2);
        }

        Ok(Self {
            magic,
            version,
            data_offset,
            load_address,
            init_address,
            play_address,
            songs,
            start_song,
            speed,
            name,
            author,
            released,
            flags,
            data,
            md5,
            second_sid_address,
            third_sid_address,
        })
    }

    /// Returns true if the tune should use PAL timing (50Hz).
    ///
    /// Most European C64 software used PAL; NTSC (60Hz) was common in North America.
    /// Defaults to PAL for v1 files or when the flag indicates PAL-compatible.
    pub const fn is_pal(&self) -> bool {
        if self.version >= 2 {
            let video_standard = (self.flags >> 2) & 0x03;
            video_standard != 2 // Not NTSC-only
        } else {
            true // Default to PAL
        }
    }

    /// Returns true if the song uses CIA timer-based playback instead of VBI.
    ///
    /// Most tunes sync to the vertical blank interrupt (50/60Hz), but some
    /// use CIA timers for custom playback rates ("2x speed" tunes set this).
    #[allow(dead_code)] // Public API; player caches `speed` raw to avoid borrowing SidFile in load_song.
    pub const fn uses_cia_timing(&self, song: u16) -> bool {
        if song == 0 || song > 32 {
            return false;
        }
        (self.speed >> (song - 1)) & 1 != 0
    }

    /// Returns true if the tune is interrupt-driven (CIA1 IRQ or KERNAL-emulation).
    ///
    /// RSID files always set their own IRQ vector during init; PSID files with
    /// `play_address == 0` are the "PSID with KERNAL emulation" variant and use
    /// the same path. Either way, the player runs with no per-frame `JSR`.
    #[allow(dead_code)] // public API, used by tests; production code reads play_address directly
    pub fn is_interrupt_driven(&self) -> bool {
        self.magic == "RSID" || self.play_address == 0
    }

    /// Returns the number of SID chips used (1, 2, or 3).
    pub const fn sid_count(&self) -> usize {
        match (self.second_sid_address, self.third_sid_address) {
            (Some(_), Some(_)) => 3,
            (Some(_), None) => 2,
            _ => 1,
        }
    }

    /// Returns the register base addresses for each SID, ordered by chip index.
    pub fn sid_addresses(&self) -> Vec<u16> {
        let mut addrs = vec![PRIMARY_SID_ADDRESS];
        addrs.extend(self.second_sid_address);
        addrs.extend(self.third_sid_address);
        addrs
    }

    /// Returns the preferred chip model per SID, applying `override_model` to all if given.
    /// Otherwise reads PSID v2+ flag bits (4-5 = SID1, 6-7 = SID2, 8-9 = SID3; value 2 = 8580).
    pub fn preferred_chip_models(&self, override_model: Option<ChipModel>) -> Vec<ChipModel> {
        (0..self.sid_count())
            .map(|i| override_model.unwrap_or_else(|| self.chip_model_for_sid(i)))
            .collect()
    }

    fn chip_model_for_sid(&self, index: usize) -> ChipModel {
        if self.version < 2 {
            return ChipModel::Mos6581;
        }
        let shift = 4 + index * 2;
        let model = (self.flags >> shift) & 0x03;
        // 0=unknown, 1=6581, 2=8580, 3=6581+8580 (PSID spec)
        if model == 2 {
            ChipModel::Mos8580
        } else {
            ChipModel::Mos6581
        }
    }

    /// A minimal PSID fixture with the `speed` bit set for song 1 (PSID-CIA mode).
    /// `init` arms CIA1 Timer A but does NOT install an IRQ vector — by PSID-CIA
    /// convention the player polls the underflow flag and drives `play()` itself.
    /// `play` increments a counter at `$0400` so tests can verify it ran.
    #[cfg(test)]
    pub fn psid_cia_test_fixture() -> Self {
        use fixture_asm::*;
        const LOAD_ADDR: u16 = 0x1000;
        const PLAY_ADDR: u16 = LOAD_ADDR + 0x40;
        const COUNTER_ADDR: u16 = 0x0400;
        const TIMER_LATCH: u16 = 0x0400;

        let mut code: Vec<u8> = Vec::new();
        // init: just program the timer — no SEI/CLI, no IRQ vector install.
        code.extend_from_slice(&lda_imm(TIMER_LATCH as u8));
        code.extend_from_slice(&sta_abs(CIA1_TIMER_A_LO));
        code.extend_from_slice(&lda_imm((TIMER_LATCH >> 8) as u8));
        code.extend_from_slice(&sta_abs(CIA1_TIMER_A_HI));
        code.extend_from_slice(&lda_imm(crate::cia::CRA_START | crate::cia::CRA_FORCE_LOAD));
        code.extend_from_slice(&sta_abs(CIA1_CRA));
        code.push(OP_RTS);

        // play (called by player on each CIA underflow): bump the counter, return.
        let play_offset = (PLAY_ADDR - LOAD_ADDR) as usize;
        code.resize(play_offset, 0);
        code.extend_from_slice(&inc_abs(COUNTER_ADDR));
        code.push(OP_RTS);

        // Speed bit 0 set → song 1 is CIA-driven.
        Self::test_skeleton("PSID", PLAY_ADDR, 1, code)
    }

    /// A minimal RSID-format fixture for testing the CIA-IRQ playback path.
    ///
    /// Synthesises a tiny program: `init` installs an IRQ handler at `$1040` and
    /// arms CIA1 Timer A; the handler increments a counter at `$0400` then ack's
    /// the CIA ICR. Tests read `memory.ram_byte($0400)` to verify IRQs fired.
    #[cfg(test)]
    pub fn rsid_test_fixture() -> Self {
        use fixture_asm::*;
        const LOAD_ADDR: u16 = 0x1000;
        const HANDLER_ADDR: u16 = LOAD_ADDR + 0x40;
        const COUNTER_ADDR: u16 = 0x0400;
        // Latch ~1024 cycles → IRQ every ~1ms; several fires per audio buffer.
        const TIMER_LATCH: u16 = 0x0400;

        let mut code: Vec<u8> = Vec::new();
        // SEI: block IRQs while we install the vector.
        code.push(OP_SEI);
        // Install user IRQ handler at $0314/$0315.
        code.extend_from_slice(&lda_imm(HANDLER_ADDR as u8));
        code.extend_from_slice(&sta_abs(USER_IRQ_VECTOR_LO));
        code.extend_from_slice(&lda_imm((HANDLER_ADDR >> 8) as u8));
        code.extend_from_slice(&sta_abs(USER_IRQ_VECTOR_HI));
        // Configure CIA1 Timer A latch and arm the timer + IRQ.
        code.extend_from_slice(&lda_imm(TIMER_LATCH as u8));
        code.extend_from_slice(&sta_abs(CIA1_TIMER_A_LO));
        code.extend_from_slice(&lda_imm((TIMER_LATCH >> 8) as u8));
        code.extend_from_slice(&sta_abs(CIA1_TIMER_A_HI));
        code.extend_from_slice(&lda_imm(crate::cia::CRA_START | crate::cia::CRA_FORCE_LOAD));
        code.extend_from_slice(&sta_abs(CIA1_CRA));
        code.extend_from_slice(&lda_imm(crate::cia::ICR_FILL_BIT | crate::cia::ICR_TIMER_A));
        code.extend_from_slice(&sta_abs(CIA1_ICR));
        code.push(OP_CLI);
        code.push(OP_RTS);

        // Pad to handler offset, then emit the IRQ handler.
        let handler_offset = (HANDLER_ADDR - LOAD_ADDR) as usize;
        code.resize(handler_offset, 0);
        code.extend_from_slice(&inc_abs(COUNTER_ADDR));
        code.extend_from_slice(&lda_abs(CIA1_ICR)); // ack so the IRQ line drops
        code.push(OP_RTI);

        // RSID convention: play_address is 0; tunes drive playback via IRQ.
        Self::test_skeleton("RSID", 0, 0, code)
    }

    /// Common shell for both fixtures — collapses the 14-field struct literal so
    /// the fixtures themselves only show the bits that vary (magic, play_address,
    /// speed bitmap, code).
    #[cfg(test)]
    fn test_skeleton(magic: &str, play_address: u16, speed: u32, data: Vec<u8>) -> Self {
        Self::test_skeleton_at(magic, 0x1000, 0x1000, play_address, speed, data)
    }

    /// `test_skeleton` variant with explicit load/init addresses for tunes that
    /// must live in BASIC or KERNAL ROM space (e.g. Cobra at $F100).
    #[cfg(test)]
    fn test_skeleton_at(
        magic: &str,
        load_address: u16,
        init_address: u16,
        play_address: u16,
        speed: u32,
        data: Vec<u8>,
    ) -> Self {
        Self {
            magic: magic.to_string(),
            version: 2,
            data_offset: 0x7c,
            load_address,
            init_address,
            play_address,
            songs: 1,
            start_song: 1,
            speed,
            name: String::new(),
            author: String::new(),
            released: String::new(),
            flags: 0,
            data,
            md5: String::new(),
            second_sid_address: None,
            third_sid_address: None,
        }
    }

    /// PSID fixture loaded into the KERNAL ROM area (init/play at $F000+).
    /// `init` writes `$0F` to the SID master volume; `play` is a no-op RTS.
    /// Without the PSID-driver `iomap` banking, the CPU would fetch RTS from
    /// the KERNAL stub at $F000 instead of running the tune's volume write.
    #[cfg(test)]
    pub fn psid_kernal_area_fixture() -> Self {
        use fixture_asm::*;
        const LOAD_ADDR: u16 = 0xF000;
        const PLAY_ADDR: u16 = LOAD_ADDR + 0x10;
        const SID_VOLUME: u16 = 0xD418;

        let mut code: Vec<u8> = Vec::new();
        code.extend_from_slice(&lda_imm(0x0F));
        code.extend_from_slice(&sta_abs(SID_VOLUME));
        code.push(OP_RTS);

        let play_offset = (PLAY_ADDR - LOAD_ADDR) as usize;
        code.resize(play_offset, 0);
        code.push(OP_RTS);

        Self::test_skeleton_at("PSID", LOAD_ADDR, LOAD_ADDR, PLAY_ADDR, 0, code)
    }

    /// A minimal silent PSID stub used when the player has nothing to play.
    pub fn silent() -> Self {
        Self {
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
            // Three RTS opcodes so init/play return immediately.
            data: vec![0x60, 0x60, 0x60],
            md5: String::new(),
            second_sid_address: None,
            third_sid_address: None,
        }
    }
}

fn read_u16_be(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn read_u32_be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Parses v3+ SID address byte: 0x42 -> $D420, 0x00 -> None.
/// The byte encodes (address - $D000) >> 4, so 0x42 means $D420.
fn parse_sid_address(byte: u8) -> Option<u16> {
    if byte == 0 {
        None
    } else {
        Some(0xD000 | (u16::from(byte) << 4))
    }
}

/// Reads a null-terminated Latin-1 string (ISO-8859-1, used in SID headers).
fn read_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    // Latin-1 maps directly to Unicode code points; trimming on the byte slice avoids
    // a second String allocation.
    bytes[..end]
        .trim_ascii()
        .iter()
        .map(|&b| b as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! test_sid {
        () => {
            SidFile {
                magic: "PSID".to_string(),
                version: 3,
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
                data: vec![],
                md5: String::new(),
                second_sid_address: None,
                third_sid_address: None,
            }
        };
    }

    #[test]
    fn parse_sid_address_none_for_zero() {
        assert_eq!(parse_sid_address(0x00), None);
    }

    #[test]
    fn parse_sid_address_d420() {
        assert_eq!(parse_sid_address(0x42), Some(0xD420));
    }

    #[test]
    fn parse_sid_address_d500() {
        assert_eq!(parse_sid_address(0x50), Some(0xD500));
    }

    #[test]
    fn parse_real_2sid_file() {
        let sid = SidFile::load("tests/Hexadecimal_2SID.sid").expect("load 2SID file");
        assert_eq!(sid.name, "Hexadecimal");
        assert_eq!(sid.version, 3);
        assert_eq!(sid.sid_count(), 2);
        assert_eq!(sid.second_sid_address, Some(0xD500));
        assert_eq!(sid.third_sid_address, None);
        // Both SIDs request 8580 (model bits = 2).
        let models = sid.preferred_chip_models(None);
        assert_eq!(models, vec![ChipModel::Mos8580, ChipModel::Mos8580]);
    }

    #[test]
    fn sid_count_single() {
        let sid = test_sid!();
        assert_eq!(sid.sid_count(), 1);
    }

    #[test]
    fn sid_count_dual() {
        let mut sid = test_sid!();
        sid.second_sid_address = Some(0xD420);
        assert_eq!(sid.sid_count(), 2);
    }

    #[test]
    fn sid_count_triple() {
        let mut sid = test_sid!();
        sid.second_sid_address = Some(0xD420);
        sid.third_sid_address = Some(0xD500);
        assert_eq!(sid.sid_count(), 3);
    }
}
