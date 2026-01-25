// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

mod memory;
mod player;
mod sid_file;
mod tui;

use clap::Parser;
use player::create_shared_player;
use sid_file::SidFile;
use std::path::PathBuf;
use tinyaudio::prelude::*;

const SAMPLE_RATE: u32 = 44100;
const BUFFER_SIZE: usize = 1024;

#[derive(Parser)]
#[command(name = "crabsid")]
#[command(about = "A SID music player for .sid files")]
struct Args {
    /// Path to .sid file
    sid_file: PathBuf,

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let sid_file = SidFile::load(&args.sid_file)?;
    let song = args.song.unwrap_or(sid_file.start_song);

    let player = create_shared_player(&sid_file, song, SAMPLE_RATE, args.chip);

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
        run_simple(&sid_file, song)?;
    } else {
        tui::run(player, &sid_file, song)?;
    }

    Ok(())
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
