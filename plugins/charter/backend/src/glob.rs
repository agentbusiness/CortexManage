//! Path globs of a rule's `applies_to`: `*` and `?` inside one folder name, `**` for any number of
//! folders. A glob with no `/` names a file in any folder (`*.tsx`); one ending in `/` names
//! everything under that folder.

/// A glob as it is matched: no leading `./` or `/`, the two short forms written out.
fn normal(glob: &str) -> String {
    let glob = glob.trim().trim_start_matches("./").trim_start_matches('/');
    if glob.is_empty() {
        return "**".into();
    }
    if let Some(folder) = glob.strip_suffix('/') {
        return format!("{folder}/**");
    }
    if !glob.contains('/') && glob != "**" {
        return format!("**/{glob}");
    }
    glob.to_owned()
}

fn clean_path(path: &str) -> &str {
    path.trim().trim_start_matches("./").trim_start_matches('/')
}

/// One folder or file name against one glob part.
fn part_matches(glob: &[char], name: &[char]) -> bool {
    match glob.split_first() {
        None => name.is_empty(),
        Some(('*', rest)) => (0..=name.len()).any(|skip| part_matches(rest, &name[skip..])),
        Some(('?', rest)) => !name.is_empty() && part_matches(rest, &name[1..]),
        Some((ch, rest)) => name.first() == Some(ch) && part_matches(rest, &name[1..]),
    }
}

fn parts_match(glob: &[&str], path: &[&str]) -> bool {
    match glob.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| parts_match(rest, &path[skip..])),
        Some((part, rest)) => match path.split_first() {
            Some((name, more)) => {
                let (part, name): (Vec<char>, Vec<char>) = (part.chars().collect(), name.chars().collect());
                part_matches(&part, &name) && parts_match(rest, more)
            }
            None => false,
        },
    }
}

pub fn matches(glob: &str, path: &str) -> bool {
    let glob = normal(glob);
    let glob: Vec<&str> = glob.split('/').collect();
    let path: Vec<&str> = clean_path(path).split('/').collect();
    parts_match(&glob, &path)
}

/// Whether a rule with these globs applies to `path`. No globs: the rule applies everywhere.
pub fn applies(globs: &[String], path: &str) -> bool {
    globs.is_empty() || globs.iter().any(|glob| matches(glob, path))
}

fn wild(ch: char) -> bool {
    ch == '*' || ch == '?'
}

/// Whether one path could match both globs. It never says no when such a path exists: two globs
/// that share a path agree on the text before their first wildcard and after their last one. It
/// may say yes for two globs that share none (the middle is not compared).
fn pair_may_overlap(a: &str, b: &str) -> bool {
    let (a, b) = (normal(a), normal(b));
    if !a.contains(wild) && !b.contains(wild) {
        return a == b;
    }
    let head = |glob: &str| glob.split(wild).next().unwrap_or_default().to_owned();
    let tail = |glob: &str| glob.rsplit(wild).next().unwrap_or_default().to_owned();
    let (head_a, head_b) = (head(&a), head(&b));
    let (tail_a, tail_b) = (tail(&a), tail(&b));
    (head_a.starts_with(&head_b) || head_b.starts_with(&head_a)) && (tail_a.ends_with(&tail_b) || tail_b.ends_with(&tail_a))
}

/// Whether two rules that both name paths could apply to one file. A rule that names no paths
/// applies everywhere and is not compared here (see the README: it would overlap every rule).
pub fn may_overlap(a: &[String], b: &[String]) -> bool {
    a.iter().any(|one| b.iter().any(|other| pair_may_overlap(one, other)))
}
