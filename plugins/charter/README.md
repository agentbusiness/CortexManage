# Charter (`cc-charter`, contract `charter@1`)

Every app's design specification (`docs/specs/03-CHARTER.md`): what the app is, its **laws**
(changed only by the owner's hand), its **constraints** (changed by a major request) and its
**conventions** (changed by a minor request). The charter is CortexManage's record; a repository
only holds the print, `AGENTS.md`. One charter, with the repository id `standard`, is the shared
standard: its laws and constraints go with every app's charter.

## What it does

| Method | Does |
|---|---|
| `charter.get {repo}` | The charter. For an app it also carries the accepted standard's laws and constraints in `standard`. |
| `charter.list {}` | `{charters: [{repo, version, rules, draft}]}`; `rules` is the number of rules in force. |
| `charter.import {repo, file, text}` | A draft from an `AGENTS.md`: one rule per paragraph or list item under each `#` heading, `source` = `file:line-line`. The first paragraph before any heading becomes `identity.what`. Importing again replaces a draft; an accepted charter refuses. |
| `charter.rule.edit` | Add or change a rule or the identity of a draft. On an accepted charter: a law, with `confirm` equal to its id. |
| `charter.rule.delete {repo, id}` | Retire a rule of a draft. |
| `charter.accept {repo}` | The draft becomes version `1.0`. |
| `charter.propose` | Old and new text, what the change needs, the rules it may conflict with, the digest. Changes nothing. |
| `charter.amend` | Apply an approved proposal when the digest is that wording's. |
| `charter.print {repo}` | The `AGENTS.md` text. |
| `charter.drift {repo, text}` | A file compared with the print. |
| `charter.rules_for {repo, paths, levels}` | The rules that apply to these paths. |
| `charter.check {repo, diff, files, standard_change}` | The checks kept by code, over a diff. |

It publishes `charter.changed {repo, version}` after every change.

### Importing

The split is by structure only. A list item nested under another is part of that item; a fenced
code block belongs to the text above it; dividers and one-line HTML comments are skipped. No code
reads what a sentence means, so **every imported rule starts as a convention kept by text**, and
the owner sets the levels in the draft.

### Ids

`L1…`, `C1…`, `V1…`, one counter per level and charter. A number is given once: a retired rule's
id never comes to mean another rule. Changing a rule's level gives it a new id of that level and
retires the old one. (Importing again starts the draft, and its counters, over.)

### What may change an accepted charter

- **A law**: `charter.rule.edit` with `confirm` equal to the law's id, which the owner typed. A new
  law takes the id it will get as `confirm` (the refusal names it). A law whose text is set to
  nothing is retired. The identity takes `confirm: "identity"`.
- **A constraint or convention**: `charter.propose`, then `charter.amend` when the request that
  carries it lands. A direct edit is refused with a sentence that says so. A proposal with an
  empty text for an existing rule proposes retiring it.
- `needs` is `law` (not possible by request; `amend` refuses it), `major` (constraint) or `minor`
  (convention).
- **The digest** is `sha256(repo + "\n" + id + "\n" + new text)`, in hex. `amend` refuses when the
  digest it is given, or the proposal's own, is not that; and when the rule no longer reads as the
  proposal's `old`.

### Versions

A draft is `0.0`; accepting makes it `1.0`. **Every change to an accepted charter adds one to the
second number** (`1.0` → `1.1` → `1.2`), whether it is an amendment to a constraint, to a
convention, or a law edited by hand. The changed rule's `since` becomes the new version and its
`amended_by` the request (empty for a change by hand). The first number is not changed by this
plugin.

### Conflicts

`conflicts` lists the other rules (the standard's as `S-L1`) that **name paths which can share a
file with the paths this rule names**. It is found by code alone, from `applies_to`: two globs can
share a file when they agree before their first wildcard and after their last. Two rules are not
compared when either names no paths: a rule without paths applies everywhere and would be listed
against every other rule. Whether two listed rules really disagree is a model question for the
caller; nothing here reads the rules' words.

### The print

```
<!-- Printed from the charter of cortexterm by CortexManage, version 1.4. … Standard charter 1.2. charter sha256:… text sha256:… -->
# Laws
## Laws of the shared standard        - S-L1. …
## Laws of this app                   - L1. …
# Constraints                         (the same two parts)
# What this app is                    what, who for, what it must never become
# Conventions
```

Each rule is one line, numbered with its id; the standard's rules are `S-L1`, `S-C2`. `why` is for
people and is not printed. CortexCode gives its model the file's first 6000 characters, cut at `#`
headings (`chatflow/backend/src/context.rs`, `fit_sections`), so the laws and constraints come
first and `head_chars` says at which character they end, the banner counted. Over 6000, the model
does not see them all; the Charter view says so. `digest` is the sha256 of the text without its
banner line, and the banner repeats it. A draft prints too (version `draft`), for the comparison
with the old file before accepting.

### Drift

The banner line is not compared. `same`: the file is the print. `edited`: it carries a banner and
its text is not the print's. `not_printed`: it has no banner. A fourth answer the contract does not
name, `outdated`: the file's text still has the hash its own banner states, so nobody edited it; the
charter moved on and the file needs printing again. `added` are the lines only in the file,
`removed` the lines only in the print.

### Rules for paths

`rules_for` gives the accepted standard's rules and the app's accepted rules whose `applies_to`
matches any of the paths; a rule without paths applies everywhere. No `levels`: laws and
constraints. Globs: `*` and `?` inside one folder name, `**` for any number of folders; a glob
without `/` names a file in any folder (`*.tsx`); one ending in `/` everything under that folder.

### Checks

Plain structural code over the file list and the lines a unified diff **adds**. What each looks
for, and the sentence a person reads, is in `backend/data/checks.toml`.

| Check | Finds |
|---|---|
| `no-record-files` | A new `.json`, `.sqlite` or `.db` file under `plugins/<id>/…/backend/` or `…/data/` that is not `package.json`, a tsconfig, a manifest, a schema or a lock |
| `no-tests` | An added `.rs` line starting with `#[cfg(test)]` or `#[test]`; a file in a `tests` folder; a `*.test.*` or `*.spec.*` file |
| `no-native-controls` | `<select`, `type="checkbox"`, `type="radio"` in added lines of `.tsx`, `.jsx`, `.html` |
| `theme-untouched` | Any changed path under `plugins/theme-cortex/`, unless `standard_change` |
| `no-secrets` | A private key header, a token with a known start, or a long literal given to a secret-sounding name (the shapes are in the data file); the value is never repeated |
| `agents-file-untouched` | `AGENTS.md` or `standard.lock` changed, unless `standard_change` |
| `contract-calls-only` | **Not run.** A diff does not hold the plugin's `plugin.toml`, and code cannot tell a method name in a line from other text soundly |

`ran` lists only the checks that could decide. A check that needs a file's added lines did not run
when that file is named in `files` but missing from the diff (and `no-record-files` cannot say
whether such a file is new). Findings a check did make are still returned. `ok` is "no findings":
read it together with `ran`. A finding's `rule` is the rule that says it is kept by that check
(`kept_by: check`, `check: <name>`), when there is one.

## Records

In the app's graph (WORKSPACE layer). Nothing is kept in files.

```
(:Charter {key: repo, repo, version, draft, what, who_for, never[], next_law, next_constraint,
           next_convention, digest, updated_ms})-[:ON]->(:Machine)
    -[:HAS_RULE]->(:CharterRule {key: "repo/L1", repo, id, level, text, why, applies_to[],
                                 kept_by, check, since, amended_by, source})
    -[:HAS_VERSION]->(:CharterVersion {key: "repo@1.2", repo, version, digest, ms, by, request,
                                       rule, level, old_text, new_text})-[:CHANGED]->(:CharterRule)
```

- A rule that leaves the charter is **retired**, never deleted.
- **History is kept as version nodes**, not temporal properties: every accepted version has one
  `CharterVersion` node saying who made it (`accept`, `owner`, `amend`), the request, the rule it
  changed and the rule's old and new text. `CharterRule` holds the text in force.

## Probe

On charters of its own that are never written to the graph: writes and accepts a one-law standard,
imports a small file, sets levels, accepts, is refused a direct edit of a constraint, proposes,
is refused an amendment with another digest, amends, prints, compares files with the print, asks
the rules for two paths, and runs the checks over a small diff.

## Data

- `backend/data/texts.toml`: the refusals and the fixed lines of the printed file.
- `backend/data/checks.toml`: the checks.

Both are compiled in; a copy in the plugin's live folder is read on every use.

## UI

`charter.page` (slot `page.charter@1`): see `ui/page/README.md`.
