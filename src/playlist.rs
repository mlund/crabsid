// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

use crate::sid_file::SidFile;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

/// A single entry in a playlist, representing a SID tune source.
#[derive(Debug, Clone)]
pub struct PlaylistEntry {
    /// Original source (file path or URL)
    pub source: String,
    /// Display name (filename without path)
    pub display_name: String,
    /// Optional subsong override (1-indexed)
    pub subsong: Option<u16>,
}

impl PlaylistEntry {
    /// Creates a new entry, extracting display name and optional subsong.
    fn new(source: &str) -> Option<Self> {
        let trimmed = source.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }

        let (path_part, subsong) = parse_subsong(trimmed);
        let display_name = extract_filename(path_part);

        Some(Self {
            source: path_part.to_string(),
            display_name,
            subsong,
        })
    }

    /// Returns true if this entry is a URL (http/https).
    pub fn is_url(&self) -> bool {
        self.source.starts_with("http://") || self.source.starts_with("https://")
    }

    /// Loads the SID file from this entry's source.
    pub fn load(&self) -> io::Result<SidFile> {
        if self.is_url() {
            load_from_url(&self.source)
        } else {
            SidFile::load(&self.source)
        }
    }
}

/// Parses optional @N subsong suffix from a path.
fn parse_subsong(s: &str) -> (&str, Option<u16>) {
    if let Some(at_pos) = s.rfind('@') {
        let suffix = &s[at_pos + 1..];
        if let Ok(num) = suffix.parse::<u16>() {
            return (&s[..at_pos], Some(num));
        }
    }
    (s, None)
}

/// Extracts filename from path or URL.
fn extract_filename(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_string()
}

/// Fetches and parses a SID file from a URL.
fn load_from_url(url: &str) -> io::Result<SidFile> {
    let response = ureq::get(url)
        .call()
        .map_err(|e| io::Error::other(e.to_string()))?;

    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .read_to_end(&mut bytes)?;

    SidFile::parse(&bytes)
}

/// A playlist of SID tunes loaded from an m3u file.
#[derive(Debug, Clone)]
pub struct Playlist {
    pub entries: Vec<PlaylistEntry>,
}

impl Playlist {
    /// Loads a playlist from an m3u file.
    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let content = fs::read_to_string(&path)?;
        let base_dir = path.as_ref().parent();

        let entries: Vec<PlaylistEntry> = content
            .lines()
            .filter_map(|line| {
                let mut entry = PlaylistEntry::new(line)?;
                // Resolve relative paths against playlist directory
                if !entry.is_url()
                    && !Path::new(&entry.source).is_absolute()
                    && let Some(base) = base_dir
                {
                    entry.source = base.join(&entry.source).to_string_lossy().to_string();
                }
                Some(entry)
            })
            .collect();

        Ok(Self { entries })
    }

    /// Returns true if the playlist has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! subsong_tests {
        ($($name:ident: $input:expr => ($path:expr, $subsong:expr),)*) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!(parse_subsong($input), ($path, $subsong));
                }
            )*
        };
    }

    subsong_tests! {
        no_subsong: "file.sid" => ("file.sid", None),
        with_subsong: "file.sid@3" => ("file.sid", Some(3)),
        url_with_subsong: "https://example.com/tune.sid@2" => ("https://example.com/tune.sid", Some(2)),
        invalid_subsong: "file.sid@abc" => ("file.sid@abc", None),
    }

    macro_rules! filename_tests {
        ($($name:ident: $input:expr => $expected:expr,)*) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!(extract_filename($input), $expected);
                }
            )*
        };
    }

    filename_tests! {
        simple_file: "tune.sid" => "tune.sid",
        unix_path: "/path/to/tune.sid" => "tune.sid",
        windows_path: "C:\\Music\\tune.sid" => "tune.sid",
        url_path: "https://example.com/music/tune.sid" => "tune.sid",
    }
}
