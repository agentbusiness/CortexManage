//! The charter service: the charters in memory, every change written to the graph, and the
//! contract's methods.
//!
//! What it keeps lives in the app's graph (WORKSPACE layer), read back at the first request:
//!
//! ```text
//! (:Charter {key: repo, repo, version, draft, what, who_for, never[], next_law, next_constraint,
//!            next_convention, digest, updated_ms})-[:ON]->(:Machine)
//!     -[:HAS_RULE]->(:CharterRule {key: "repo/L1", repo, id, level, text, why, applies_to[],
//!                                  kept_by, check, since, amended_by, source})
//!         a rule in force; retired (never deleted) when it leaves the charter
//!     -[:HAS_VERSION]->(:CharterVersion {key: "repo@1.2", repo, version, digest, ms, by, request,
//!                                        rule, level, old_text, new_text})
//!         one node per accepted version: who made it and the one change it made
//!         -[:CHANGED]->(:CharterRule)
//! ```

use crate::book::{Book, Note, STANDARD, Shelf};
use crate::checks;
use crate::data::Data;
use crate::graph::{self, Graph};
use crate::print;
use cc_protocol::contracts::charter::{
    AmendParams, Charter, CheckParams, DriftParams, Identity, ImportParams, ProposeParams, Rule, RulePut, RulesForParams, kept_by, level, methods, topics,
};
use cc_protocol::contracts::lifecycle::{ProbeCheck, ProbeReport};
use cc_protocol::contracts::memory::{GraphEdge, GraphNode};
use cc_protocol::{Ctx, RpcError, parse_params, to_value};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Instant;
use tokio::sync::OnceCell;

const CHARTER: &str = "Charter";
const RULE: &str = "CharterRule";
const VERSION: &str = "CharterVersion";
/// Nodes and edges in one put (the memory plugin takes 5000 at most).
const BATCH: usize = 1000;
/// Rows read at most per label when the charters are read back.
const READ_LIMIT: u32 = 5000;

fn now_ms() -> u64 {
    cc_protocol::contracts::now_ms()
}

fn rule_key(repo: &str, id: &str) -> String {
    format!("{repo}/{id}")
}

/// A list property: the graph keeps lists of one scalar type, and an empty list has none, so an
/// empty list is written as no property.
fn list(items: &[String]) -> Value {
    if items.is_empty() { Value::Null } else { json!(items) }
}

fn rule_node(rule: &Rule, repo: &str) -> GraphNode {
    graph::node(
        RULE,
        &rule_key(repo, &rule.id),
        graph::props([
            ("repo", json!(repo)),
            ("id", json!(rule.id)),
            ("level", json!(rule.level)),
            ("text", json!(rule.text)),
            ("why", json!(rule.why)),
            ("applies_to", list(&rule.applies_to)),
            ("kept_by", json!(rule.kept_by)),
            ("check", json!(rule.check)),
            ("since", json!(rule.since)),
            ("amended_by", json!(rule.amended_by)),
            ("source", json!(rule.source)),
        ]),
    )
}

fn rule_from(props: &Map<String, Value>) -> Rule {
    Rule {
        id: graph::text(props, "id"),
        level: graph::text(props, "level"),
        scope: graph::text(props, "repo"),
        text: graph::text(props, "text"),
        why: graph::text(props, "why"),
        applies_to: graph::texts(props, "applies_to"),
        kept_by: graph::text(props, "kept_by"),
        check: graph::text(props, "check"),
        since: graph::text(props, "since"),
        amended_by: graph::text(props, "amended_by"),
        source: graph::text(props, "source"),
    }
}

#[derive(Deserialize)]
struct RepoParams {
    repo: String,
}

#[derive(Deserialize)]
struct RuleRef {
    repo: String,
    id: String,
}

pub struct Charters {
    shelf: Mutex<Shelf>,
    data: Data,
    graph: Graph,
    ctx: OnceLock<Ctx>,
    /// Set once the charters are read back from the graph.
    loaded: OnceCell<()>,
}

impl Charters {
    pub fn new(live_dir: Option<PathBuf>) -> Arc<Charters> {
        Arc::new(Charters { shelf: Mutex::new(Shelf::default()), data: Data::new(live_dir), graph: Graph::new(), ctx: OnceLock::new(), loaded: OnceCell::new() })
    }

    /// The frame connection arrives with the first call; the graph's writer starts with it.
    pub fn attach(&self, ctx: &Ctx) {
        if self.ctx.set(ctx.clone()).is_ok() {
            self.graph.start(graph::ctx_caller(ctx));
        }
    }

    /// Wait until every write asked for so far is in the graph.
    pub async fn settle(&self) {
        if self.ctx.get().is_some() {
            self.graph.flush().await;
        }
    }

    fn shelf(&self) -> MutexGuard<'_, Shelf> {
        self.shelf.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Read the charters back, once. A failed read is tried again by the next request.
    async fn ready(&self) -> Result<(), RpcError> {
        self.loaded.get_or_try_init(|| self.load()).await.map(|_| ())
    }

    async fn load(&self) -> Result<(), RpcError> {
        let charters = self.graph.nodes(&format!("MATCH (c:{CHARTER}) WHERE c.ended_ms IS NULL RETURN c"), Map::new(), READ_LIMIT).await?;
        let rules = self.graph.nodes(&format!("MATCH (r:{RULE}) WHERE r.ended_ms IS NULL RETURN r"), Map::new(), READ_LIMIT).await?;
        let mut shelf = Shelf::default();
        for props in &charters {
            let repo = graph::text(props, "repo");
            if repo.is_empty() {
                continue;
            }
            let next = |name: &str| graph::number(props, name).unwrap_or(1).max(1) as u32;
            shelf.books.insert(
                repo.clone(),
                Book {
                    repo,
                    version: graph::text(props, "version"),
                    draft: graph::flag(props, "draft"),
                    identity: Identity { what: graph::text(props, "what"), who_for: graph::text(props, "who_for"), never: graph::texts(props, "never") },
                    rules: Vec::new(),
                    next: [next("next_law"), next("next_constraint"), next("next_convention")],
                    updated_ms: graph::number(props, "updated_ms").unwrap_or_default(),
                },
            );
        }
        for props in &rules {
            let rule = rule_from(props);
            if let Some(book) = shelf.books.get_mut(&rule.scope)
                && !rule.id.is_empty()
            {
                book.rules.push(rule);
            }
        }
        for book in shelf.books.values_mut() {
            book.rules.sort_by_key(|rule| (rule.level != level::LAW, rule.level != level::CONSTRAINT, rule.id.get(1..).and_then(|n| n.parse::<u32>().ok()).unwrap_or(0)));
        }
        tracing::info!(charters = shelf.books.len(), "the charters were read from the graph");
        *self.shelf() = shelf;
        Ok(())
    }

    /// Write what changed between two states of one charter: the charter's node, the rules that
    /// are new or differ, the rules that left (retired), and the version's node.
    fn persist(&self, before: Option<&Book>, after: &Book, note: Option<&Note>) {
        let repo = after.repo.as_str();
        let charter = graph::node(
            CHARTER,
            repo,
            graph::props([
                ("repo", json!(repo)),
                ("version", json!(after.version)),
                ("draft", json!(after.draft)),
                ("what", json!(after.identity.what)),
                ("who_for", json!(after.identity.who_for)),
                ("never", list(&after.identity.never)),
                ("next_law", json!(after.next[0])),
                ("next_constraint", json!(after.next[1])),
                ("next_convention", json!(after.next[2])),
                ("digest", json!(after.digest())),
                ("updated_ms", json!(after.updated_ms)),
            ]),
        );
        let mut nodes = vec![charter];
        let mut edges: Vec<GraphEdge> = vec![graph::edge((CHARTER, repo), "ON", graph::machine())];
        let was = |id: &str| before.and_then(|book| book.rules.iter().find(|rule| rule.id == id));
        for rule in after.rules.iter().filter(|rule| was(&rule.id) != Some(*rule)) {
            nodes.push(rule_node(rule, repo));
            edges.push(graph::edge((CHARTER, repo), "HAS_RULE", graph::at(RULE, &rule_key(repo, &rule.id))));
            if nodes.len() >= BATCH {
                self.graph.put(std::mem::take(&mut nodes), std::mem::take(&mut edges));
            }
        }
        if let Some(note) = note.filter(|_| !after.draft) {
            let key = format!("{repo}@{}", after.version);
            nodes.push(graph::node(
                VERSION,
                &key,
                graph::props([
                    ("repo", json!(repo)),
                    ("version", json!(after.version)),
                    ("digest", json!(after.digest())),
                    ("ms", json!(after.updated_ms)),
                    ("by", json!(note.by)),
                    ("request", json!(note.request)),
                    ("rule", json!(note.rule)),
                    ("level", json!(note.level)),
                    ("old_text", json!(note.old)),
                    ("new_text", json!(note.new)),
                ]),
            ));
            edges.push(graph::edge((CHARTER, repo), "HAS_VERSION", graph::at(VERSION, &key)));
            // The rule the version changed, when it is still in force (a retired rule keeps its node).
            if after.rules.iter().any(|rule| rule.id == note.rule) {
                edges.push(graph::edge((VERSION, &key), "CHANGED", graph::at(RULE, &rule_key(repo, &note.rule))));
            }
        }
        self.graph.put(nodes, edges);
        let gone: Vec<_> = before
            .map(|book| book.rules.iter().filter(|rule| !after.rules.iter().any(|kept| kept.id == rule.id)).map(|rule| graph::at(RULE, &rule_key(repo, &rule.id))).collect())
            .unwrap_or_default();
        self.graph.retire(gone, "left the charter");
    }

    /// One change to one charter: made in memory, written to the graph, announced.
    async fn change(&self, repo: &str, apply: impl FnOnce(&mut Shelf, &Data, u64) -> Result<Option<Note>, RpcError>) -> Result<Charter, RpcError> {
        self.ready().await?;
        let repo = repo.trim();
        let view = {
            let mut shelf = self.shelf();
            let before = shelf.books.get(repo).cloned();
            let note = apply(&mut shelf, &self.data, now_ms())?;
            let after = shelf.books.get(repo).ok_or_else(|| RpcError::internal("the charter is gone after its own change"))?;
            self.persist(before.as_ref(), after, note.as_ref());
            shelf.view(repo).ok_or_else(|| RpcError::internal("the charter is gone after its own change"))?
        };
        if let Some(ctx) = self.ctx.get() {
            ctx.publish(topics::CHANGED, json!({"repo": view.repo, "version": view.version}));
        }
        Ok(view)
    }

    fn charter(&self, repo: &str) -> Result<Charter, RpcError> {
        self.shelf().view(repo).ok_or_else(|| RpcError::not_found(self.data.texts.get("no_charter", &[("repo", repo)])))
    }

    fn printed(&self, repo: &str) -> Result<cc_protocol::contracts::charter::Printed, RpcError> {
        let shelf = self.shelf();
        let book = shelf.books.get(repo).ok_or_else(|| RpcError::not_found(self.data.texts.get("no_charter", &[("repo", repo)])))?;
        Ok(print::print(&self.data.texts, book, shelf.standard_for(repo)))
    }

    pub async fn handle(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        match method {
            methods::GET => {
                self.ready().await?;
                let params: RepoParams = parse_params(params)?;
                to_value(&self.charter(params.repo.trim())?)
            }
            methods::LIST => {
                self.ready().await?;
                let charters: Vec<Value> = self
                    .shelf()
                    .books
                    .values()
                    .map(|book| json!({"repo": book.repo, "version": book.version, "rules": book.rules.len(), "draft": book.draft}))
                    .collect();
                Ok(json!({"charters": charters}))
            }
            methods::IMPORT => {
                let params: ImportParams = parse_params(params)?;
                to_value(&self.change(&params.repo, |shelf, data, now| shelf.import(&data.texts, &params, now).map(|_| None)).await?)
            }
            methods::ACCEPT => {
                let params: RepoParams = parse_params(params)?;
                to_value(&self.change(&params.repo, |shelf, data, now| shelf.accept(&data.texts, params.repo.trim(), now).map(Some)).await?)
            }
            methods::RULE_EDIT => {
                let params: RulePut = parse_params(params)?;
                to_value(&self.change(&params.repo, |shelf, data, now| shelf.rule_edit(&data.texts, &params, now)).await?)
            }
            methods::RULE_DELETE => {
                let params: RuleRef = parse_params(params)?;
                to_value(&self.change(&params.repo, |shelf, data, now| shelf.rule_delete(&data.texts, params.repo.trim(), params.id.trim(), now).map(|_| None)).await?)
            }
            methods::PROPOSE => {
                self.ready().await?;
                let params: ProposeParams = parse_params(params)?;
                to_value(&self.shelf().propose(&self.data.texts, &params)?)
            }
            methods::AMEND => {
                let params: AmendParams = parse_params(params)?;
                to_value(&self.change(&params.proposal.repo, |shelf, data, now| shelf.amend(&data.texts, &params, now).map(Some)).await?)
            }
            methods::PRINT => {
                self.ready().await?;
                let params: RepoParams = parse_params(params)?;
                to_value(&self.printed(params.repo.trim())?)
            }
            methods::CHECK => {
                self.ready().await?;
                let params: CheckParams = parse_params(params)?;
                let data = self.data.checks().map_err(RpcError::internal)?;
                let rules = self.shelf().rules_in_force(&params.repo);
                to_value(&checks::run(&params, &rules, &data))
            }
            methods::RULES_FOR => {
                self.ready().await?;
                let params: RulesForParams = parse_params(params)?;
                Ok(json!({"rules": self.shelf().rules_for(&params)}))
            }
            methods::DRIFT => {
                self.ready().await?;
                let params: DriftParams = parse_params(params)?;
                to_value(&print::drift(&self.printed(params.repo.trim())?, &params.text))
            }
            other => Err(RpcError::unknown_method(other)),
        }
    }

    /// The plugin's own test, run for real on charters of its own that are never written: import a
    /// small file, set levels, accept, propose, amend, print, compare, check a small diff.
    pub fn probe(&self) -> ProbeReport {
        let started = Instant::now();
        let mut checks = Vec::new();
        let mut step = |name: &str, outcome: Result<String, String>| {
            let (ok, detail) = match outcome {
                Ok(detail) => (true, detail),
                Err(detail) => (false, detail),
            };
            checks.push(ProbeCheck { name: name.into(), ok, detail, ms: started.elapsed().as_millis() as u64 });
            ok
        };
        let texts = &self.data.texts;
        let said = |error: RpcError| error.message;
        let mut shelf = Shelf::default();
        let repo = "probe";

        // The standard: written from nothing, one law, accepted.
        let standard = shelf
            .rule_edit(texts, &RulePut { repo: STANDARD.into(), level: Some(level::LAW.into()), text: Some("The graph is the only record.".into()), ..RulePut::default() }, 1)
            .and_then(|_| shelf.accept(texts, STANDARD, 1))
            .map(|_| "one law, version 1.0".to_owned());
        step("the standard charter is written and accepted", standard.map_err(said));

        let file = "Probe is a small app for the plugin's own test.\n\n# Rules\n\n- Buttons are the theme's detents.\n- Tool descriptions are at most 100 characters.\n\n## Releases\n\nReleases are built on the owner's Mac.\n";
        let imported = shelf.import(texts, &ImportParams { repo: repo.into(), file: "AGENTS.md".into(), text: file.into() }, 2).map_err(said).and_then(|_| {
            let book = &shelf.books[repo];
            let right = book.draft
                && book.rules.len() == 3
                && book.identity.what.starts_with("Probe is")
                && book.rules.iter().all(|rule| rule.level == level::CONVENTION && rule.kept_by == kept_by::TEXT)
                && book.rules[2].source == "AGENTS.md:10-10";
            if right { Ok("3 rules, all conventions kept by text".to_owned()) } else { Err(format!("the draft is not what the file says: {:?}", book.rules)) }
        });
        if !step("a file is split into a draft by its structure", imported) {
            return ProbeReport::from_checks(checks);
        }

        // The owner sets levels: the third rule becomes a constraint (a new id, C1), the first a
        // rule kept by a check, on the UI's paths.
        let levelled = shelf
            .rule_edit(texts, &RulePut { repo: repo.into(), id: "V3".into(), level: Some(level::CONSTRAINT.into()), ..RulePut::default() }, 3)
            .and_then(|_| {
                shelf.rule_edit(
                    texts,
                    &RulePut {
                        repo: repo.into(),
                        id: "V1".into(),
                        kept_by: Some(kept_by::CHECK.into()),
                        check: Some(checks::AGENTS_FILE_UNTOUCHED.into()),
                        applies_to: Some(vec!["plugins/*/ui/**".into()]),
                        ..RulePut::default()
                    },
                    3,
                )
            })
            .map_err(said)
            .and_then(|_| {
                let ids: Vec<&str> = shelf.books[repo].rules.iter().map(|rule| rule.id.as_str()).collect();
                if ids == ["C1", "V1", "V2"] { Ok("ids C1, V1, V2".to_owned()) } else { Err(format!("ids are {ids:?}")) }
            });
        step("a rule's level is set in the draft and gives it an id of that level", levelled);

        let accepted = shelf.accept(texts, repo, 4).map_err(said).and_then(|_| {
            let view = shelf.view(repo).ok_or("no charter after accepting")?;
            if view.version == "1.0" && !view.draft && view.standard.len() == 1 { Ok("version 1.0 with the standard's law".to_owned()) } else { Err(format!("version {} draft {}", view.version, view.draft)) }
        });
        step("the draft is accepted as version 1.0", accepted);

        let direct = shelf.rule_edit(texts, &RulePut { repo: repo.into(), id: "C1".into(), text: Some("Releases are built anywhere.".into()), ..RulePut::default() }, 5);
        step("a constraint of an accepted charter refuses a direct edit", match direct {
            Err(error) => Ok(error.message),
            Ok(_) => Err("the edit went through".into()),
        });

        let new_text = "Releases are built on the owner's Mac or on the runner.";
        let proposal = shelf.propose(texts, &ProposeParams { repo: repo.into(), id: "C1".into(), text: new_text.into(), why: "The runner builds too.".into(), ..ProposeParams::default() });
        let proposed = match &proposal {
            Ok(proposal) if proposal.needs == "major" && proposal.digest.len() == 64 && proposal.old.starts_with("Releases are built") => Ok(format!("needs {}", proposal.needs)),
            Ok(proposal) => Err(format!("needs {} digest {}", proposal.needs, proposal.digest)),
            Err(error) => Err(error.message.clone()),
        };
        step("a change to a constraint is proposed as a major change", proposed);

        if let Ok(proposal) = proposal {
            let wrong = shelf.amend(texts, &AmendParams { proposal: proposal.clone(), digest: "0".repeat(64), request: "probe-request".into() }, 6);
            step("an amendment with another digest is refused", match wrong {
                Err(error) => Ok(error.message),
                Ok(_) => Err("it was applied".into()),
            });
            let amended = shelf.amend(texts, &AmendParams { digest: proposal.digest.clone(), proposal, request: "probe-request".into() }, 7).map_err(said).and_then(|_| {
                let book = &shelf.books[repo];
                let rule = book.rules.iter().find(|rule| rule.id == "C1").ok_or("C1 is gone")?;
                if book.version == "1.1" && rule.text == new_text && rule.since == "1.1" && rule.amended_by == "probe-request" { Ok("version 1.1".to_owned()) } else { Err(format!("version {} text {}", book.version, rule.text)) }
            });
            step("the approved amendment is applied and the version goes up", amended);
        }

        let printed = print::print(texts, &shelf.books[repo], shelf.standard_for(repo));
        let laws_first = printed.text.find("S-L1.").zip(printed.text.find("C1.")).zip(printed.text.find("V1.")).is_some_and(|((law, constraint), convention)| law < constraint && constraint < convention);
        let print_ok = printed.text.starts_with("<!--") && laws_first && printed.head_chars <= print::HEAD_LIMIT && printed.digest.len() == 64;
        step("the file is printed, laws first, inside the model's first characters", if print_ok { Ok(format!("{} characters to the end of the constraints", printed.head_chars)) } else { Err(printed.text.clone()) });

        let same = print::drift(&printed, &printed.text);
        let edited = print::drift(&printed, &format!("{}\n- V9. A line somebody added.\n", printed.text));
        let bare = print::drift(&printed, file);
        let drift_ok = same.state == "same" && edited.state == "edited" && edited.added.len() == 1 && bare.state == "not_printed";
        step("a file is compared with the print", if drift_ok { Ok("same, edited, not printed".to_owned()) } else { Err(format!("{} {} {}", same.state, edited.state, bare.state)) });

        let for_ui = shelf.rules_for(&RulesForParams { repo: repo.into(), paths: vec!["plugins/charter/ui/page/src/index.tsx".into()], levels: vec![level::CONVENTION.into()] });
        let for_backend = shelf.rules_for(&RulesForParams { repo: repo.into(), paths: vec!["plugins/charter/backend/src/main.rs".into()], levels: vec![level::CONVENTION.into()] });
        let paths_ok = for_ui.iter().any(|rule| rule.id == "V1") && !for_backend.iter().any(|rule| rule.id == "V1");
        step("a rule goes only to the paths it names", if paths_ok { Ok(format!("{} rules for the UI file, {} for the backend file", for_ui.len(), for_backend.len())) } else { Err("V1 went to the wrong paths".into()) });

        let diff = "diff --git a/AGENTS.md b/AGENTS.md\n--- a/AGENTS.md\n+++ b/AGENTS.md\n@@ -1,2 +1,3 @@\n # Notes\n+- A line somebody added.\n - Keep it simple.\n";
        let checked = self.data.checks().and_then(|data| {
            let answer = checks::run(&CheckParams { repo: repo.into(), diff: diff.into(), ..CheckParams::default() }, &shelf.rules_in_force(repo), &data);
            let found = answer.findings.len() == 1 && answer.findings[0].check == checks::AGENTS_FILE_UNTOUCHED && answer.findings[0].rule == "V1";
            // Only the check a rule asks for runs, and the one for secrets.
            if !answer.ok && found && answer.ran == [checks::NO_SECRETS, checks::AGENTS_FILE_UNTOUCHED] { Ok(answer.findings[0].text.clone()) } else { Err(format!("ran {:?}, found {:?}", answer.ran, answer.findings)) }
        });
        step("the checks find an edit of the printed file in a small diff", checked);

        ProbeReport::from_checks(checks)
    }
}
