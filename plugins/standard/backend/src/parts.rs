//! Which files of each part are the standard (`data/parts.toml`): the shipped copy compiled in,
//! the live copy (`backend/data/parts.toml` in the plugin's live folder) read on every use, so an
//! edit there takes effect at once. A live file that does not parse, or lists no part, is ignored.
//!
//! The file reads here are plain `std::fs`; the callers run them in `spawn_blocking`.

use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const SHIPPED: &str = include_str!("../data/parts.toml");

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Skip {
    /// Folder names never looked into.
    #[serde(default)]
    pub folders: Vec<String>,
    /// File names never counted.
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Own {
    /// The texts a member's priority directly follows where it is the member's own value.
    #[serde(default)]
    pub priority_after: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Part {
    pub id: String,
    /// Paths relative to the member's folder for the part, with `*`, `?` and `**`.
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Parts {
    #[serde(default)]
    pub skip: Skip,
    #[serde(default)]
    pub own: Own,
    #[serde(default, rename = "part")]
    pub parts: Vec<Part>,
}

fn parse(text: &str) -> Option<Parts> {
    let parts: Parts = toml::from_str(text).ok()?;
    let usable = !parts.parts.is_empty() && parts.parts.iter().all(|part| !part.id.trim().is_empty());
    usable.then_some(parts)
}

/// Where the parts are read from.
#[derive(Debug, Clone)]
pub struct Source {
    live: Option<PathBuf>,
}

impl Source {
    pub fn new(live_dir: Option<PathBuf>) -> Self {
        Self { live: live_dir.map(|dir| dir.join("backend").join("data").join("parts.toml")) }
    }

    /// The parts as they are now. Reads a file: call it off the async threads.
    pub fn read(&self) -> Parts {
        self.live
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| parse(&text))
            .or_else(|| parse(SHIPPED))
            .unwrap_or_default()
    }
}

impl Parts {
    pub fn ids(&self) -> Vec<String> {
        self.parts.iter().map(|part| part.id.clone()).collect()
    }

    pub fn part(&self, id: &str) -> Option<&Part> {
        self.parts.iter().find(|part| part.id == id)
    }

    /// Where a part stands in the order of the file (an unknown part comes last).
    pub fn order(&self, id: &str) -> usize {
        self.parts.iter().position(|part| part.id == id).unwrap_or(self.parts.len())
    }

    /// Whether `path` (relative to the part's folder, `/` between names) is a file of the part.
    pub fn counts(&self, part: &Part, path: &str) -> bool {
        let names: Vec<&str> = path.split('/').filter(|name| !name.is_empty()).collect();
        let Some((file, folders)) = names.split_last() else { return false };
        if self.skip.files.iter().any(|skipped| skipped == file) || folders.iter().any(|folder| self.skip.folders.iter().any(|skipped| skipped == folder)) {
            return false;
        }
        part.files.iter().any(|pattern| {
            let pattern: Vec<&str> = pattern.split('/').filter(|name| !name.is_empty()).collect();
            matches(&pattern, &names)
        })
    }

    /// Every text file of `part` under `folder`, as paths relative to it, sorted. A folder that is
    /// not there has none.
    pub fn files_in(&self, part: &Part, folder: &Path) -> Vec<String> {
        let mut found = BTreeSet::new();
        // A part that names its files one by one is looked up directly; only a pattern walks.
        let mut walk = false;
        for pattern in &part.files {
            if pattern.contains(['*', '?']) {
                walk = true;
            } else if self.counts(part, pattern) && folder.join(pattern).is_file() {
                found.insert(pattern.trim_matches('/').to_owned());
            }
        }
        if walk {
            let mut pending = vec![(folder.to_path_buf(), String::new())];
            while let Some((dir, prefix)) = pending.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let relative = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                    // A link is not followed: the standard is the files that are in the folder.
                    let Ok(kind) = entry.file_type() else { continue };
                    if kind.is_dir() {
                        if !self.skip.folders.contains(&name) {
                            pending.push((entry.path(), relative));
                        }
                    } else if kind.is_file() && self.counts(part, &relative) {
                        found.insert(relative);
                    }
                }
            }
        }
        found.into_iter().filter(|relative| std::fs::read(folder.join(relative)).is_ok_and(|bytes| is_text(&bytes))).collect()
    }
}

/// Text: valid UTF-8 without a NUL byte. Anything else (a font, an image) is not a standard file.
pub fn is_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// A pattern against a path, both split into names. `**` stands for any number of names.
fn matches(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skipped| matches(rest, &path[skipped..])),
        Some((first, rest)) => path.split_first().is_some_and(|(name, more)| name_matches(first.as_bytes(), name.as_bytes()) && matches(rest, more)),
    }
}

/// One name against one name of a pattern: `*` is any run of bytes, `?` one byte.
fn name_matches(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some((b'*', rest)) => (0..=name.len()).any(|taken| name_matches(rest, &name[taken..])),
        Some((b'?', rest)) => name.split_first().is_some_and(|(_, more)| name_matches(rest, more)),
        Some((byte, rest)) => name.split_first().is_some_and(|(first, more)| first == byte && name_matches(rest, more)),
    }
}
