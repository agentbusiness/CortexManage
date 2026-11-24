# chatflow-manage

CortexManage's chat flow: category `chatflow-manage` (a member of the `chatflow` family, contract
`chatflow@1`), binary `cc-chatflow-manage`. Sessions created with `flow: "manage"` route their chat
verbs here. Everything the manager asks a model goes through this plugin, one question per call.
Specs: `docs/specs/05-MANAGE-CONTRACT.md` §5, `docs/specs/06-PLUGINS.md` §3–4.

It keeps no records. Its answers go back to the caller (the manage plugin), which owns them.

## Methods

The chat verbs (`chatflow@1`), as far as a request's session needs them:

| Method | Does |
|---|---|
| `chatflow-manage.turn {session_id, text}` | Writes the text into the session as `user_message`, answers `{run_id}` at once, then finds the request whose session this is (`manage.request.list`), passes the text on (`manage.request.say {request, text}`) and writes the returned line as `assistant_message`. A failure is written as an `error` entry. |
| `chatflow-manage.steer` | The same as `turn`: whether a text is added to the request or steers the builder is the manage plugin's to say. |
| `chatflow-manage.state {session_id}` | `FlowState`: whether a message is on its way or a stage question runs for the session, and which. |
| `chatflow-manage.running` | The sessions with something in flight. |
| `chatflow-manage.cancel {session_id}` | Cancels what is in flight for the session. `{cancelled}` |
| any other chat verb | Refused: not part of this flow. |

The manager's questions (`manage::flow`), each one model call in a context of its own:

| Method | Answer | Host check, and what stands when the model's answer cannot be used |
|---|---|---|
| `assess` | `AssessedParams` | `because` (and `urgent_because` when urgent) must be words of the request; not actionable needs its questions. Else: major, not urgent, actionable. Questions are kept only when not actionable. |
| `match` | `MatchAnswer` | Else: `unrelated`. |
| `plan` | `Plan` | Tasks numbered from 1; a task depends only on earlier ones; a `command` walk has its command; limits never zero; at most 12 tasks. No usable plan is an error. |
| `validate` | `ValidateAnswer` | A verdict without a reason is `ask`; `keep` with one of the five answers false is `ask`; a point on a standard file is never `standard_safe`. Else: `ask`. |
| `drift` | `DriftAnswer` | A yes without a file and a line is a no. An empty diff is a no without a model call. Else: no. Whether the line is in the diff is the caller's check. |
| `converge` | `ConvergeAnswer` | `file` can only be a file of the diff, `point` a point given. An empty diff misses every point, without a model call. Else: an error (data `{extra: [], missing: [], compared: false}`), never empty lists as an answer. |
| `digest {facts}` | `{text}` | Free text, cut to two lines. Nothing is decided from it. |
| `why {facts}` | `WhyAnswer` | No usable words is an error. |

An answer that fails its schema or its check is asked once more, with what was wrong as the line
before the step; then the cautious choice stands. A call that fails (no model, cancelled) is an
error, never an answer.

## How a model call is made

`backend/src/llm.rs`, the same mechanism as CortexCode's chat flow:

1. `model.count` sizes the prompt.
2. `mesh.claim {kind: "model", tag, session_id: <key>, foreground: false, tokens}` takes one use of
   the best free model with the stage's tag (`office`, the Gemma), waiting when none is free. The frame then
   routes the key's calls there. Background: a person's turn goes first.
3. `model.chat` runs with `cache_key: <key>`, the stage's grammar, greedy sampling, a token limit.
4. `mesh.release`, and `mesh.place.end` when the question is over.

Each question has a new key, so it starts from an empty context. The answer's shape is the stage's
JSON Schema, sent as a GBNF grammar generated from it (the property order is the schema's
`required` list: JSON loses its order on the way through the frame). Constrained stages do not think.

PLAN is the only stage with tools, and only three that read: `read`, `search`, `tree`, served by
`manage.discover.read | search | tree` with the plan's `repo`. A short loop (at most 12 calls, each
result shown head and tail), then the plan as one constrained question that carries what was read
as text and no tools. When discovery answers an error, the plan is written from the request and
the rules alone and its `note` says so.

## Data

`backend/data/`, compiled in and read again from the live folder
(`<home>/live/plugins/chatflow-manage/backend/data/`) for every call. A live file that does not
validate is left out; the shipped one stays in use, and the probe names it.

```
stages.toml     per stage: model tag, prompts, schema, token limit; the tool loop's limits; sampling
texts.toml      the blocks and last lines of each question, tool texts, the few texts a person reads
prompts/        assess match discover plan plan_fields validate drift converge digest why
schemas/        assess match plan validate drift converge why
```

The prompts follow CortexCode's rules for model-facing text: short numbered steps, one reason per
choice, a neutral example, the evidence right before the decision, the step to take as the last
line, tools only in the one prompt that can run them.

## Probe

Loads every prompt, schema, text and stage setting, builds an answer of each schema's shape and
reads it into the contract's type the way a model's answer is read. No model is called.

## UI

`ui/session`: `chatflow-manage.session` on `manage.session@1` (see its README).
