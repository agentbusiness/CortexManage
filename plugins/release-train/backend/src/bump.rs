//! A part's next version (`docs/specs/02-HOLD-PACKAGES-RELEASE.md` §7): three numbers, each 0 to
//! 99, carried into the number before (`0.3.99` → `0.4.0`, `0.99.99` → `1.0.0`).

/// The largest value of each of the three numbers.
const TOP: u32 = 99;

/// `0.4.12` as its three numbers; `None` when it is not three numbers of 0 to 99.
pub fn parse(version: &str) -> Option<[u32; 3]> {
    let mut numbers = version.trim().split('.').map(|part| {
        // `u32::from_str` takes a leading `+`; a version does not.
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) { None } else { part.parse::<u32>().ok().filter(|n| *n <= TOP) }
    });
    let out = [numbers.next()??, numbers.next()??, numbers.next()??];
    if numbers.next().is_some() { None } else { Some(out) }
}

/// The version after `version` for a change of `tier` (`safe`, `minor`, `major`). `breaking` is a
/// major change whose contract lost or changed a field. The error is a sentence for a person.
pub fn next(version: &str, tier: &str, breaking: bool) -> Result<String, String> {
    let [first, middle, last] = parse(version).ok_or_else(|| format!("\"{version}\" is not a version of three numbers from 0 to 99, like 0.4.12."))?;
    let full = || format!("There is no version after {version} for this change: its numbers cannot go higher.");
    let first_up = |first: u32| if first < TOP { Ok([first + 1, 0, 0]) } else { Err(full()) };
    let middle_up = |first: u32, middle: u32| if middle < TOP { Ok([first, middle + 1, 0]) } else { first_up(first) };
    let [first, middle, last] = match (tier, breaking) {
        ("safe", _) => {
            if last < TOP {
                [first, middle, last + 1]
            } else {
                middle_up(first, middle)?
            }
        }
        ("minor", _) | ("major", false) => middle_up(first, middle)?,
        ("major", true) => first_up(first)?,
        _ => return Err(format!("\"{tier}\" is not a tier. The tiers are safe, minor and major.")),
    };
    Ok(format!("{first}.{middle}.{last}"))
}
