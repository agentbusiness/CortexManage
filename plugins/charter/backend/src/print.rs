//! The `AGENTS.md` printed from a charter, and the comparison of a repository's file with it.
//!
//! The order is fixed: laws (the standard's, then the app's), constraints, then what the app is,
//! then conventions. CortexCode gives its model the first 6000 characters of the file, cut at `#`
//! headings: each section whole while it fits. So the laws and constraints come first, under their
//! own headings, and `head_chars` says how far into the file they end.

use crate::book::{Book, STANDARD, sha256, shown_id};
use crate::data::Texts;
use cc_protocol::contracts::charter::{DriftAnswer, Printed, Rule, level};
use std::collections::BTreeMap;

/// How many characters of the file CortexCode's model reads.
pub const HEAD_LIMIT: u32 = 6000;
/// What the banner's hashes are written after; the comparison reads them back by these.
const CHARTER_HASH: &str = "charter sha256:";
const TEXT_HASH: &str = "text sha256:";

/// A rule as one line: its id, then its text without line breaks. A small model follows a short
/// numbered line best.
fn rule_line(rule: &Rule, repo: &str) -> String {
    let text = rule.text.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("- {}. {text}\n", shown_id(rule, repo))
}

/// One level's section: the standard's rules under one heading, the app's under another.
fn level_section(texts: &Texts, book: &Book, standard: Option<&Book>, level_name: &str) -> String {
    let of = |source: &Book| source.rules.iter().filter(|rule| rule.level == level_name).map(|rule| rule_line(rule, &book.repo)).collect::<String>();
    let shared = standard.map(of).unwrap_or_default();
    let own = of(book);
    if shared.is_empty() && own.is_empty() {
        return String::new();
    }
    let mut out = format!("# {}\n\n{}\n\n", texts.get(&format!("print_{level_name}_title"), &[]), texts.get(&format!("print_{level_name}_intro"), &[]));
    if book.repo == STANDARD {
        out.push_str(&own);
        out.push('\n');
        return out;
    }
    if !shared.is_empty() {
        out.push_str(&format!("## {}\n\n{shared}\n", texts.get(&format!("print_{level_name}_standard"), &[])));
    }
    if !own.is_empty() {
        out.push_str(&format!("## {}\n\n{own}\n", texts.get(&format!("print_{level_name}_own"), &[])));
    }
    out
}

fn identity_section(texts: &Texts, book: &Book) -> String {
    let identity = &book.identity;
    if identity.what.is_empty() && identity.who_for.is_empty() && identity.never.is_empty() {
        return String::new();
    }
    let title = if book.repo == STANDARD { "print_identity_title_standard" } else { "print_identity_title" };
    let mut out = format!("# {}\n\n", texts.get(title, &[]));
    if !identity.what.is_empty() {
        out.push_str(&format!("{}\n\n", identity.what));
    }
    if !identity.who_for.is_empty() {
        out.push_str(&format!("{}\n\n", texts.get("print_who_for", &[("who_for", &identity.who_for)])));
    }
    if !identity.never.is_empty() {
        out.push_str(&format!("{}\n\n", texts.get("print_never", &[])));
        for line in &identity.never {
            out.push_str(&format!("- {line}\n"));
        }
        out.push('\n');
    }
    out
}

/// The file's first line and everything after it.
fn cut_banner(text: &str) -> (&str, &str) {
    text.split_once('\n').unwrap_or((text, ""))
}

/// The hash written after `label` in a banner.
fn banner_hash<'a>(banner: &'a str, label: &str) -> Option<&'a str> {
    let rest = banner.split_once(label)?.1;
    let end = rest.find(|ch: char| !ch.is_ascii_hexdigit()).unwrap_or(rest.len());
    (end == 64).then(|| &rest[..end])
}

pub fn print(texts: &Texts, book: &Book, standard: Option<&Book>) -> Printed {
    let mut body = String::new();
    body.push_str(&level_section(texts, book, standard, level::LAW));
    body.push_str(&level_section(texts, book, standard, level::CONSTRAINT));
    let head_body = body.chars().count();
    body.push_str(&identity_section(texts, book));
    body.push_str(&level_section(texts, book, None, level::CONVENTION));
    let body = format!("{}\n", body.trim_end());
    let digest = sha256(&[&body]);
    let version = if book.draft { texts.get("print_draft_version", &[]) } else { book.version.clone() };
    let standard_version = standard.map(|standard| texts.get("print_banner_standard", &[("version", &standard.version)])).unwrap_or_default();
    let banner = format!(
        "<!-- {}{standard_version} {CHARTER_HASH}{} {TEXT_HASH}{digest} -->\n",
        texts.get("print_banner", &[("repo", &book.repo), ("version", &version)]),
        book.digest(),
    );
    // The model's 6000 characters start at the file's first character, so the banner counts.
    let head_chars = (banner.chars().count() + head_body.min(body.chars().count())) as u32;
    Printed { repo: book.repo.clone(), version: book.version.clone(), text: format!("{banner}{body}"), digest, head_chars }
}

/// A text as it is compared: one line ending, nothing after the last line.
fn normal(text: &str) -> String {
    format!("{}\n", text.replace("\r\n", "\n").trim_end())
}

/// The lines of `a` that `b` does not have (a line `b` has once and `a` twice counts once).
fn only_in(a: &str, b: &str) -> Vec<String> {
    let mut have: BTreeMap<&str, i32> = BTreeMap::new();
    for line in b.lines().filter(|line| !line.trim().is_empty()) {
        *have.entry(line).or_default() += 1;
    }
    let mut out = Vec::new();
    for line in a.lines().filter(|line| !line.trim().is_empty()) {
        match have.get_mut(line) {
            Some(left) if *left > 0 => *left -= 1,
            _ => out.push(line.to_owned()),
        }
    }
    out
}

/// Compare a repository's file with the print; the banner line is not compared.
///
/// `same`: the file is the print. `edited`: it was printed and somebody changed it. `outdated`: it
/// is an untouched print of an earlier charter (the banner's own hash still fits its text), so
/// nobody edited it and it only needs printing again. `not_printed`: it has no banner.
pub fn drift(printed: &Printed, file: &str) -> DriftAnswer {
    let print_body = normal(cut_banner(&printed.text).1);
    let file = file.replace("\r\n", "\n");
    let (first, rest) = cut_banner(&file);
    let banner_text_hash = if first.trim_start().starts_with("<!--") { banner_hash(first, TEXT_HASH) } else { None };
    let file_body = match banner_text_hash {
        Some(_) => normal(rest),
        None => normal(&file),
    };
    let state = match banner_text_hash {
        None => "not_printed",
        Some(_) if file_body == print_body => "same",
        Some(hash) if sha256(&[&file_body]) == hash => "outdated",
        Some(_) => "edited",
    };
    if state == "same" {
        return DriftAnswer { state: state.into(), ..DriftAnswer::default() };
    }
    DriftAnswer { state: state.into(), added: only_in(&file_body, &print_body), removed: only_in(&print_body, &file_body) }
}
