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

#[derive(Debug)]
pub struct SidFile {
    #[allow(dead_code)] // Parsed for format validation
    pub magic: String,
    pub version: u16,
    #[allow(dead_code)] // Parsed for completeness
    pub data_offset: u16,
    pub load_address: u16,
    pub init_address: u16,
    pub play_address: u16,
    pub songs: u16,
    pub start_song: u16,
    #[allow(dead_code)] // For future CIA timing support
    pub speed: u32,
    pub name: String,
    pub author: String,
    pub released: String,
    pub flags: u16,
    pub data: Vec<u8>,
}

impl SidFile {
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

    pub const fn is_pal(&self) -> bool {
        if self.version >= 2 {
            let video_standard = (self.flags >> 2) & 0x03;
            video_standard != 2 // Not NTSC-only
        } else {
            true // Default to PAL
        }
    }

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
