# rustsid

A command-line SID music player for .sid files from e.g the [High Voltage SID Collection](https://hvsc.c64.org/).

Emulates the MOS 6502 CPU and MOS 6581/8580 SID chip to play Commodore 64 music.

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
rustsid music.sid                # Play default song
rustsid music.sid --song 3       # Play song 3
rustsid music.sid --chip 8580    # Force 8580 chip emulation
rustsid music.sid -s 2 -c 6581   # Song 2 with 6581 chip
```

## Options

| Option | Description |
|--------|-------------|
| `-s, --song <N>` | Song number to play (default: from file) |
| `-c, --chip <MODEL>` | SID chip: 6581 or 8580 (default: from file) |

## Dependencies

- [mos6502](https://crates.io/crates/mos6502) - 6502 CPU emulation
- [resid-rs](https://crates.io/crates/resid-rs) - SID chip emulation
- [tinyaudio](https://crates.io/crates/tinyaudio) - Audio output

## License

MIT
