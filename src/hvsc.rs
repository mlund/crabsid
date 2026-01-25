// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! HVSC (High Voltage SID Collection) browser with STIL metadata support.

use crate::sid_file::SidFile;
use std::collections::HashMap;
use std::io::{self, Read};

const HVSC_BASE_URL: &str = "https://hvsc.brona.dk/HVSC/C64Music";
const STIL_URL: &str = "https://hvsc.brona.dk/HVSC/C64Music/DOCUMENTS/STIL.txt";

/// Metadata for a SID file from STIL.
#[derive(Debug, Clone, Default)]
pub struct StilEntry {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub comment: Option<String>,
}

/// Parsed STIL database mapping paths to metadata.
#[derive(Debug, Default)]
pub struct StilDatabase {
    entries: HashMap<String, StilEntry>,
}

impl StilDatabase {
    /// Fetches and parses the STIL file from HVSC.
    pub fn fetch() -> io::Result<Self> {
        let response = ureq::get(STIL_URL)
            .call()
            .map_err(|e: ureq::Error| io::Error::other(e.to_string()))?;

        // STIL uses Latin-1 encoding, read as bytes and convert
        let mut bytes = Vec::new();
        response.into_body().into_reader().read_to_end(&mut bytes)?;
        let content: String = bytes.iter().map(|&b| b as char).collect();

        Ok(Self::parse(&content))
    }

    fn parse(content: &str) -> Self {
        let mut entries = HashMap::new();
        let mut current_path: Option<String> = None;
        let mut current_entry = StilEntry::default();

        for line in content.lines() {
            // STIL format: path line starts new entry, field lines are indented
            if line.starts_with('/') && line.ends_with(".sid") {
                // Save previous entry (even without metadata, for search)
                if let Some(path) = current_path.take() {
                    entries.insert(path, current_entry);
                }
                current_path = Some(line.to_string());
                current_entry = StilEntry::default();
                continue;
            }

            // Parse field lines
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("TITLE:") {
                current_entry.title = Some(rest.trim().to_string());
            } else if let Some(rest) = trimmed.strip_prefix("ARTIST:") {
                current_entry.artist = Some(rest.trim().to_string());
            } else if let Some(rest) = trimmed.strip_prefix("COMMENT:") {
                current_entry.comment = Some(rest.trim().to_string());
            }
        }

        // Don't forget last entry
        if let Some(path) = current_path {
            entries.insert(path, current_entry);
        }

        Self { entries }
    }

    /// Returns the number of entries in the database.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the database is empty.
    #[allow(dead_code)] // Required by clippy for len() method
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Looks up STIL info for a given HVSC path.
    pub fn get(&self, path: &str) -> Option<&StilEntry> {
        self.entries.get(path)
    }

    /// Searches paths, titles, and artists for entries containing the query (case-insensitive).
    pub fn search(&self, query: &str) -> Vec<&str> {
        let query_lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|(path, entry)| {
                path.to_lowercase().contains(&query_lower)
                    || entry
                        .title
                        .as_ref()
                        .is_some_and(|t| t.to_lowercase().contains(&query_lower))
                    || entry
                        .artist
                        .as_ref()
                        .is_some_and(|a| a.to_lowercase().contains(&query_lower))
            })
            .map(|(path, _)| path.as_str())
            .collect()
    }
}

/// An entry in the HVSC browser (directory or file).
#[derive(Debug, Clone)]
pub struct HvscEntry {
    /// Display name
    pub name: String,
    /// Full HVSC path (e.g., "/MUSICIANS/H/Hubbard_Rob/")
    pub path: String,
    /// True if this is a directory
    pub is_dir: bool,
}

impl HvscEntry {
    /// Returns the full URL for this entry.
    pub fn url(&self) -> String {
        format!("{HVSC_BASE_URL}{}", self.path)
    }

    /// Loads this entry as a SID file (only valid for files).
    pub fn load(&self) -> io::Result<SidFile> {
        if self.is_dir {
            return Err(io::Error::other("Cannot load directory as SID file"));
        }
        let url = self.url();
        let response = ureq::get(&url)
            .call()
            .map_err(|e| io::Error::other(e.to_string()))?;

        let mut bytes = Vec::new();
        response.into_body().into_reader().read_to_end(&mut bytes)?;
        SidFile::parse(&bytes)
    }
}

/// HVSC directory browser state.
pub struct HvscBrowser {
    /// Current directory path
    pub current_path: String,
    /// Entries in current directory
    pub entries: Vec<HvscEntry>,
    /// Selected index
    pub selected: usize,
    /// STIL database for metadata
    pub stil: Option<StilDatabase>,
    /// STIL loading error (persists across navigation)
    pub stil_error: Option<String>,
    /// Loading state
    pub loading: bool,
    /// Error message if any
    pub error: Option<String>,
}

impl HvscBrowser {
    /// Creates a new browser at the root level.
    pub fn new() -> Self {
        let entries = vec![
            HvscEntry {
                name: "MUSICIANS".to_string(),
                path: "/MUSICIANS/".to_string(),
                is_dir: true,
            },
            HvscEntry {
                name: "GAMES".to_string(),
                path: "/GAMES/".to_string(),
                is_dir: true,
            },
            HvscEntry {
                name: "DEMOS".to_string(),
                path: "/DEMOS/".to_string(),
                is_dir: true,
            },
        ];

        Self {
            current_path: "/".to_string(),
            entries,
            selected: 0,
            stil: None,
            stil_error: None,
            loading: false,
            error: None,
        }
    }

    /// Fetches the STIL database.
    pub fn load_stil(&mut self) {
        match StilDatabase::fetch() {
            Ok(db) => self.stil = Some(db),
            Err(e) => self.stil_error = Some(e.to_string()),
        }
    }

    /// Returns STIL info for the selected entry if available.
    #[allow(dead_code)]
    pub fn selected_stil_info(&self) -> Option<&StilEntry> {
        let entry = self.entries.get(self.selected)?;
        if entry.is_dir {
            return None;
        }
        self.stil.as_ref()?.get(&entry.path)
    }

    /// Navigate into the selected directory or return the selected file.
    pub fn enter(&mut self) -> Option<HvscEntry> {
        let entry = self.entries.get(self.selected)?.clone();

        if entry.is_dir {
            self.navigate_to(&entry.path);
            None
        } else {
            Some(entry)
        }
    }

    /// Go up one directory level.
    pub fn go_up(&mut self) {
        if self.current_path == "/" {
            return;
        }

        // Remove trailing slash, find parent
        let path = self.current_path.trim_end_matches('/');
        if let Some(pos) = path.rfind('/') {
            let parent = if pos == 0 {
                "/".to_string()
            } else {
                format!("{}/", &path[..pos])
            };
            self.navigate_to(&parent);
        }
    }

    /// Navigate to a specific path.
    pub fn navigate_to(&mut self, path: &str) {
        if path == "/" {
            // STIL is expensive to fetch, preserve it across navigation
            let stil = self.stil.take();
            let stil_error = self.stil_error.take();
            *self = Self::new();
            self.stil = stil;
            self.stil_error = stil_error;
            return;
        }

        self.loading = true;
        self.error = None;

        match fetch_directory(path) {
            Ok(entries) => {
                self.current_path = path.to_string();
                self.entries = entries;
                self.selected = 0;
            }
            Err(e) => {
                self.error = Some(e.to_string());
            }
        }

        self.loading = false;
    }

    pub fn select_next(&mut self) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + 1).min(self.entries.len() - 1);
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Returns the currently selected entry.
    #[allow(dead_code)]
    pub fn selected_entry(&self) -> Option<&HvscEntry> {
        self.entries.get(self.selected)
    }
}

/// Fetches and parses a directory listing from HVSC.
fn fetch_directory(path: &str) -> io::Result<Vec<HvscEntry>> {
    let url = format!("{HVSC_BASE_URL}{path}");
    let response = ureq::get(&url)
        .call()
        .map_err(|e| io::Error::other(e.to_string()))?;

    let mut html = String::new();
    response
        .into_body()
        .into_reader()
        .read_to_string(&mut html)?;

    Ok(parse_directory_listing(&html, path))
}

/// Extracts href value from an HTML line, filtering navigation/special links.
fn extract_href(line: &str) -> Option<&str> {
    if line.contains("Parent Directory") {
        return None;
    }

    let start = line.find("href=\"")? + 6;
    let rest = &line[start..];
    let end = rest.find('"')?;
    let href = &rest[..end];

    // Apache listings include sort links and parent refs we don't want
    let dominated_by_nav =
        href.starts_with('?') || href.starts_with('/') || href.starts_with("http") || href == "../";

    if dominated_by_nav { None } else { Some(href) }
}

/// Parses an Apache-style directory listing HTML.
fn parse_directory_listing(html: &str, base_path: &str) -> Vec<HvscEntry> {
    let mut entries: Vec<HvscEntry> = html
        .lines()
        .filter_map(|line| {
            let href = extract_href(line)?;
            let is_dir = href.ends_with('/');
            let name = href.trim_end_matches('/').to_string();

            // HVSC contains non-SID files (txt, etc) we skip
            if !is_dir && !name.to_lowercase().ends_with(".sid") {
                return None;
            }

            Some(HvscEntry {
                name,
                path: format!("{base_path}{href}"),
                is_dir,
            })
        })
        .collect();

    // Directories first for easier navigation
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! stil_tests {
        ($($name:ident: $path:expr => ($title:expr, $artist:expr),)*) => {
            const STIL_CONTENT: &str = r#"
/MUSICIANS/H/Hubbard_Rob/Commando.sid
  TITLE: Commando
 ARTIST: Rob Hubbard

/MUSICIANS/H/Hubbard_Rob/Delta.sid
  TITLE: Delta
"#;

            $(
                #[test]
                fn $name() {
                    let db = StilDatabase::parse(STIL_CONTENT);
                    let entry = db.get($path).unwrap();
                    assert_eq!(entry.title.as_deref(), $title);
                    assert_eq!(entry.artist.as_deref(), $artist);
                }
            )*
        };
    }

    stil_tests! {
        stil_with_artist: "/MUSICIANS/H/Hubbard_Rob/Commando.sid" => (Some("Commando"), Some("Rob Hubbard")),
        stil_title_only: "/MUSICIANS/H/Hubbard_Rob/Delta.sid" => (Some("Delta"), None),
    }

    macro_rules! href_tests {
        ($($name:ident: $line:expr => $expected:expr,)*) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!(extract_href($line), $expected);
                }
            )*
        };
    }

    href_tests! {
        href_directory: r#"<a href="A/">A/</a>"# => Some("A/"),
        href_file: r#"<a href="Commando.sid">Commando.sid</a>"# => Some("Commando.sid"),
        href_skip_sort: r#"<a href="?C=N;O=D">Name</a>"# => None,
        href_skip_parent: r#"<a href="../">Parent Directory</a>"# => None,
    }

    #[test]
    fn directory_listing_filters_non_sid() {
        let html = r#"
<a href="0-9/">0-9/</a>
<a href="tune.sid">tune.sid</a>
<a href="readme.txt">readme.txt</a>
"#;
        let entries = parse_directory_listing(html, "/TEST/");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "0-9");
        assert_eq!(entries[1].name, "tune.sid");
    }
}
