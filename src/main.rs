// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! CrabSid - A SID music player for Commodore 64 .sid files.

#![deny(missing_docs)]

mod audio;
mod cia;
mod config;
mod hvsc;
mod memory;
mod player;
mod playlist;
mod sid_file;
mod tui;

use audio::AudioOutput;
use clap::Parser;
use config::Config;
use player::{SamplingMethod, create_shared_player};
use playlist::Playlist;
use residfp::ChipModel;
use sid_file::SidFile;
use std::path::PathBuf;

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

    /// Maximum song playtime in seconds before advancing
    #[arg(long, default_value = "180")]
    playtime: u64,

    /// Audio resampling method: fast, interpolate, resample, resample-fast, two-pass
    #[arg(long, default_value = "two-pass", value_parser = parse_sampling_method)]
    sampling: SamplingMethod,

    /// Use EKV transistor model filter for more accurate 6581 emulation
    #[arg(long)]
    ekv: bool,
}

/// Parse sampling method from CLI string.
fn parse_sampling_method(s: &str) -> Result<SamplingMethod, String> {
    match s.to_lowercase().as_str() {
        "fast" => Ok(SamplingMethod::Fast),
        "interpolate" => Ok(SamplingMethod::Interpolate),
        "resample" => Ok(SamplingMethod::Resample),
        "resample-fast" => Ok(SamplingMethod::ResampleFast),
        "two-pass" | "twopass" => Ok(SamplingMethod::ResampleTwoPass),
        _ => Err(format!(
            "unknown sampling method '{s}', expected: fast, interpolate, resample, resample-fast, two-pass"
        )),
    }
}

fn default_playlist_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("crabsid")
        .join("playlist.m3u")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let playlist_path = args.playlist.clone().unwrap_or_else(default_playlist_path);
    let mut playlist = Playlist::load_or_create(&playlist_path)?;
    let mut playlist_modified = false;
    for file in &args.files {
        let absolute = file.canonicalize().unwrap_or_else(|_| file.clone());
        playlist_modified |= playlist.add(&absolute.to_string_lossy(), None);
    }

    // With no CLI files and an empty playlist we still need a SID to construct the player;
    // the TUI starts with HVSC browser focused so the user can pick one immediately.
    let (sid_file, initial_song) = if !args.files.is_empty() {
        let sid = SidFile::load(&args.files[0])?;
        let song = args.song.unwrap_or(sid.start_song);
        (sid, song)
    } else if !playlist.is_empty() {
        let entry = &playlist.entries[0];
        let sid = entry.load()?;
        let song = args.song.or(entry.subsong).unwrap_or(sid.start_song);
        (sid, song)
    } else {
        (SidFile::silent(), 1)
    };

    let chip_override = args.chip.map(|n| match n {
        8580 => ChipModel::Mos8580,
        _ => ChipModel::Mos6581,
    });

    // Probe the audio device first so the player's resampler runs at the device rate.
    let audio = AudioOutput::probe()?;

    let player = create_shared_player(
        &sid_file,
        initial_song,
        audio.sample_rate,
        chip_override,
        args.sampling,
    )
    .map_err(|e| format!("{e}"))?;

    if args.ekv && let Ok(mut p) = player.lock() {
        p.set_ekv_filter(true);
    }

    // Audio callback runs on cpal's internal audio thread.
    let player_audio = player.clone();
    let _stream = audio.start(move |data| {
        if let Ok(mut p) = player_audio.lock() {
            p.fill_buffer(data);
        }
    })?;

    if args.no_tui {
        run_simple(&sid_file, initial_song)?;
    } else {
        let mut user_config = Config::load();
        let focus_hvsc = args.files.is_empty() && playlist.is_empty();
        let tui_config = tui::TuiConfig {
            player,
            sid_file: &sid_file,
            song: initial_song,
            playlist,
            playlist_path,
            focus_hvsc,
            playlist_modified,
            hvsc_url: &args.hvsc_url,
            playtime_secs: args.playtime,
            color_scheme: user_config.color_scheme,
        };
        let final_color_scheme = tui::run_tui(tui_config)?;
        user_config.color_scheme = final_color_scheme;
        user_config.save();
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
