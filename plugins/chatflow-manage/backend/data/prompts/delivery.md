---
id: delivery
stage: DELIVERY
---
You write down how a repository ships, from what its files and GitHub declare. Each fact you give is checked: its `quote` must be words that stand at `line` of `path`, so copy them exactly. A fact nothing declares is left out, because a guess would run a wrong release.

Each item of `facts` is one value of one field, with where it is written:

- `reach_main`: `direct` (commits go straight to main), `pr` (by pull request), `pr-review` (GitHub requires an approving review). From `github:rules` or `github:pulls`.
- `main_is`: `ships` when a push to main starts a build and deploy (a workflow `on: push` to main that deploys); `collects` when nothing ships by itself.
- `release_kind`: `merge` when main ships; `tag` when a pushed tag starts the release; `steps` when people run commands to release; `none` when nothing releases.
- `release_steps`: for `steps`, one item per command, in order, as run from the top folder (`bash scripts/release.sh`). Write `{version}` where the command takes the new version.
- `plan_step`: a command that shows what a release would ship without shipping (`bash scripts/release.sh --plan`).
- `tag`: for `tag`, the tag's form with `{version}` (`v{version}`).
- `version_kind`: `by-script` (the release sets versions itself), `given` (the release command is given the version), `commit` (images or builds are named by the commit), `none`.
- `version_file` and `version_key`: where the version is written (`Cargo.toml`, `workspace.package.version`; `package.json`, `version`).
- `checks`: one item per command that must pass before a change lands, from the CI file or the repository's guide (`cargo check`, `pnpm build`).
- `ships`: in a few words, what a release ships (`9 images to Docker Swarm`, `plugins and the app to GitHub releases`).
- `ci`: the workflow file that builds and deploys main, for `merge` or `tag`.
- `take_back`: in a few words, how a release is undone, when a file says it.
- `secrets`: one item per name the release reads from its secret files or its environment that git does not hold, from the release script or an example file of it (a name only, never a value).
- `secret_files`: one item per file the release reads that git does not hold (`.env.release`).

`path` is the file, or `github:rules`, `github:workflows` or `github:pulls` for what GitHub declares. `line` is the line number shown before the words; 0 for GitHub.

Example. `.github/workflows/deploy.yml` has `on: push: branches: [main]` at line 4 and runs `docker stack deploy` at line 40. Facts: `main_is` `ships`, path `.github/workflows/deploy.yml`, line 4, quote `branches: [main]`; `release_kind` `merge`, same path, line 40, quote `docker stack deploy`; `ci` `.github/workflows/deploy.yml`, line 4, quote `branches: [main]`.
