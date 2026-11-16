//! What the repository's policy decides, in plain code: the tier's floor from the paths a plan
//! touches and from its size, how long a package is held, its budget, how many rework rounds it
//! gets and how long it soaks. No model is asked here.

use crate::texts::Texts;
use cc_protocol::contracts::manage::{Point, Policy, Tier};

/// More points than this make a change major (CortexCode's plan limit).
pub const MAX_POINTS: usize = 12;
/// More limit lines than this in one plan make a change major.
pub const MAX_LINES: u32 = 2500;
/// How many reasons of one sort are written out; the rest are counted.
const REASONS: usize = 4;

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

/// The floor and why, one sentence per reason.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Floor {
    pub tier: Tier,
    pub reasons: Vec<String>,
}

fn name_some(texts: &Texts, key: &str, paths: &[String]) -> Vec<String> {
    let mut out: Vec<String> = paths.iter().take(REASONS).map(|path| texts.get(key, &[("path", path)])).collect();
    if paths.len() > REASONS {
        out.push(texts.get("reason_more", &[("count", &(paths.len() - REASONS).to_string())]));
    }
    out
}

/// The tier's floor of a plan: from its kept points' scopes against the policy's paths, from the
/// standard paths among them (`standard`, as the standard plugin answered), and from its size.
pub fn floor(policy: &Policy, points: &[Point], standard: &[String], texts: &Texts) -> Floor {
    let mut touches: Vec<String> = points.iter().flat_map(|point| point.touches.iter().cloned()).collect();
    touches.sort();
    touches.dedup();
    let hit = |globs: &[String]| -> Vec<String> { touches.iter().filter(|touch| globs.iter().any(|glob| touch_hits(glob, touch))).cloned().collect() };
    let major = hit(&policy.major_paths);
    let minor: Vec<String> = hit(&policy.minor_paths).into_iter().filter(|path| !major.contains(path)).collect();
    let lines: u32 = points.iter().map(|point| point.limit_lines).sum();

    let mut tier = Tier::Safe;
    let mut reasons = Vec::new();
    if !major.is_empty() {
        tier = Tier::Major;
        reasons.extend(name_some(texts, "reason_major_path", &major));
    }
    if !standard.is_empty() {
        tier = Tier::Major;
        reasons.extend(name_some(texts, "reason_standard", standard));
    }
    if points.len() > MAX_POINTS {
        tier = Tier::Major;
        reasons.push(texts.get("reason_many_points", &[("count", &points.len().to_string()), ("limit", &MAX_POINTS.to_string())]));
    }
    if lines > MAX_LINES {
        tier = Tier::Major;
        reasons.push(texts.get("reason_many_lines", &[("count", &lines.to_string()), ("limit", &MAX_LINES.to_string())]));
    }
    if !minor.is_empty() {
        tier = tier.max(Tier::Minor);
        reasons.extend(name_some(texts, "reason_minor_path", &minor));
    }
    if reasons.is_empty() {
        reasons.push(texts.get("reason_safe", &[]));
    }
    Floor { tier, reasons }
}

/// How long a package of this tier is held, in minutes. None: it waits for a yes (major).
pub fn hold_min(policy: &Policy, tier: Tier) -> Option<u32> {
    match tier {
        Tier::Safe => Some(policy.hold_safe_min),
        Tier::Minor => Some(policy.hold_minor_min),
        Tier::Major => None,
    }
}

/// When a package's hold ends: `now` plus the tier's time, never later than the longest hold
/// counted from when the package was first held. 0 for a major package (no clock), `now` for an
/// urgent one.
pub fn hold_until(policy: &Policy, tier: Tier, urgent: bool, now_ms: u64, held_since_ms: u64) -> u64 {
    if urgent {
        return now_ms;
    }
    match hold_min(policy, tier) {
        None => 0,
        Some(minutes) => {
            let cap = held_since_ms + u64::from(policy.hold_max_min) * 60_000;
            (now_ms + u64::from(minutes) * 60_000).min(cap).max(now_ms)
        }
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

/// Minutes a landed package soaks; none for an urgent one.
pub fn soak_min(policy: &Policy, tier: Tier, urgent: bool) -> u32 {
    if urgent {
        return 0;
    }
    match tier {
        Tier::Safe => policy.soak_safe_min,
        Tier::Minor => policy.soak_minor_min,
        Tier::Major => policy.soak_major_min,
    }
}
