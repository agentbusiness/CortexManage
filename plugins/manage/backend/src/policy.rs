//! What the repository's policy decides, in plain code: how long a package waits for similar
//! work to join it, and path matching for a plan's scope. The tier is the model's; no folder
//! decides it.

use cc_protocol::contracts::manage::{Policy, Tier};

/// Whether `path` matches `glob`. `**` stands for any number of folders, `*` and `?` for
/// characters inside one name. A folder matches the glob of everything below it
/// (`protocol` matches `protocol/**`).
pub fn glob_match(glob: &str, path: &str) -> bool {
    segments(&parts(glob), &parts(path))
}

fn parts(path: &str) -> Vec<&str> {
    path.split('/').filter(|part| !part.is_empty() && *part != ".").collect()
}

fn segments(glob: &[&str], path: &[&str]) -> bool {
    match glob.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| segments(rest, &path[skip..])),
        Some((name, rest)) => match path.split_first() {
            Some((first, tail)) => name_match(name.as_bytes(), first.as_bytes()) && segments(rest, tail),
            None => false,
        },
    }
}

fn name_match(glob: &[u8], name: &[u8]) -> bool {
    match glob.split_first() {
        None => name.is_empty(),
        Some((b'*', rest)) => (0..=name.len()).any(|skip| name_match(rest, &name[skip..])),
        Some((b'?', rest)) => !name.is_empty() && name_match(rest, &name[1..]),
        Some((byte, rest)) => name.first() == Some(byte) && name_match(rest, &name[1..]),
    }
}

/// Whether `path` could be a folder above something `glob` matches (`plugins/mesh` is above
/// `plugins/*/ui/**`).
fn above(glob: &[&str], path: &[&str]) -> bool {
    match (glob.split_first(), path.split_first()) {
        (_, None) => !glob.is_empty(),
        (None, Some(_)) => false,
        (Some((&"**", _)), Some(_)) => true,
        (Some((name, rest)), Some((first, tail))) => name_match(name.as_bytes(), first.as_bytes()) && above(rest, tail),
    }
}

/// Whether a point's scope entry (a file or a folder) reaches what `glob` names. A scope that is
/// a folder above the glob's paths reaches them too. Globs that begin with `**` name files
/// anywhere, and every folder is above some such file, so for those only the entry itself counts.
pub fn touch_hits(glob: &str, touch: &str) -> bool {
    if glob_match(glob, touch) {
        return true;
    }
    let (glob, touch) = (parts(glob), parts(touch));
    glob.first() != Some(&"**") && above(&glob, &touch)
}

/// Whether two scope entries overlap: the same path, or one is a folder above the other.
pub fn overlap(a: &str, b: &str) -> bool {
    let (a, b) = (parts(a), parts(b));
    let shared = a.len().min(b.len());
    a[..shared] == b[..shared]
}

/// Whether `file` is inside a scope entry.
pub fn inside(touch: &str, file: &str) -> bool {
    let (touch, file) = (parts(touch), parts(file));
    (touch.len() <= file.len() && touch[..] == file[..touch.len()]) || segments(&touch, &file)
}

/// How long a package of this tier is held, in minutes. None: it waits for a yes (major).
pub fn hold_min(policy: &Policy, tier: Tier) -> Option<u32> {
    match tier {
        Tier::Safe => Some(policy.hold_safe_min),
        Tier::Minor => Some(policy.hold_minor_min),
        Tier::Major => None,
    }
}

/// When a package's wait ends: `now` plus the tier's time (the time similar requests have to
/// come in and join it). 0 for a major package (it waits for a yes), `now` for an urgent one.
pub fn hold_until(policy: &Policy, tier: Tier, urgent: bool, now_ms: u64) -> u64 {
    if urgent {
        return now_ms;
    }
    match hold_min(policy, tier) {
        None => 0,
        Some(minutes) => now_ms + u64::from(minutes) * 60_000,
    }
}

/// `[minutes, tokens, lines]`.
pub fn budget(policy: &Policy, tier: Tier) -> [u64; 3] {
    match tier {
        Tier::Safe => policy.budget_safe,
        Tier::Minor => policy.budget_minor,
        Tier::Major => policy.budget_major,
    }
}

pub fn rework(policy: &Policy, tier: Tier) -> u32 {
    match tier {
        Tier::Safe => policy.rework_safe,
        Tier::Minor => policy.rework_minor,
        Tier::Major => policy.rework_major,
    }
}

/// How long finished work runs in the person's apps before its release: never. It goes with the
/// repository's next scheduled release (owner, 2026-11-21).
pub fn soak_min(_policy: &Policy, _tier: Tier, _urgent: bool) -> u32 {
    0
}
