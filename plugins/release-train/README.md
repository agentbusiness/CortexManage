# release-train

Soak and the release train (`train@1`, `docs/specs/02-HOLD-PACKAGES-RELEASE.md` §6-7). Category
`train`, binary `cc-train`, one UI package (`release-train.page`, slot `page.releases@1`).

A landed package is not released at once: it soaks in the owner's own apps. The repository's daily
train releases every package that finished its soak without a report. Nobody presses, for any
tier. A released package can be taken back with one press.

## Methods

| Method | What it does |
|---|---|
| `train.landed` | A package landed. Its soak ends at `now + soak_min`; with `0`, or when urgent, it is ready at once. An urgent package leaves on a hotfix train of its own straight away. |
| `train.report {repo, part, text}` | Every package of the repository still in its soak that names the part gets the report and is `held_back`. |
| `train.get {repo}` | When the next train leaves, what it would carry now, what has not left yet, the last 20 releases. |
| `train.run {repo}` | Runs the train now and answers the `Release`. |
| `train.skip {repo}` | The next train of the repository does not run (the day is remembered). |
| `train.list {repo}` | The repository's releases, newest first. |
| `train.take_back {package}` | Opens an urgent request that reverts a released package (`manage.request.create`) and marks the soak `taken_back`. Answers `{request}`. |
| `train.bump {version, tier, breaking}` | A part's next version: three numbers of 0 to 99 with carry. |

It publishes `train.changed {repo}` after any change and `train.released` (the `Release`) when a
train shipped (`ran` or `nothing`). It subscribes to nothing.

## The clock

The clock looks at the wall clock at least every 30 s (the monotonic clock stops while a Mac
sleeps). Soaks whose time passed with no report become `ready`. When a repository's train time
(`manage.repo.get` → `policy.train_at`, local `HH:MM`, `03:00` when it cannot be read) has passed
today, the day is written to the graph first and then the train runs, so it leaves once per day,
also after a restart or a failure. A package that becomes ready later that day waits for the next
day's train. A Mac that slept through the train time runs the train when it wakes.

## A train

1. `manage.runner.release {repo, plan_only: true}`: the parts that would move. None: a release in
   state `nothing`; the ready packages are done.
2. `manage.runner.release {repo, plan_only: false}`.
3. The release is recorded with its part moves (which requests each part carries), the note (one
   line per request, grouped by part) and the script's last lines; the soaks become `released`.

A failure is recorded as `failed` with the script's last lines; the soaks stay ready and ride the
next day's train. One train per repository runs at a time.

## Records (the graph, WORKSPACE layer)

| Node | Key | Properties |
|---|---|---|
| `Soak` | the package | `package repo tier urgent commit landed_ms until_ms state parts reports release taken_back_request`, and the requests as three lists of the same length: `request_ids request_titles request_tiers` |
| `Release` | its id | `id repo ms state note tail hotfix packages` |
| `PartMove` | `<release>/<part>` | `release n part from to carries` |
| `TrainClock` | the repository | `repo ran_day skip_day` |

Edges: `(:Release)-[:SHIPS]->(:Soak)` for each package a release shipped,
`(:Release)-[:MOVES]->(:PartMove)`. Nothing is deleted; the only node retired is the probe's soak.
A list with nothing in it is not written (the property is removed).

## Probe

The bump cases, and a soak written to the graph under the repository id `probe.release-train`,
read back, compared and retired.

## Texts

`backend/data/texts.toml`: the release note's fixed lines, the take back request, and what a
refused call says. A live copy in the plugin's live folder is read on every use.
