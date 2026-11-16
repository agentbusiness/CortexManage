# manage

CortexManage's lifecycle engine (`manage@1`, `protocol/src/contracts/manage.rs`,
`docs/specs/05-MANAGE-CONTRACT.md`). A request is assessed, planned and validated, waits in a
package for its hold to end, is built by a runner under the watch, proven by the gates, landed and
handed to the release train. The plugin never edits code: a CortexCode on a runner does.

- Backend: `backend/` (binary `cc-manage`, category `manage`).
- UI: `ui/` (sidebar, centre, card, watch, evidence, inbox, hold, settings), listed in `plugin.toml`.
- Texts a person or a builder reads: `backend/data/texts.toml`, read live from the live folder.

## The path of a request

```
arrived → assessed → planned → validated → HELD → building → proving → landed → soaking → released
              ↘ clarifying (your answers)      ↑ clock / your yes        ↑ gates, rework rounds
ends as: released · declined · merged · withdrawn
```

1. **Pipeline** (`pipeline.rs`), in the background, one model question per call to `chatflow-manage`:
   `assess` (not actionable → the clarify form), `plan` (with the charter's identity and rules;
   versions are numbered here), `validate` (one call per point). Host checks: an urgent sentence
   must be in the request; a verdict without a reason is `ask`; a `keep` that reaches standard
   files in a change that is not a standard change is `ask`; in a safe plan a point whose only
   fault is "no walk of its own" is kept when a kept point has a walk. An `ask` point becomes a
   keep / leave out question to the person. Most points not fitting the app → declined. No kept
   point → declined.
2. **Tier** = the stricter of the model's proposal and the host's floor (`policy.rs`): safe before
   a plan exists; after it, from the kept points' `touches` against the repository's
   `major_paths` / `minor_paths`, from `standard.touches` (any standard path → major) and from the
   plan's size (more than 12 points or 2500 limit lines → major). **Urgent**: a fix or a security
   change that is not major, as the model proposes with a sentence of the request; the person's
   own switch wins over the proposal. A major change is never urgent.
3. **Packages and the hold** (`packages.rs`): a validated request joins a held package of its
   repository when their scopes overlap (code) or the model says they belong together; the same
   thing asked twice is merged; a contradiction waits on you. At most 12 points and 5 requests.
   Urgent and standard requests are alone. Safe and minor go by the clock (a joiner restarts it,
   never past `hold_max_min` since the package was first held); major waits for your yes. The
   clock looks at the wall time every 15 s and starts the due package when the Hold switch is off
   and nothing else of the repository is building (urgent first).
4. **The runner link** (`link.rs`): `mesh.link {node}` → a WebSocket to the runner's frame, the
   frame's JSON envelopes as a client, one connection per runner, opened again when it drops.
   Calls `runner.*`, subscribes to `runner.beat`. A repository with no runner cannot build: the
   package waits on you with "this repository has no runner yet; set one in its settings".
5. **The job** (`job.rs`): prepare (branch `cm/<package>-<slug>`), start with the kept points as
   tasks (no walks; `known` gains the charter's rules for the task's paths; `protected` is the
   standard's paths plus `AGENTS.md` and `standard.lock`). Every beat is answered (`stop` when
   the package was stopped or Hold is on) and the watch rules run over its facts:

   | Rule | Fires when | Ladder |
   |---|---|---|
   | W1 | the same failing command 3 times | steer → restart → you |
   | W2 | the same call and result 4 times | steer → restart → you |
   | W3 | 10 calls without an edit, or 10 minutes idle | steer → restart → you |
   | W4 | a file changed outside the task's scope | steer → restart → you |
   | W5 | a protected file changed | restart (restores the checkpoint) → you |
   | W6 | a charter check fails on a checkpoint's diff | restart → you |
   | W7 | minutes, tokens or lines over the tier's budget | stop → you |
   | W8 | no beat for 60 s | marked lost; `runner.get` when the runner is back |

   Per rule and task. Nothing is steered while the builder waits on an approval. Approval
   questions are refused with a sentence (plans list no approval classes yet); a second refusal
   on one task waits on you. A held replan is validated task by task; accepted when all are kept
   and the floor does not rise, else refused (a risen floor stops the job and holds the package
   again at the new tier). After each checkpoint: `runner.diff`, then one `drift` question; a yes
   whose file and line are not in the diff counts as no; a no restarts the task with the reason.
6. **Gates** (`gates.rs`) when the job is done: scope, standard, charter, build, walks, converge.
   `command` walks run on the runner (`expect` must be in the output); `computer`, `mesh` and
   `journal` walks are `blocked` (nothing on the mesh performs them yet). A failed gate starts a
   rework round (the failing tasks restart with the finding), as many as the policy gives the
   tier, then it waits on you. Blocked walks → your yes; major packages → your yes with the
   evidence. Then `runner.land` (a rebase → build and walks again; conflicts → one "resolve"
   restart, then you), the soak build, `train.landed`, `runner.remove`. `train.released` marks
   the requests released.
7. **The inbox** (`inbox.rs`): open asks with `why` / `if_yes` / `if_no` (the chat flow's `why`,
   the templates of `texts.toml` until and unless it answers), what goes by itself, the count
   line, and `manage.decide` (refused with `conflict` when the attempt is not the current one).
8. **The word** (`words.rs`) is computed each time: on hold › needs you › stuck › rework › working
   › watching › held › queued › done › idle.

When the chat flow, the charter, the standard or the train does not answer, that is "not
available": the request or package stays where it is with one note in its thread and goes on when
the plugin's `plugin.state` / `plugin.swapped` event says it is active (and every ten minutes).
A missing charter is tolerated in assessing, planning and the builder's rules; the charter gate
waits for it.

## Methods

| Method | Does |
|---|---|
| `manage.repo.list` / `get` / `create` / `edit` | Repositories: source, runner, policy |
| `manage.request.create` / `get` / `list` / `edit` | Requests; `edit` takes `urgent`, `withdraw`, `answers`, `add_text` |
| `manage.request.say {request, text}` | The person wrote in the thread: re-plan before the build, a steer during it. Answers `{text}`, the line to show (the chat flow writes the person's message itself) |
| `manage.package.get` / `list` / `edit` | Packages with gates, interventions, evidence, the strip; `edit` takes `start_now`, `stop`, `unbundle` |
| `manage.decide {id, attempt, action, note}` | Answers an ask |
| `manage.inbox.list` | Asks, going, the count line |
| `manage.hold.get` / `edit {on}` | The Hold switch |
| `manage.evidence.read {id}` | `{kind, mime, base64}` of a walk's output |
| `manage.discover.read` / `search` / `tree` | The runner's `runner.read` / `search` / `tree` (an error when there is no runner) |
| `manage.runner.get {repo}` | `{reachable, node, name, jobs}` |
| `manage.runner.release {repo, plan_only}` | The runner's `runner.release` (the train calls it) |
| `manage.shares` | The shared service `manage.request` (`create`, `get`, `list`), for apps |

Events: `manage.changed {what, id, repo}`, `manage.watch {package, strip, digest}`.

## Records (the graph, WORKSPACE layer; nothing is deleted)

```
(:Repo)-[:ON]->(:Machine)                (:Repo)-[:HAS_REQUEST]->(:Request)-[:ASKS]->(:Question)
(:Request)-[:HAS_PLAN]->(:Plan)-[:HAS_POINT]->(:Point)-[:PROVED_BY]->(:Walk)
(:Repo)-[:HAS_PACKAGE]->(:Package)       (:Request)-[:PART_OF]->(:Package)
(:Package)-[:HAS_GATE]->(:Gate)          (:Package)-[:WATCHED]->(:Intervention)
(:Package)-[:HAS_EVIDENCE]->(:Evidence)  (:Request|:Package|:Repo)-[:DECIDED]->(:Decision)
(:Request)-[:MERGED_INTO]->(:Request)    (:ManageState)-[:ON]->(:Machine)
```

Every node carries `of: "manage"`. Ids: requests `<PREFIX>-<4 digits>` per repository, packages
`pk-<4 digits>`. An ask is a `Decision` that is still open. Stage, tier, kind, urgent, the hold's
end, a point's verdict and a gate's result are also written as temporal properties
(`<name>_history`). The plugin works from a copy in memory, read when it is activated and written
through. Bytes (a walk's output) live in the plugin's data folder under `evidence/`.

Each repository has a frame project (`<CORTEX_HOME>/repos/<id>`), each request a frame session
(`flow: "manage"`); the session's journal is the request's thread (`manage::kinds`).

## Not wired yet

- A charter request's landing does not call `charter.amend`: the charter page opens the request,
  applying the proposal is future work.
- Unbundling one failing request out of a built package (reverting its commits): the runner has
  no call for it; a package whose gates keep failing waits on you as a whole.
- An urgent package goes first in the queue but does not interrupt a job that is running.
- `computer`, `mesh` and `journal` walks are blocked by design until something on the mesh
  performs them.
- Soak reports opening requests by themselves, and a global policy above the repositories'.
