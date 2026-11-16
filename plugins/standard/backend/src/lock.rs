//! The lock and the check against it: hashing a member's copies (with the member's own values
//! replaced where a file carries them), comparing every member's copies with the lock, and the
//! lock as the text of `standard.lock`.
//!
//! Everything here reads files with plain `std::fs` and is run in `spawn_blocking` by the service.

use crate::parts::{Own, Parts, is_text};
use cc_protocol::contracts::standard::{Cell, Lock, LockFile, Member, Report};
use cc_protocol::sha256;
use std::collections::BTreeSet;
use std::path::Path;

/// How many differing files a cell lists (the contract's limit).
const CELL_FILES: usize = 20;
/// What a member's priority becomes in a file hashed with its own values replaced.
const PRIORITY: &str = "<<priority>>";

/// What one of a member's names becomes. Names of different shapes get different placeholders,
/// so `cortexterm` where the standard has `CortexTerm` is still a difference.
fn placeholder(name: &str) -> String {
    let letters = |test: fn(char) -> bool| name.chars().any(test);
    let case = match (letters(char::is_uppercase), letters(char::is_lowercase)) {
        (true, true) => "mixed",
        (true, false) => "upper",
        _ => "lower",
    };
    let marks: BTreeSet<char> = name.chars().filter(|c| !c.is_alphanumeric()).collect();
    format!("<<name:{case}{}>>", marks.into_iter().collect::<String>())
}

/// `text` with the member's own values replaced: its names wherever they stand, its priority
/// where it directly follows one of the texts `own` lists.
fn without_own(text: &str, member: &Member, own: &Own) -> String {
    // The longest name first, so a name that holds another is taken whole.
    let mut names: Vec<(&str, String)> = member.names.iter().filter(|name| !name.is_empty()).map(|name| (name.as_str(), placeholder(name))).collect();
    names.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(b.0)));
    let priority = member.priority.to_string();
    let before: Vec<&str> = own.priority_after.iter().map(String::as_str).filter(|text| !text.is_empty()).collect();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'text: while !rest.is_empty() {
        for (name, placeholder) in &names {
            if let Some(after) = rest.strip_prefix(name) {
                out.push_str(placeholder);
                rest = after;
                continue 'text;
            }
        }
        for prefix in &before {
            let Some(after) = rest.strip_prefix(prefix) else { continue };
            // The whole number, not the start of a longer one.
            if let Some(tail) = after.strip_prefix(priority.as_str())
                && !tail.starts_with(|c: char| c.is_ascii_digit())
            {
                out.push_str(prefix);
                out.push_str(PRIORITY);
                rest = tail;
                continue 'text;
            }
        }
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            out.push(c);
        }
        rest = chars.as_str();
    }
    out
}

/// A file's hash for the lock: of its bytes, or, when it carries the member's own values, of its
/// text with those replaced (`names`).
pub fn hash(bytes: &[u8], member: &Member, own: &Own) -> (String, bool) {
    if is_text(bytes)
        && let Ok(text) = std::str::from_utf8(bytes)
    {
        let replaced = without_own(text, member, own);
        if replaced != text {
            return (sha256::hex(replaced.as_bytes()), true);
        }
    }
    (sha256::hex(bytes), false)
}

/// The folder of a part in a member's checkout (none: the member has no folder for it).
fn folder<'a>(member: &'a Member, part: &str) -> Option<&'a str> {
    member.folders.get(part).map(String::as_str).filter(|folder| !folder.trim().is_empty())
}

/// Every standard file of `member`'s checkout, hashed, in the order of the parts.
pub fn files_of(member: &Member, parts: &Parts) -> Result<Vec<LockFile>, String> {
    let root = Path::new(&member.root);
    if member.root.trim().is_empty() || !root.is_dir() {
        return Err(format!("{} has no folder on this computer; set it in the Standard view first", member.name));
    }
    let mut files = Vec::new();
    for part in &parts.parts {
        if member.without.contains(&part.id) {
            continue;
        }
        let Some(relative) = folder(member, &part.id) else { continue };
        let dir = root.join(relative);
        for path in parts.files_in(part, &dir) {
            let bytes = std::fs::read(dir.join(&path)).map_err(|error| format!("cannot read {}: {error}", dir.join(&path).display()))?;
            let (sha256, names) = hash(&bytes, member, &parts.own);
            files.push(LockFile { part: part.id.clone(), path, sha256, names });
        }
    }
    if files.is_empty() {
        return Err(format!("no standard file was found in {}", root.display()));
    }
    Ok(files)
}

/// The parts the lock has files of, in the order of `parts.toml`.
pub fn lock_parts(lock: &Lock, parts: &Parts) -> Vec<String> {
    let mut ids: Vec<String> = lock.files.iter().map(|file| file.part.clone()).collect::<BTreeSet<_>>().into_iter().collect();
    ids.sort_by(|a, b| parts.order(a).cmp(&parts.order(b)).then(a.cmp(b)));
    ids
}

fn cell(member: &Member, part: &str, state: &str, mut files: Vec<String>) -> Cell {
    let count = files.len() as u32;
    files.truncate(CELL_FILES);
    Cell { member: member.id.clone(), part: part.to_owned(), state: state.to_owned(), files, count }
}

/// One part of one member against the lock.
fn compare(member: &Member, part: &str, lock: &Lock, parts: &Parts) -> Cell {
    if member.deferred {
        return cell(member, part, "deferred", Vec::new());
    }
    if member.without.iter().any(|without| without == part) {
        return cell(member, part, "without", Vec::new());
    }
    let root = Path::new(&member.root);
    if member.root.trim().is_empty() || !root.is_dir() {
        return cell(member, part, "not_here", Vec::new());
    }
    let locked: Vec<&LockFile> = lock.files.iter().filter(|file| file.part == part).collect();
    let all = || locked.iter().map(|file| file.path.clone()).collect::<Vec<_>>();
    // No folder named for the part and not declared out: the member lacks it.
    let Some(relative) = folder(member, part) else { return cell(member, part, "missing", all()) };
    let dir = root.join(relative);
    if !dir.is_dir() {
        return cell(member, part, "missing", all());
    }
    let mut differing = Vec::new();
    let (mut absent, mut by_names) = (0usize, false);
    for file in &locked {
        let Ok(bytes) = std::fs::read(dir.join(&file.path)) else {
            absent += 1;
            differing.push(file.path.clone());
            continue;
        };
        if !file.names && sha256::hex(&bytes) == file.sha256 {
            continue;
        }
        let (sha256, _) = hash(&bytes, member, &parts.own);
        if sha256 == file.sha256 {
            by_names = true;
        } else {
            differing.push(file.path.clone());
        }
    }
    // A file of the part the lock does not have is a difference too (a kit is every file in it).
    if let Some(known) = parts.part(part) {
        let in_lock: BTreeSet<&str> = locked.iter().map(|file| file.path.as_str()).collect();
        differing.extend(parts.files_in(known, &dir).into_iter().filter(|path| !in_lock.contains(path.as_str())));
    }
    if !locked.is_empty() && absent == locked.len() && differing.len() == absent {
        return cell(member, part, "missing", differing);
    }
    let state = if !differing.is_empty() {
        "differs"
    } else if by_names {
        "names"
    } else {
        "same"
    };
    cell(member, part, state, differing)
}

/// Every member's copies against the lock. Before the first lock there is nothing to compare:
/// the report has no parts and no cells.
pub fn check(members: &[Member], lock: &Lock, parts: &Parts, now_ms: u64) -> Report {
    let ids = lock_parts(lock, parts);
    let mut cells = Vec::new();
    for member in members {
        for part in &ids {
            cells.push(compare(member, part, lock, parts));
        }
    }
    let drift = drift_of(members, &cells);
    Report { version: lock.version.clone(), parts: ids, members: members.to_vec(), cells, drift, checked_ms: now_ms }
}

/// The members with at least one part that differs or is missing, in the members' order.
pub fn drift_of(members: &[Member], cells: &[Cell]) -> Vec<String> {
    members.iter().filter(|member| cells.iter().any(|cell| cell.member == member.id && matches!(cell.state.as_str(), "differs" | "missing"))).map(|member| member.id.clone()).collect()
}

/// The version after `last`: its minor plus one; `1.00` when there is none yet.
pub fn next_version(last: &str) -> String {
    match parse_version(last) {
        Some((major, minor)) if minor >= 99 => format!("{}.00", major + 1),
        Some((major, minor)) => format!("{major}.{:02}", minor + 1),
        None => "1.00".to_owned(),
    }
}

/// `1.08` as (1, 8).
pub fn parse_version(version: &str) -> Option<(u32, u32)> {
    let (major, minor) = version.trim().split_once('.')?;
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    if !digits(major) || !digits(minor) {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// A text as a TOML string.
fn quoted(text: &str) -> String {
    toml::Value::String(text.to_owned()).to_string()
}

/// A TOML key: bare when it may be, quoted otherwise.
fn key(text: &str) -> String {
    let bare = !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if bare { text.to_owned() } else { quoted(text) }
}

fn list(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|item| quoted(item)).collect::<Vec<_>>().join(", "))
}

/// The lock as the text of `standard.lock`: the version, the members (priority, names, folders,
/// what each is without), one entry per file. Nothing of this computer is in it (no checkout
/// folder, no time), so the same lock prints the same text everywhere.
pub fn print(lock: &Lock, members: &[Member]) -> String {
    let mut out = String::new();
    out.push_str("# The mesh standard. Written by CortexManage; a change by hand is a drift.\n");
    out.push_str(&format!("standard = {}\n", quoted(&lock.version)));
    if !lock.from.is_empty() {
        out.push_str(&format!("from = {}\n", quoted(&lock.from)));
    }
    out.push_str("\n[members]\n");
    let width = members.iter().map(|member| key(&member.id).len()).max().unwrap_or(0);
    for member in members {
        let mut fields = vec![format!("name = {}", quoted(&member.name)), format!("priority = {}", member.priority)];
        if !member.names.is_empty() {
            fields.push(format!("names = {}", list(&member.names)));
        }
        // Each part's folder under the part's own id, as docs/specs/04-STANDARD.md §2 has it.
        fields.extend(member.folders.iter().filter(|(_, folder)| !folder.trim().is_empty()).map(|(part, folder)| format!("{} = {}", key(part), quoted(folder))));
        if !member.without.is_empty() {
            fields.push(format!("without = {}", list(&member.without)));
        }
        if member.deferred {
            fields.push("deferred = true".to_owned());
        }
        out.push_str(&format!("{:width$} = {{ {} }}\n", key(&member.id), fields.join(", ")));
    }
    for file in &lock.files {
        out.push_str(&format!("\n[[file]]\npart = {}\npath = {}\nsha256 = {}\n", quoted(&file.part), quoted(&file.path), quoted(&file.sha256)));
        if file.names {
            out.push_str("names = true\n");
        }
    }
    out
}

/// The files of a printed lock, read back (the probe checks that a lock survives its own text).
pub fn files_of_text(text: &str) -> Result<(String, Vec<LockFile>), String> {
    #[derive(serde::Deserialize)]
    struct Printed {
        standard: String,
        #[serde(default)]
        file: Vec<LockFile>,
    }
    let printed: Printed = toml::from_str(text).map_err(|error| error.to_string())?;
    Ok((printed.standard, printed.file))
}
