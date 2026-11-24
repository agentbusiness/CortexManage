//! Reading a unified diff: its files, whether a named line is in it, and the part of it that is
//! inside a scope. Used to check the model's answers against the facts.

use crate::policy;

/// One file's part of a diff.
pub struct Section<'a> {
    pub file: String,
    pub text: &'a str,
}

/// The diff cut into its files (`diff --git` blocks).
pub fn sections(diff: &str) -> Vec<Section<'_>> {
    let mut starts: Vec<usize> = Vec::new();
    let mut at = 0;
    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            starts.push(at);
        }
        at += line.len();
    }
    if starts.is_empty() && !diff.trim().is_empty() {
        starts.push(0);
    }
    let mut out = Vec::new();
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(diff.len());
        let text = &diff[*start..end];
        out.push(Section { file: file_of(text), text });
    }
    out
}

/// The file a block changes: its new name, or its old one when it was deleted.
fn file_of(block: &str) -> String {
    let mut old = String::new();
    for line in block.lines() {
        if let Some(name) = line.strip_prefix("+++ ") {
            let name = name.trim();
            if name != "/dev/null" {
                return name.strip_prefix("b/").unwrap_or(name).to_owned();
            }
        } else if let Some(name) = line.strip_prefix("--- ") {
            let name = name.trim();
            if name != "/dev/null" {
                old = name.strip_prefix("a/").unwrap_or(name).to_owned();
            }
        } else if line.starts_with("@@") {
            break;
        }
    }
    if old.is_empty()
        && let Some(header) = block.lines().next().and_then(|line| line.strip_prefix("diff --git "))
        && let Some((_, new)) = header.rsplit_once(" b/")
    {
        return new.trim().to_owned();
    }
    old
}

/// `@@ -a,b +c,d @@` -> `((a, b), (c, d))`.
fn hunk(line: &str) -> Option<((u32, u32), (u32, u32))> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let range = |text: &str| -> Option<(u32, u32)> {
        match text.split_once(',') {
            Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
            None => Some((text.parse().ok()?, 1)),
        }
    };
    Some((range(old)?, range(new)?))
}

/// Whether `file` is in the diff and `line` is one of its added lines (numbered in the new file).
/// A file the change only removes from has no added line: there, a line inside one of its hunks
/// (in the new file or the old) is the removal named.
pub fn has_line(diff: &str, file: &str, line: u32) -> bool {
    let wanted = file.trim().trim_start_matches("./");
    if wanted.is_empty() || line == 0 {
        return false;
    }
    sections(diff).iter().filter(|section| section.file == wanted).any(|section| {
        let added = added_lines(section.text);
        if !added.is_empty() {
            return added.contains(&line);
        }
        section.text.lines().filter_map(hunk).any(|((old, old_count), (new, new_count))| {
            let within = |start: u32, count: u32| line >= start && line < start + count.max(1);
            within(new, new_count) || within(old, old_count)
        })
    })
}

/// The new file's numbers of the lines a file's part of a diff adds.
fn added_lines(text: &str) -> Vec<u32> {
    let mut added = Vec::new();
    // Lines still to come in the open hunk (old side, new side), and the next new line's number.
    let (mut old_left, mut new_left, mut next) = (0u32, 0u32, 0u32);
    for line in text.lines() {
        if old_left == 0 && new_left == 0 {
            if let Some((old, new)) = hunk(line) {
                (old_left, new_left, next) = (old.1, new.1, new.0);
            }
        } else if line.starts_with('+') {
            added.push(next);
            next += 1;
            new_left = new_left.saturating_sub(1);
        } else if line.starts_with('-') {
            old_left = old_left.saturating_sub(1);
        } else if !line.starts_with('\\') {
            next += 1;
            old_left = old_left.saturating_sub(1);
            new_left = new_left.saturating_sub(1);
        }
    }
    added
}

/// The files a diff changes.
pub fn files(diff: &str) -> Vec<String> {
    let mut out: Vec<String> = sections(diff).into_iter().map(|section| section.file).filter(|file| !file.is_empty()).collect();
    out.dedup();
    out
}

/// The part of a diff inside a scope (a point's `touches`).
pub fn only(diff: &str, touches: &[String]) -> String {
    sections(diff).into_iter().filter(|section| touches.iter().any(|touch| policy::inside(touch, &section.file))).map(|section| section.text).collect()
}

/// The last `count` lines of a text that are not blank.
pub fn tail(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

/// Every added line of a diff with its file (without the `+`).
pub fn added(diff: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for section in sections(diff) {
        let mut in_hunk = false;
        for line in section.text.lines() {
            if line.starts_with("@@") {
                in_hunk = true;
            } else if in_hunk && let Some(text) = line.strip_prefix('+') {
                out.push((section.file.clone(), text.to_owned()));
            }
        }
    }
    out
}
