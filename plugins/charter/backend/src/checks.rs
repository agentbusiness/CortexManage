//! The checks kept by code: plain structural tests over a change's file list and the lines its
//! unified diff adds. What each one looks for is in `data/checks.toml`, with the sentence a person
//! reads. A check is reported as run only when it could really decide: a check that needs a file's
//! added lines did not run when that file is named but its diff is missing.

use crate::book::shown_id;
use crate::data::{Checks, fill};
use cc_protocol::contracts::charter::{CheckAnswer, CheckFinding, CheckParams, Rule, kept_by};

pub const NO_RECORD_FILES: &str = "no-record-files";
pub const NO_TESTS: &str = "no-tests";
pub const NO_NATIVE_CONTROLS: &str = "no-native-controls";
pub const THEME_UNTOUCHED: &str = "theme-untouched";
pub const NO_SECRETS: &str = "no-secrets";
pub const AGENTS_FILE_UNTOUCHED: &str = "agents-file-untouched";

/// One file of a change.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changed {
    pub path: String,
    /// The diff says the file did not exist before.
    pub new: bool,
    /// The diff deletes the file.
    pub gone: bool,
    /// The diff holds this file (its added lines are known); false for a path only named in `files`.
    pub in_diff: bool,
    /// Added lines with their line number in the new file.
    pub added: Vec<(u32, String)>,
}

impl Changed {
    fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    fn folders(&self) -> Vec<&str> {
        let mut parts: Vec<&str> = self.path.split('/').collect();
        parts.pop();
        parts
    }
}

/// `a/x b/x` -> `x`; a path with a timestamp after a tab loses it.
fn diff_path(text: &str) -> String {
    let text = text.split('\t').next().unwrap_or(text).trim();
    let text = text.trim_matches('"');
    text.strip_prefix("a/").or_else(|| text.strip_prefix("b/")).unwrap_or(text).to_owned()
}

/// `@@ -3,4 +5,6 @@` -> (4, 5, 6): old lines, first new line, new lines.
fn hunk_head(line: &str) -> Option<(u32, u32, u32)> {
    let mut parts = line.strip_prefix("@@ ")?.split(' ');
    let range = |text: &str| -> Option<(u32, u32)> {
        let (start, count) = text.split_once(',').unwrap_or((text, "1"));
        Some((start.parse().ok()?, count.parse().ok()?))
    };
    let (_, old_count) = range(parts.next()?.strip_prefix('-')?)?;
    let (new_start, new_count) = range(parts.next()?.strip_prefix('+')?)?;
    Some((old_count, new_start, new_count))
}

/// The files of a unified diff (`git diff` or plain `diff -u`) with the lines each adds.
pub fn parse_diff(diff: &str) -> Vec<Changed> {
    let mut files: Vec<Changed> = Vec::new();
    // Lines left in the hunk being read: a `---` inside a hunk is a removed line, not a file.
    let (mut old_left, mut new_left, mut new_line) = (0u32, 0u32, 0u32);
    // The current file's `+++` line was read (the next `---` starts another file).
    let mut headed = false;
    for line in diff.lines() {
        if old_left > 0 || new_left > 0 {
            let current = files.last_mut();
            match line.as_bytes().first() {
                Some(b'+') => {
                    if let Some(file) = current {
                        file.added.push((new_line, line[1..].to_owned()));
                    }
                    new_line += 1;
                    new_left = new_left.saturating_sub(1);
                }
                Some(b'-') => old_left = old_left.saturating_sub(1),
                Some(b'\\') => {}
                _ => {
                    new_line += 1;
                    old_left = old_left.saturating_sub(1);
                    new_left = new_left.saturating_sub(1);
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("diff --git ") {
            // `a/x b/x`: the new path is what follows the last ` b/`.
            let path = rest.rsplit_once(" b/").map(|(_, path)| path.trim_matches('"').to_owned()).unwrap_or_else(|| diff_path(rest));
            files.push(Changed { path, in_diff: true, ..Changed::default() });
            headed = false;
        } else if let Some(rest) = line.strip_prefix("--- ") {
            if files.is_empty() || headed {
                files.push(Changed { in_diff: true, ..Changed::default() });
                headed = false;
            }
            let file = files.last_mut().expect("a file");
            if rest.trim() == "/dev/null" {
                file.new = true;
            } else if file.path.is_empty() {
                file.path = diff_path(rest);
            }
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            if let Some(file) = files.last_mut() {
                if rest.trim() == "/dev/null" {
                    file.gone = true;
                } else {
                    file.path = diff_path(rest);
                }
                headed = true;
            }
        } else if line.starts_with("new file mode") {
            if let Some(file) = files.last_mut() {
                file.new = true;
            }
        } else if line.starts_with("deleted file mode") {
            if let Some(file) = files.last_mut() {
                file.gone = true;
            }
        } else if let Some(rest) = line.strip_prefix("rename to ") {
            if let Some(file) = files.last_mut() {
                file.path = rest.trim().trim_matches('"').to_owned();
            }
        } else if let Some((old_count, new_start, new_count)) = hunk_head(line) {
            old_left = old_count;
            new_left = new_count;
            new_line = new_start;
            headed = true;
        }
    }
    files.retain(|file| !file.path.is_empty());
    files
}

/// The diff's files, then the paths only named in `files`.
fn changed_files(params: &CheckParams) -> Vec<Changed> {
    let mut files = parse_diff(&params.diff);
    for path in &params.files {
        let path = path.trim().trim_start_matches("./");
        if !path.is_empty() && !files.iter().any(|file| file.path == path) {
            files.push(Changed { path: path.to_owned(), ..Changed::default() });
        }
    }
    files
}

/// What one check found, and whether it could decide.
struct Outcome {
    ran: bool,
    /// (file, line, the finding's `{name}` values).
    found: Vec<(String, u32, Vec<(&'static str, String)>)>,
}

fn ends_with_any(name: &str, endings: &[String]) -> Option<String> {
    endings.iter().find(|ending| name.ends_with(ending.as_str())).cloned()
}

/// A new record-shaped file under a plugin's backend or data folder.
fn no_record_files(files: &[Changed], data: &Checks) -> Outcome {
    let records = &data.records;
    let mut out = Outcome { ran: true, found: Vec::new() };
    for file in files.iter().filter(|file| !file.gone) {
        let name = file.name();
        let Some(ending) = ends_with_any(name, &records.endings) else { continue };
        let allowed = records.allowed_names.iter().any(|allowed| allowed == name)
            || records.allowed_starts.iter().any(|start| name.starts_with(start.as_str()))
            || records.allowed_ends.iter().any(|end| name.ends_with(end.as_str()));
        if allowed {
            continue;
        }
        // `plugins/<id>/…/<folder>/…`: the folder is looked for below the plugin's own folder.
        let folders = file.folders();
        let Some(plugins_at) = folders.iter().position(|folder| *folder == "plugins") else { continue };
        let Some(folder) = folders.iter().skip(plugins_at + 2).find(|folder| records.folders.iter().any(|known| known == *folder)) else { continue };
        if !file.in_diff {
            // Only the diff says whether a file is new: this one cannot be decided.
            out.ran = false;
        } else if file.new {
            out.found.push((file.path.clone(), 0, vec![("ending", ending), ("folder", (*folder).to_owned())]));
        }
    }
    out
}

/// Test code, test folders, test files.
fn no_tests(files: &[Changed], data: &Checks) -> Outcome {
    let tests = &data.tests;
    let mut out = Outcome { ran: true, found: Vec::new() };
    for file in files.iter().filter(|file| !file.gone) {
        if let Some(folder) = file.folders().into_iter().find(|folder| tests.folders.iter().any(|known| known == folder)) {
            out.found.push((file.path.clone(), 0, vec![("what", format!("it is in a `{folder}` folder"))]));
            continue;
        }
        let parts: Vec<&str> = file.name().split('.').collect();
        if parts.len() >= 3
            && let Some(part) = parts[1..parts.len() - 1].iter().find(|part| tests.name_parts.iter().any(|known| known == *part))
        {
            out.found.push((file.path.clone(), 0, vec![("what", format!("its name has `.{part}.`"))]));
            continue;
        }
        if ends_with_any(file.name(), &tests.line_endings).is_none() {
            continue;
        }
        if !file.in_diff {
            out.ran = false;
            continue;
        }
        for (line, text) in &file.added {
            if let Some(start) = tests.line_starts.iter().find(|start| text.trim_start().starts_with(start.as_str())) {
                out.found.push((file.path.clone(), *line, vec![("what", format!("line {line} starts with `{start}`"))]));
            }
        }
    }
    out
}

/// `<name` as the start of an element: followed by a space, `>`, `/` or the line's end.
fn has_tag(text: &str, name: &str) -> bool {
    let open = format!("<{name}");
    text.match_indices(&open).any(|(at, _)| text[at + open.len()..].chars().next().is_none_or(|next| next.is_whitespace() || next == '>' || next == '/'))
}

fn no_native_controls(files: &[Changed], data: &Checks) -> Outcome {
    let controls = &data.controls;
    let mut out = Outcome { ran: true, found: Vec::new() };
    for file in files.iter().filter(|file| !file.gone && ends_with_any(file.name(), &controls.endings).is_some()) {
        if !file.in_diff {
            out.ran = false;
            continue;
        }
        for (line, text) in &file.added {
            for mark in &controls.mark {
                let hit = (!mark.tag.is_empty() && has_tag(text, &mark.tag)) || (!mark.text.is_empty() && text.contains(&mark.text));
                if hit {
                    out.found.push((file.path.clone(), *line, vec![("what", format!("{} on line {line}", mark.what))]));
                }
            }
        }
    }
    out
}

fn theme_untouched(files: &[Changed], data: &Checks, standard_change: bool) -> Outcome {
    let folder = data.theme.folder.as_str();
    let found = if standard_change || folder.is_empty() {
        Vec::new()
    } else {
        files.iter().filter(|file| file.path.starts_with(folder)).map(|file| (file.path.clone(), 0, Vec::new())).collect()
    };
    Outcome { ran: true, found }
}

fn agents_file_untouched(files: &[Changed], data: &Checks, standard_change: bool) -> Outcome {
    let found = if standard_change {
        Vec::new()
    } else {
        files.iter().filter(|file| data.printed.names.iter().any(|name| name == file.name())).map(|file| (file.path.clone(), 0, Vec::new())).collect()
    };
    Outcome { ran: true, found }
}

fn token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

/// A token with a known start, long enough after it, not in the middle of a word.
fn known_token<'a>(text: &str, data: &'a Checks) -> Option<&'a str> {
    data.secrets.token.iter().find_map(|token| {
        text.match_indices(token.starts.as_str())
            .any(|(at, _)| {
                let alone = text[..at].chars().next_back().is_none_or(|before| !before.is_ascii_alphanumeric());
                alone && text[at + token.starts.len()..].chars().take_while(|ch| token_char(*ch)).count() >= token.tail
            })
            .then_some(token.what.as_str())
    })
}

/// A long literal given to a secret-sounding name: `api_key = "…"`, `"token": "…"`.
fn secret_assignment(text: &str, data: &Checks) -> bool {
    let secrets = &data.secrets;
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        let quote = chars[at];
        if !matches!(quote, '"' | '\'' | '`') {
            at += 1;
            continue;
        }
        let Some(length) = chars[at + 1..].iter().position(|ch| *ch == quote) else { break };
        let value = &chars[at + 1..at + 1 + length];
        let shaped = value.len() >= secrets.value_chars
            && value.iter().all(|ch| ch.is_ascii_alphanumeric() || "+/=_-.".contains(*ch))
            && value.iter().any(char::is_ascii_digit)
            && value.iter().any(char::is_ascii_alphabetic);
        if shaped {
            // Backwards from the value: spaces, `=` or `:`, spaces, a closing quote, then the name.
            let mut back = at;
            while back > 0 && chars[back - 1].is_whitespace() {
                back -= 1;
            }
            if back > 0 && matches!(chars[back - 1], '=' | ':') {
                back -= 1;
                while back > 0 && (chars[back - 1].is_whitespace() || matches!(chars[back - 1], '"' | '\'')) {
                    back -= 1;
                }
                let end = back;
                while back > 0 && (token_char(chars[back - 1]) || chars[back - 1] == '.') {
                    back -= 1;
                }
                let name: String = chars[back..end].iter().collect::<String>().to_lowercase();
                if secrets.names.iter().any(|known| name.contains(known.as_str())) {
                    return true;
                }
            }
        }
        at += length + 2;
    }
    false
}

fn no_secrets(files: &[Changed], data: &Checks) -> Outcome {
    let secrets = &data.secrets;
    let mut out = Outcome { ran: true, found: Vec::new() };
    for file in files.iter().filter(|file| !file.gone) {
        if !file.in_diff {
            out.ran = false;
            continue;
        }
        for (line, text) in &file.added {
            let header = text.split_once(secrets.header_start.as_str()).is_some_and(|(_, rest)| rest.contains(secrets.header_end.as_str()));
            let what = if header {
                Some("a private key".to_owned())
            } else if let Some(what) = known_token(text, data) {
                Some(what.to_owned())
            } else if secret_assignment(text, data) {
                Some("a key or token given to a name".to_owned())
            } else {
                None
            };
            if let Some(what) = what {
                out.found.push((file.path.clone(), *line, vec![("what", format!("{what} on line {line}"))]));
            }
        }
    }
    out
}

/// Run the checks this repository's own rules ask for. `rules` are the rules in force for the
/// repository: a check runs only when one of them says it is kept by that check, and a finding
/// names that rule. A repository without such a rule is not held to it. The one check every
/// repository gets is the one for secrets.
pub fn run(params: &CheckParams, rules: &[Rule], data: &Checks) -> CheckAnswer {
    let files = changed_files(params);
    let outcomes = [
        (NO_RECORD_FILES, no_record_files(&files, data)),
        (NO_TESTS, no_tests(&files, data)),
        (NO_NATIVE_CONTROLS, no_native_controls(&files, data)),
        (THEME_UNTOUCHED, theme_untouched(&files, data, params.standard_change)),
        (NO_SECRETS, no_secrets(&files, data)),
        (AGENTS_FILE_UNTOUCHED, agents_file_untouched(&files, data, params.standard_change)),
    ];
    let mut answer = CheckAnswer { ok: true, ran: Vec::new(), findings: Vec::new() };
    for (id, outcome) in outcomes {
        // A check switched off in the data file (`not_run`) does not run, and says so by its absence.
        let Some(check) = data.text(id).filter(|check| check.not_run.is_empty()) else { continue };
        let rule = rules.iter().find(|rule| rule.kept_by == kept_by::CHECK && rule.check == id).map(|rule| shown_id(rule, &params.repo)).unwrap_or_default();
        if rule.is_empty() && id != NO_SECRETS {
            continue;
        }
        if outcome.ran {
            answer.ran.push(id.to_owned());
        }
        for (file, line, mut vars) in outcome.found {
            vars.push(("file", file.clone()));
            let vars: Vec<(&str, &str)> = vars.iter().map(|(name, value)| (*name, value.as_str())).collect();
            answer.findings.push(CheckFinding { check: id.to_owned(), rule: rule.clone(), file, line, text: fill(&check.finding, &vars) });
        }
    }
    answer.ok = answer.findings.is_empty();
    answer
}
