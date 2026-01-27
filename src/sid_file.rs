// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

use md5::{Digest, Md5};
use std::fs;
use std::io;
use std::path::Path;

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
    /// File format identifier ("PSID" or "RSID")
    #[allow(dead_code)] // Parsed for format validation
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
    #[allow(dead_code)] // For future CIA timing support
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

        let data_start = data_offset as usize;
        if data_start > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Data offset beyond file",
            ));
        }

        let mut data = bytes[data_start..].to_vec();

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
    /// use CIA timers for custom playback rates.
    #[allow(dead_code)] // For future CIA timing support
    pub const fn uses_cia_timing(&self, song: u16) -> bool {
        if song == 0 || song > 32 {
            return false;
        }
        (self.speed >> (song - 1)) & 1 != 0
    }

    /// Returns true if the file likely requires full C64 emulation.
    ///
    /// RSID files and interrupt-driven tunes need CIA/VIC emulation
    /// that this player doesn't provide, so they may fail to initialize.
    pub fn requires_full_emulation(&self) -> bool {
        self.magic == "RSID" || self.play_address == 0 || self.speed != 0
    }

    /// Returns the number of SID chips used (1, 2, or 3).
    pub const fn sid_count(&self) -> usize {
        match (self.second_sid_address, self.third_sid_address) {
            (Some(_), Some(_)) => 3,
            (Some(_), None) => 2,
            _ => 1,
        }
    }

    /// Returns the preferred chip model for the nth SID (0-indexed).
    /// Bits 4-5 of flags: first SID, bits 6-7: second SID, bits 8-9: third SID.
    pub fn chip_model_for_sid(&self, index: usize) -> Option<u8> {
        if self.version < 2 {
            return None;
        }
        let shift = 4 + index * 2;
        let model = (self.flags >> shift) & 0x03;
        // 0=unknown, 1=6581, 2=8580, 3=6581+8580
        if model == 0 { None } else { Some(model as u8) }
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
    bytes[..end]
        .iter()
        .map(|&b| b as char) // Latin-1 maps directly to Unicode code points
        .collect::<String>()
        .trim()
        .to_string()
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
        // Both SIDs request 8580 (model bits = 2)
        assert_eq!(sid.chip_model_for_sid(0), Some(2));
        assert_eq!(sid.chip_model_for_sid(1), Some(2));
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
