# CortexManage: notes for anyone working on this repository

CortexManage manages the development of software, new or existing: any repository a person adds.
People use it for their own repositories; the Cortex apps are only the first it manages. A request
is a chat session under its repository; the app rates it, plans it, holds it, and drives a running
CortexCode (the runner) to build it, watching every step. It is a Rust frame that runs everything as plugins and an Electron
UI, like its siblings. Design: `docs/specs/README.md`. The window: `docs/brief/cortexmanage-brief.html`.

This file is written by hand until the charter plugin prints it (`docs/specs/03-CHARTER.md`).

## Laws

1. **The graph is the only record.** Everything the app keeps is nodes and edges through the memory
   plugin (`memory.graph.*`, streams, settings). No JSON, TOML or SQLite files as records; no maps
   or JSON text in a property. Things are retired, never deleted. A session's journal is the one
   exception. Bytes (a screenshot) are not records.
2. **CortexManage never edits code.** Building is done by a CortexCode, in its own clones. Nothing
   in this app writes into a managed repository's files.
3. **No gate rests on a model's opinion.** Done means the checks, the builds and the walks passed.
   A model's yes or no must name a line that code then finds.
4. **The model that reads a request has no tool that writes.** Only planning has tools, and only
   for reading.
5. **Plugins talk only through contract methods.** No plugin reads another plugin's records.
6. **The mesh standard is carried byte for byte.** `protocol/` (the shared files), the mesh plugin's
   core, `sandbox-kit/`, `service-kit/`, `model-kit/`, `stats-kit/`, `shared-models/`, the memory plugin's graph
   files and `theme-cortex` are the same as in the other Cortex apps. A change to any of them is a
   standard change, made in every app in the same piece of work (`docs/specs/04-STANDARD.md`).
7. **Secrets live in the Keychain or the Secret Service**, never in a file or the graph. A model
   never holds a key.
8. **Local only.** No cloud; an app's errors and metrics go only to a CortexManage on the
   person's own mesh (`docs/specs/07-REPORTS.md`); a model runs only on a GPU of the mesh.
9. **Nothing is recognised by guessing.** No name matching, no heuristics, no regular expressions
   and no patterns decide what a thing is or what it belongs to. A thing is what it declares in a
   fixed place (a field, a file at a fixed path), or what a person set. What is not declared is not
   there: it is not inferred and not shown.
10. **Nothing of one owner is built in.** No repository, app name, path, member list or rule of any
    one person's software is in the code or shown by default. Such a thing appears only when a
    managed repository declares it.

## Constraints

1. Nothing is built, run, committed or pushed in a managed repository unless the owner asked. A
   package that passed its hold in CortexManage is the owner's ask for that package: its checks,
   its proving build, its commits on its own branch, its landing, and its release by the train.
2. The manager's model is the shared office Gemma (`office` tag); builders use the Qwen (`coding`).
   Claimed in the background so a person's turn in another app always goes first. Prompts are
   written for the small model: short, one decision per question.
3. The ladder is steer once, restart once, then a person. A steer is a fixed sentence that ends
   with the next step.
4. Walks are the manager's records. They are never sent to a builder and never written into a
   managed repository.
5. The frame is a small core. New work goes into plugins.
6. No tests, harnesses or check scripts in this repository; a plugin's `probe` exercises it for
   real, and a change is checked by using the real app.
7. Releases are built on the computer that does the coding, never on GitHub; versions are three
   numbers, each 0 to 99.
8. The window uses the theme's controls only: no native select, checkbox or radio; no radius, no
   shadow, no gradient; the tokens of `theme-cortex`.

## Conventions

- Keep it simple: where two mechanisms could do a job, there is one. Reuse what the Cortex apps
  already have before adding anything.
- Code decides what code can decide: risk floors, scope, hashes and the watch are counting. Meaning
  is a model question in the chat flow, never a keyword or a pattern (law 9).
- Model-facing text follows CortexCode's lens: short numbered steps, one reason per choice, a
  neutral example, the evidence right before the decision, the last sentence is the next step,
  never prose and a decision in one answer. Decisions are schema-bound and greedy.
- Prompts, schemas and texts are data in a plugin's `data` folder, edited live; code carries none.
- Texts a person reads are plain English, and few: a screen shows what a person needs to act, not
  what the app did. An error says what went wrong and what to do.
- Simple first: a person adds a repository and writes what should change. Everything else has a
  default and stays out of the way.
- No locks held across an await; no sleeps as waits.
- The contracts are `protocol/src/contracts/{manage,charter,standard,train,runner}.rs`. A contract
  changes additively.

## Where things are

| Part | Folder |
|---|---|
| Lifecycle, packages, the hold, the job, the watch, gates, the inbox | `plugins/manage` |
| The chat flow's stages, prompts and schemas | `plugins/chatflow-manage` |
| Rules, proposals, printing `AGENTS.md`, checks | `plugins/charter` |
| The lock and the Standard view | `plugins/standard` |
| Soak and the release train | `plugins/release-train` |
| Runner support in CortexCode | `docs/specs/01-WORK.md`; CortexCode's `plugins/runner` |

Building the app: `scripts/build-app.sh build` makes `build/CortexManage.app`.
