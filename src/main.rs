// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

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
#[command(name = "crabsid")]
#[command(about = "A SID music player for .sid files")]
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
}

fn default_playlist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".crabsid.m3u")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Determine playlist path and load/create it
    let playlist_path = args.playlist.clone().unwrap_or_else(default_playlist_path);
    let playlist = if args.files.is_empty() {
        // No files given: load existing or create empty default playlist
        Playlist::load_or_create(&playlist_path)?
    } else {
        // Files given: create new playlist with these files
        let mut pl = Playlist::new();
        for file in &args.files {
            pl.add(&file.to_string_lossy(), None);
        }
        pl
    };

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

    let player = create_shared_player(&sid_file, initial_song, SAMPLE_RATE, args.chip);

    let params = OutputDeviceParameters {
        channels_count: 1,
        sample_rate: SAMPLE_RATE as usize,
        channel_sample_count: BUFFER_SIZE,
    };

    // Audio callback runs in separate thread
    let _device = run_output_device(params, {
        let player = player.clone();
        move |data| {
            if let Ok(mut p) = player.lock() {
                p.fill_buffer(data);
            }
        }
    })?;

    if args.no_tui {
        run_simple(&sid_file, initial_song)?;
    } else {
        let focus_hvsc = args.files.is_empty() && playlist.is_empty();
        tui::run_tui(player, &sid_file, initial_song, playlist, playlist_path, focus_hvsc)?;
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
