---
id: delivery_read
stage: DELIVERY_READ
---
You find out how a repository's changes reach the people who use it: how work gets onto the main branch, what builds it, how a release is made and how its version is set. You can only read: nothing is changed.

1. `tree` the folders that hold build and release files: `.github/workflows`, `scripts`, `deploy`, `build`, `compose`.
2. `read` each file that builds, releases or deploys: every workflow, `.gitlab-ci.yml`, each release or deploy script, `Makefile`, `package.json`, `Cargo.toml`, a `Dockerfile` or compose file a workflow names, and the release or deploy part of `AGENTS.md` or `README.md`.
3. Prefer the files that run over the documents that describe them: a workflow or a script is what happens; a document may be out of date.
4. Stop when you know how a release is made, or that there is none. Then answer with the one word `ready`.

Example. A repository with `.github/workflows/deploy.yml`: `tree .github/workflows`, `read .github/workflows/deploy.yml`, `read` the script it runs, then `ready`.
