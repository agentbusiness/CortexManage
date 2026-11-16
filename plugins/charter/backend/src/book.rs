//! The charters and what may be done to them: drafts, accepting, a law edited by hand, proposals
//! and amendments. Plain values and plain code, no I/O: the service keeps a [`Shelf`] in memory and
//! writes every change to the graph, and the probe runs the same code on a shelf of its own.

use crate::data::Texts;
use crate::glob;
use crate::import;
use cc_protocol::contracts::charter::{AmendParams, Charter, Identity, ImportParams, Proposal, ProposeParams, Rule, RulePut, RulesForParams, kept_by, level};
use cc_protocol::{RpcError, codes};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// The repository id of the standard charter, whose laws and constraints every app carries.
pub const STANDARD: &str = "standard";
/// A draft's version, and the version a draft becomes.
pub const DRAFT_VERSION: &str = "0.0";
pub const FIRST_VERSION: &str = "1.0";
/// What the owner types to change the identity of an accepted charter.
pub const CONFIRM_IDENTITY: &str = "identity";

const LEVELS: [&str; 3] = [level::LAW, level::CONSTRAINT, level::CONVENTION];
const KEPT_BY: [&str; 3] = [kept_by::CHECK, kept_by::VALIDATOR, kept_by::TEXT];

pub fn sha256(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            hash.update(b"\n");
        }
        hash.update(part.as_bytes());
    }
    hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What an approval is bound to: the repository, the rule and the exact new wording.
pub fn proposal_digest(repo: &str, id: &str, new: &str) -> String {
    sha256(&[repo, id, new])
}

fn level_index(name: &str) -> usize {
    LEVELS.iter().position(|known| *known == name).unwrap_or(LEVELS.len())
}

fn letter(level_name: &str) -> char {
    match level_name {
        level::LAW => 'L',
        level::CONSTRAINT => 'C',
        _ => 'V',
    }
}

/// `C12` -> 12.
fn number(id: &str) -> u32 {
    id.get(1..).and_then(|digits| digits.parse().ok()).unwrap_or(0)
}

/// A rule's id as it is shown beside another charter's rules: the standard's are `S-L1`, so they
/// are never taken for the app's own `L1`.
pub fn shown_id(rule: &Rule, beside: &str) -> String {
    if rule.scope == STANDARD && beside != STANDARD { format!("S-{}", rule.id) } else { rule.id.clone() }
}

/// One charter as it is kept.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Book {
    pub repo: String,
    pub version: String,
    pub draft: bool,
    pub identity: Identity,
    /// The rules in force, laws first, each level by number.
    pub rules: Vec<Rule>,
    /// The next number of each level (law, constraint, convention). A number is used once, so an
    /// id never comes to mean another rule.
    pub next: [u32; 3],
    pub updated_ms: u64,
}

impl Book {
    fn draft(repo: &str, now: u64) -> Book {
        Book { repo: repo.to_owned(), version: DRAFT_VERSION.into(), draft: true, next: [1; 3], updated_ms: now, ..Book::default() }
    }

    fn new_id(&mut self, level_name: &str) -> String {
        let slot = &mut self.next[level_index(level_name).min(2)];
        // A counter read back smaller than a rule in force (an older record) never gives a used id.
        let used = self.rules.iter().filter(|rule| rule.level == level_name).map(|rule| number(&rule.id)).max().unwrap_or(0);
        let given = (*slot).max(used + 1).max(1);
        *slot = given + 1;
        format!("{}{given}", letter(level_name))
    }

    fn sort(&mut self) {
        self.rules.sort_by_key(|rule| (level_index(&rule.level), number(&rule.id)));
    }

    fn find(&self, id: &str) -> Option<usize> {
        self.rules.iter().position(|rule| rule.id == id)
    }

    /// sha256 of everything the charter says: its version, identity and rules.
    pub fn digest(&self) -> String {
        let mut lines = vec![self.repo.clone(), self.version.clone(), self.identity.what.clone(), self.identity.who_for.clone(), self.identity.never.join("\u{1f}")];
        for rule in &self.rules {
            lines.push([rule.id.as_str(), &rule.level, &rule.text, &rule.why, &rule.applies_to.join(","), &rule.kept_by, &rule.check].join("\u{1f}"));
        }
        sha256(&lines.iter().map(String::as_str).collect::<Vec<_>>())
    }

    /// The next version: every change to an accepted charter adds one to the second number.
    fn bump(&mut self) {
        let (first, second) = self.version.split_once('.').unwrap_or((self.version.as_str(), "0"));
        let first: u32 = first.parse().unwrap_or(1);
        let second: u32 = second.parse().unwrap_or(0);
        self.version = format!("{first}.{}", second + 1);
    }
}

/// What a new version records: who made it and which rule it changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Note {
    /// accept | owner | amend.
    pub by: String,
    pub request: String,
    /// The rule's id after the change (empty: the identity, or the whole draft).
    pub rule: String,
    pub level: String,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, Default)]
pub struct Shelf {
    pub books: BTreeMap<String, Book>,
}

fn refuse(text: String) -> RpcError {
    RpcError::new(codes::CONFLICT, text)
}

fn clean(text: &str) -> String {
    text.trim().to_owned()
}

fn clean_globs(globs: &[String]) -> Vec<String> {
    globs.iter().map(|glob| clean(glob)).filter(|glob| !glob.is_empty()).collect()
}

fn check_level(texts: &Texts, name: &str) -> Result<(), RpcError> {
    if LEVELS.contains(&name) { Ok(()) } else { Err(RpcError::invalid_params(texts.get("bad_level", &[("level", name)]))) }
}

/// The fields of `put` laid over `rule` (not its level, which changes the id).
fn lay_over(texts: &Texts, rule: &mut Rule, put: &RulePut) -> Result<(), RpcError> {
    if let Some(text) = &put.text {
        rule.text = clean(text);
    }
    if let Some(why) = &put.why {
        rule.why = clean(why);
    }
    if let Some(globs) = &put.applies_to {
        rule.applies_to = clean_globs(globs);
    }
    if let Some(kept) = &put.kept_by {
        if !KEPT_BY.contains(&kept.as_str()) {
            return Err(RpcError::invalid_params(texts.get("bad_kept_by", &[("kept_by", kept)])));
        }
        rule.kept_by = kept.clone();
    }
    if let Some(check) = &put.check {
        rule.check = clean(check);
    }
    if rule.kept_by.is_empty() {
        rule.kept_by = kept_by::TEXT.into();
    }
    if rule.kept_by == kept_by::CHECK && rule.check.is_empty() {
        return Err(RpcError::invalid_params(texts.get("check_needs_name", &[])));
    }
    if rule.kept_by != kept_by::CHECK {
        rule.check.clear();
    }
    if rule.text.is_empty() {
        return Err(RpcError::invalid_params(texts.get("rule_needs_text", &[])));
    }
    Ok(())
}

impl Shelf {
    fn book(&self, texts: &Texts, repo: &str) -> Result<&Book, RpcError> {
        self.books.get(repo).ok_or_else(|| RpcError::not_found(texts.get("no_charter", &[("repo", repo)])))
    }

    /// The standard charter once it is accepted, for an app's charter.
    pub fn standard_for(&self, repo: &str) -> Option<&Book> {
        if repo == STANDARD {
            return None;
        }
        self.books.get(STANDARD).filter(|book| !book.draft)
    }

    /// The standard's rules of these levels, as an app carries them.
    fn standard_rules(&self, repo: &str, levels: &[&str]) -> Vec<Rule> {
        self.standard_for(repo).map(|book| book.rules.iter().filter(|rule| levels.contains(&rule.level.as_str())).cloned().collect()).unwrap_or_default()
    }

    pub fn view(&self, repo: &str) -> Option<Charter> {
        let book = self.books.get(repo)?;
        Some(Charter {
            repo: book.repo.clone(),
            version: book.version.clone(),
            draft: book.draft,
            identity: book.identity.clone(),
            rules: book.rules.clone(),
            standard: self.standard_rules(repo, &[level::LAW, level::CONSTRAINT]),
            standard_version: self.standard_for(repo).map(|standard| standard.version.clone()).unwrap_or_default(),
            digest: book.digest(),
            updated_ms: book.updated_ms,
        })
    }

    /// A draft from a file's text. Every rule starts as a convention kept by text: the file's
    /// words do not say which rules are laws, the owner does.
    pub fn import(&mut self, texts: &Texts, params: &ImportParams, now: u64) -> Result<(), RpcError> {
        let repo = clean(&params.repo);
        if repo.is_empty() {
            return Err(RpcError::invalid_params(texts.get("needs_repo", &[])));
        }
        if self.books.get(&repo).is_some_and(|book| !book.draft) {
            return Err(refuse(texts.get("import_accepted", &[("repo", &repo)])));
        }
        let split = import::split(&params.text);
        if split.what.is_empty() && split.rules.is_empty() {
            return Err(RpcError::invalid_params(texts.get("import_empty", &[])));
        }
        let file = if params.file.trim().is_empty() { "AGENTS.md" } else { params.file.trim() };
        let mut book = Book::draft(&repo, now);
        book.identity.what = split.what;
        for piece in split.rules {
            let id = book.new_id(level::CONVENTION);
            book.rules.push(Rule {
                id,
                level: level::CONVENTION.into(),
                scope: repo.clone(),
                text: piece.text,
                kept_by: kept_by::TEXT.into(),
                source: format!("{file}:{}-{}", piece.start, piece.end),
                ..Rule::default()
            });
        }
        self.books.insert(repo, book);
        Ok(())
    }

    /// Add or change a rule or the identity. A draft takes any edit. An accepted charter takes a
    /// law's edit, and only with the law's id typed as `confirm`; everything else there goes
    /// through a proposal.
    pub fn rule_edit(&mut self, texts: &Texts, put: &RulePut, now: u64) -> Result<Option<Note>, RpcError> {
        let repo = clean(&put.repo);
        if repo.is_empty() {
            return Err(RpcError::invalid_params(texts.get("needs_repo", &[])));
        }
        if let Some(name) = &put.level {
            check_level(texts, name)?;
        }
        // A charter written from nothing starts with its first rule (the standard charter does).
        let mut book = match self.books.get(&repo) {
            Some(book) => book.clone(),
            None if put.id.is_empty() => Book::draft(&repo, now),
            None => return Err(RpcError::not_found(texts.get("no_charter", &[("repo", &repo)]))),
        };
        let mut note = None;
        if let Some(identity) = &put.identity {
            if !book.draft && put.confirm != CONFIRM_IDENTITY {
                return Err(refuse(texts.get("identity_needs_confirm", &[("word", CONFIRM_IDENTITY)])));
            }
            let identity = Identity { what: clean(&identity.what), who_for: clean(&identity.who_for), never: clean_globs(&identity.never) };
            if !book.draft && identity != book.identity {
                note = Some(Note { by: "owner".into(), old: book.identity.what.clone(), new: identity.what.clone(), ..Note::default() });
            }
            book.identity = identity;
        }
        let touches_rule = !put.id.is_empty() || put.text.is_some() || put.identity.is_none();
        if touches_rule {
            note = self.edit_rule(texts, &mut book, put)?.or(note);
        }
        if note.is_some() {
            book.bump();
            let version = book.version.clone();
            if let Some(note) = &note
                && let Some(at) = book.find(&note.rule)
            {
                book.rules[at].since = version;
                book.rules[at].amended_by.clear();
            }
        }
        book.sort();
        book.updated_ms = now;
        self.books.insert(repo, book);
        Ok(note)
    }

    /// The rule part of [`Shelf::rule_edit`]. The note is for an accepted charter only.
    fn edit_rule(&self, texts: &Texts, book: &mut Book, put: &RulePut) -> Result<Option<Note>, RpcError> {
        if put.id.is_empty() {
            let level_name = put.level.clone().unwrap_or_else(|| level::CONVENTION.into());
            if !book.draft {
                if level_name != level::LAW {
                    return Err(refuse(texts.get("accepted_needs_proposal", &[("level", &level_name)])));
                }
                let coming = book.clone().new_id(level::LAW);
                if put.confirm != coming {
                    return Err(refuse(texts.get("new_law_needs_confirm", &[("id", &coming)])));
                }
            }
            let mut rule = Rule { level: level_name.clone(), scope: book.repo.clone(), ..Rule::default() };
            lay_over(texts, &mut rule, put)?;
            rule.id = book.new_id(&level_name);
            let note = (!book.draft).then(|| Note { by: "owner".into(), rule: rule.id.clone(), level: level_name, new: rule.text.clone(), ..Note::default() });
            book.rules.push(rule);
            return Ok(note);
        }
        let at = book.find(&put.id).ok_or_else(|| RpcError::not_found(texts.get("no_rule", &[("id", &put.id), ("repo", &book.repo)])))?;
        let was = book.rules[at].clone();
        if !book.draft {
            if was.level != level::LAW {
                return Err(refuse(texts.get("accepted_needs_proposal", &[("level", &was.level)])));
            }
            if put.confirm != was.id {
                return Err(refuse(texts.get("law_needs_confirm", &[("id", &was.id)])));
            }
            // A law with its text taken away is retired: the one way a law leaves a charter.
            if put.text.as_deref().is_some_and(|text| text.trim().is_empty()) {
                book.rules.remove(at);
                return Ok(Some(Note { by: "owner".into(), rule: was.id, level: was.level, old: was.text, ..Note::default() }));
            }
        }
        let mut rule = was.clone();
        lay_over(texts, &mut rule, put)?;
        if let Some(level_name) = &put.level
            && *level_name != was.level
        {
            // Another level is another id; the old id is retired and never given again.
            rule.level = level_name.clone();
            rule.id = book.new_id(level_name);
        }
        let changed = rule != was;
        let note = (!book.draft && changed).then(|| Note { by: "owner".into(), rule: rule.id.clone(), level: rule.level.clone(), old: was.text.clone(), new: rule.text.clone(), ..Note::default() });
        book.rules[at] = rule;
        Ok(note)
    }

    /// Retire a rule of a draft.
    pub fn rule_delete(&mut self, texts: &Texts, repo: &str, id: &str, now: u64) -> Result<(), RpcError> {
        let mut book = self.book(texts, repo)?.clone();
        if !book.draft {
            return Err(refuse(texts.get("delete_accepted", &[])));
        }
        let at = book.find(id).ok_or_else(|| RpcError::not_found(texts.get("no_rule", &[("id", id), ("repo", repo)])))?;
        book.rules.remove(at);
        book.updated_ms = now;
        self.books.insert(repo.to_owned(), book);
        Ok(())
    }

    /// The draft becomes version 1.0.
    pub fn accept(&mut self, texts: &Texts, repo: &str, now: u64) -> Result<Note, RpcError> {
        let mut book = self.book(texts, repo)?.clone();
        if !book.draft {
            return Err(refuse(texts.get("already_accepted", &[("version", &book.version)])));
        }
        if book.rules.is_empty() {
            return Err(refuse(texts.get("accept_empty", &[])));
        }
        book.draft = false;
        book.version = FIRST_VERSION.into();
        for rule in &mut book.rules {
            rule.since = FIRST_VERSION.into();
            rule.amended_by.clear();
        }
        book.sort();
        book.updated_ms = now;
        self.books.insert(repo.to_owned(), book);
        Ok(Note { by: "accept".into(), ..Note::default() })
    }

    /// What a change would be: old and new text, what it needs, the rules it may conflict with.
    /// Nothing is changed.
    pub fn propose(&self, texts: &Texts, params: &ProposeParams) -> Result<Proposal, RpcError> {
        let book = self.book(texts, &params.repo)?;
        if book.draft {
            return Err(refuse(texts.get("propose_draft", &[])));
        }
        let new = clean(&params.text);
        let (level_name, old, own_globs) = if params.id.is_empty() {
            check_level(texts, &params.level)?;
            if new.is_empty() {
                return Err(RpcError::invalid_params(texts.get("rule_needs_text", &[])));
            }
            (params.level.clone(), String::new(), Vec::new())
        } else {
            let at = book.find(&params.id).ok_or_else(|| RpcError::not_found(texts.get("no_rule", &[("id", &params.id), ("repo", &params.repo)])))?;
            let rule = &book.rules[at];
            (rule.level.clone(), rule.text.clone(), rule.applies_to.clone())
        };
        if new == old {
            return Err(RpcError::invalid_params(texts.get("propose_same", &[])));
        }
        let needs = match level_name.as_str() {
            level::LAW => "law",
            level::CONSTRAINT => "major",
            _ => "minor",
        };
        // Found by code alone: the other rules that name paths this rule's paths can share. Whether
        // two such rules really disagree is a question for the model, asked by the caller.
        let mut conflicts = Vec::new();
        if !own_globs.is_empty() {
            let standard = self.standard_rules(&params.repo, &LEVELS);
            for rule in standard.iter().chain(book.rules.iter()) {
                let itself = rule.scope == book.repo && rule.id == params.id;
                if !itself && !rule.applies_to.is_empty() && glob::may_overlap(&own_globs, &rule.applies_to) {
                    conflicts.push(shown_id(rule, &book.repo));
                }
            }
        }
        Ok(Proposal {
            repo: params.repo.clone(),
            id: params.id.clone(),
            level: level_name,
            old,
            digest: proposal_digest(&params.repo, &params.id, &new),
            new,
            why: clean(&params.why),
            needs: needs.into(),
            conflicts,
        })
    }

    /// Apply an approved proposal. The approval was given to one wording: the digest must be that
    /// wording's, and the rule must still read as it did when the change was proposed.
    pub fn amend(&mut self, texts: &Texts, params: &AmendParams, now: u64) -> Result<Note, RpcError> {
        let proposal = &params.proposal;
        let mut book = self.book(texts, &proposal.repo)?.clone();
        if book.draft {
            return Err(refuse(texts.get("propose_draft", &[])));
        }
        if params.request.trim().is_empty() {
            return Err(RpcError::invalid_params(texts.get("amend_needs_request", &[])));
        }
        let digest = proposal_digest(&proposal.repo, &proposal.id, &proposal.new);
        if params.digest != digest || proposal.digest != digest {
            return Err(refuse(texts.get("amend_digest", &[])));
        }
        let mut note = Note { by: "amend".into(), request: params.request.clone(), old: proposal.old.clone(), new: proposal.new.clone(), ..Note::default() };
        if proposal.id.is_empty() {
            check_level(texts, &proposal.level)?;
            if proposal.level == level::LAW {
                return Err(refuse(texts.get("amend_law", &[])));
            }
            if proposal.new.is_empty() {
                return Err(RpcError::invalid_params(texts.get("rule_needs_text", &[])));
            }
            let id = book.new_id(&proposal.level);
            book.rules.push(Rule {
                id: id.clone(),
                level: proposal.level.clone(),
                scope: book.repo.clone(),
                text: proposal.new.clone(),
                why: proposal.why.clone(),
                kept_by: kept_by::TEXT.into(),
                ..Rule::default()
            });
            note.rule = id;
            note.level = proposal.level.clone();
        } else {
            let at = book.find(&proposal.id).ok_or_else(|| RpcError::not_found(texts.get("no_rule", &[("id", &proposal.id), ("repo", &proposal.repo)])))?;
            if book.rules[at].level == level::LAW {
                return Err(refuse(texts.get("amend_law", &[])));
            }
            if book.rules[at].text != proposal.old {
                return Err(refuse(texts.get("amend_stale", &[("id", &proposal.id)])));
            }
            note.rule = proposal.id.clone();
            note.level = book.rules[at].level.clone();
            if proposal.new.is_empty() {
                book.rules.remove(at);
            } else {
                let rule = &mut book.rules[at];
                rule.text = proposal.new.clone();
                if !proposal.why.is_empty() {
                    rule.why = proposal.why.clone();
                }
            }
        }
        book.bump();
        let version = book.version.clone();
        if let Some(at) = book.find(&note.rule) {
            book.rules[at].since = version;
            book.rules[at].amended_by = params.request.clone();
        }
        book.sort();
        book.updated_ms = now;
        self.books.insert(proposal.repo.clone(), book);
        Ok(note)
    }

    /// The rules that apply to these paths: the standard's first, then the app's. A rule that
    /// names no paths applies everywhere. No levels asked: laws and constraints.
    pub fn rules_for(&self, params: &RulesForParams) -> Vec<Rule> {
        let levels: Vec<&str> = if params.levels.is_empty() { vec![level::LAW, level::CONSTRAINT] } else { params.levels.iter().map(String::as_str).collect() };
        let own: Vec<Rule> = self.books.get(&params.repo).filter(|book| !book.draft).map(|book| book.rules.clone()).unwrap_or_default();
        self.standard_rules(&params.repo, &LEVELS)
            .into_iter()
            .chain(own)
            .filter(|rule| levels.contains(&rule.level.as_str()))
            .filter(|rule| rule.applies_to.is_empty() || params.paths.iter().any(|path| glob::applies(&rule.applies_to, path)))
            .collect()
    }

    /// Every rule a check may keep, for the repository: the standard's, then its own.
    pub fn rules_in_force(&self, repo: &str) -> Vec<Rule> {
        let own: Vec<Rule> = self.books.get(repo).filter(|book| !book.draft).map(|book| book.rules.clone()).unwrap_or_default();
        self.standard_rules(repo, &LEVELS).into_iter().chain(own).collect()
    }
}
