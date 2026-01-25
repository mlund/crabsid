// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

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
}

impl SidFile {
    /// Loads and parses a PSID/RSID file from disk.
    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        Self::parse(&bytes)
    }

    fn parse(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < HEADER_MIN_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "File too small"));
        }

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
}

fn read_u16_be(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn read_u32_be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn read_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}
