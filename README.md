# Crabsid

A command-line SID music player for .sid files from the [High Voltage SID Collection](https://hvsc.c64.org/).

Emulates the MOS 6502 CPU and MOS 6581/8580 SID chip to play Commodore 64 music.

## Features

- Plays PSID and RSID format files
- Supports both MOS 6581 and MOS 8580 SID chip emulation
- PAL and NTSC timing detection from file headers
- Multi-song files with prev/next navigation
- Terminal UI with:
  - VU meters showing per-voice envelope levels
  - Oscilloscope displaying envelope waveforms for all three voices
  - Real-time chip model switching
- Headless mode for background playback

## Installation

Requires Rust and ALSA development libraries (Linux):

```bash
# Debian/Ubuntu
sudo apt install pkg-config libasound2-dev

# Build and install
cargo install --path .
```

## Usage

```bash
crabsid music.sid                # Play default song
crabsid music.sid --song 3       # Play song 3
crabsid music.sid --chip 8580    # Force 8580 chip emulation
crabsid music.sid --no-tui       # Headless mode
```

## Keyboard Controls

| Key | Action |
|-----|--------|
| `Space` | Pause/Resume |
| `Left` / `p` | Previous song |
| `Right` / `n` | Next song |
| `s` | Switch SID chip model |
| `q` / `Esc` | Quit |

## Options

| Option | Description |
|--------|-------------|
| `-s, --song <N>` | Song number to play (default: from file) |
| `-c, --chip <MODEL>` | SID chip: 6581 or 8580 (default: from file) |
| `--no-tui` | Disable TUI, simple text output |

## Dependencies

- [mos6502](https://crates.io/crates/mos6502) - 6502 CPU emulation
- [resid-rs](https://crates.io/crates/resid-rs) - SID chip emulation (reSID port)
- [tinyaudio](https://crates.io/crates/tinyaudio) - Cross-platform audio output
- [ratatui](https://crates.io/crates/ratatui) - Terminal UI
- [clap](https://crates.io/crates/clap) - Command-line parsing

## License

MIT
