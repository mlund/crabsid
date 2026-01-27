// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! CrabSid - A SID music player for Commodore 64 .sid files.

#![deny(missing_docs)]

mod hvsc;
mod memory;
mod player;
mod playlist;
mod sid_file;
mod tui;

use clap::Parser;
use player::create_shared_player;
use playlist::Playlist;
use sid_file::SidFile;
use std::path::PathBuf;
use tinyaudio::prelude::*;

const SAMPLE_RATE: u32 = 44100;
const BUFFER_SIZE: usize = 1024;

#[derive(Parser)]
#[command(name = "crabsid", version, about = "C64 SID music player in pure Rust")]
struct Args {
    /// SID file(s) to play or add to playlist
    #[arg(name = "FILE")]
    files: Vec<PathBuf>,

    /// Path to .m3u playlist file
    #[arg(short = 'l', long)]
    playlist: Option<PathBuf>,

    /// Song number to play (default: start song from file)
    #[arg(short, long)]
    song: Option<u16>,

    /// SID chip model: 6581 or 8580 (default: from file)
    #[arg(short, long)]
    chip: Option<u16>,

    /// Disable TUI and use simple text output
    #[arg(long)]
    no_tui: bool,

    /// HVSC mirror base URL
    #[arg(long, default_value = hvsc::DEFAULT_HVSC_URL)]
    hvsc_url: String,
}

fn default_playlist_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("crabsid")
        .join("playlist.m3u")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Load existing playlist or create new one, then append CLI files as absolute paths
    let playlist_path = args.playlist.clone().unwrap_or_else(default_playlist_path);
    let mut playlist = Playlist::load_or_create(&playlist_path)?;
    let mut playlist_modified = false;
    for file in &args.files {
        let absolute = file.canonicalize().unwrap_or_else(|_| file.clone());
        playlist_modified |= playlist.add(&absolute.to_string_lossy(), None);
    }

    // Determine initial SID file to play
    let (sid_file, initial_song) = if !args.files.is_empty() {
        // Play first file from CLI
        let sid = SidFile::load(&args.files[0])?;
        let song = args.song.unwrap_or(sid.start_song);
        (sid, song)
    } else if !playlist.is_empty() {
        // Play first from playlist
        let entry = &playlist.entries[0];
        let sid = entry.load()?;
        let song = args.song.or(entry.subsong).unwrap_or(sid.start_song);
        (sid, song)
    } else {
        // Empty playlist, no files - need a dummy SID for player init
        // TUI will start with HVSC browser focused
        let dummy = create_silent_sid();
        (dummy, 1)
    };

    if sid_file.requires_full_emulation() {
        return Err("Unsupported RSID-like format (requires CIA/interrupt emulation)".into());
    }

    let player = create_shared_player(&sid_file, initial_song, SAMPLE_RATE, args.chip)
        .map_err(|e| format!("{e}"))?;

    let params = OutputDeviceParameters {
        channels_count: 1,
        sample_rate: SAMPLE_RATE as usize,
        channel_sample_count: BUFFER_SIZE,
    };

    // Audio callback runs in separate thread
    let _device = run_output_device(params, {
        let player = player.clone();
        move |data| {
            if let Ok(mut p) = player.lock()
                && let Err(e) = p.fill_buffer(data)
            {
                eprintln!("Playback error: {e}");
            }
        }
    })?;

    if args.no_tui {
        run_simple(&sid_file, initial_song)?;
    } else {
        let focus_hvsc = args.files.is_empty() && playlist.is_empty();
        tui::run_tui(
            player,
            &sid_file,
            initial_song,
            playlist,
            playlist_path,
            focus_hvsc,
            playlist_modified,
            &args.hvsc_url,
        )?;
    }

    Ok(())
}

/// Creates a minimal silent SID for when no file is loaded.
fn create_silent_sid() -> SidFile {
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
        data: vec![0x60, 0x60, 0x60], // RTS instructions
    }
}

fn run_simple(sid_file: &SidFile, song: u16) -> Result<(), Box<dyn std::error::Error>> {
    println!("Title:    {}", sid_file.name);
    println!("Author:   {}", sid_file.author);
    println!("Released: {}", sid_file.released);
    println!("Songs:    {}", sid_file.songs);
    println!("Playing song {} of {}", song, sid_file.songs);
    println!("Press Ctrl+C to stop");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
