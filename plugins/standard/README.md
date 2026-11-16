# standard

One mesh standard for every Cortex app (`standard@1`, `docs/specs/04-STANDARD.md`): the members,
the lock, the check of every member's copy against the lock, the handshake, and the Standard view.

## What it does

- **Members.** Who carries the standard, each with its priority, its own names, the folder of
  each part in its repository, the parts it is without, whether it is left out for now
  (`deferred`), and where its checkout is on this computer (`root`). Written on the first start
  from the six of the spec; a `root` is set only when that folder exists here.
- **The lock.** `standard.lock.write {from}` hashes (SHA-256) every standard file of the `from`
  member's checkout. A file that carries the member's own values is hashed with them replaced and
  marked `names`: each of the member's `names` becomes a placeholder (one per shape of name, so
  `cortexterm` for `CortexTerm` is still a difference), and its priority becomes a placeholder
  where it directly follows a text `parts.toml` lists. The version is the one given, or the last
  one's minor plus one, starting at `1.00`. `standard.lock.print` gives the lock as the text of
  `standard.lock`.
- **The check.** `standard.check` compares every member that has a checkout here and is not
  deferred with the lock, part by part: `same`, `names`, `differs`, `missing`, `without`,
  `deferred`, `not_here`. A file of a part that the lock does not have is a difference too. It
  publishes `standard.changed {version, drift}`. A lock write and a member change check again.
- **Standard paths.** `standard.paths {member}` gives the member's standard paths as globs
  relative to its repository root (and `standard.lock` itself); `standard.touches {member, paths}`
  says which of some changed paths are standard files.
- **The handshake.** `standard.handshake` reads `mesh.peers` and asks each reachable member for
  its card (`mesh.call` of `mesh.card`), then checks the answer against the contract's `Card`.
  That is the one read method a member's mesh answers to another member; `mesh.status`,
  `mesh.peers` and `mesh.services` are refused over the mesh, so they are not asked. A row is
  `answers` only when a card came back and fits; otherwise `unreachable` or `fails` with the reason.

## Which files are the standard

Data, not code: `backend/data/parts.toml` (packed, and read live from the live folder on every
use). One `[[part]]` per part with its file patterns, `[skip]` for build output and lock files,
`[own]` for where a member's priority stands. The choices and their reasons are in its comments.

## Records (the graph, WORKSPACE layer)

| Node | Key | Properties | Edge |
|---|---|---|---|
| `StandardMember` | software id | `name priority without[] deferred root names[]` | `ON` → `Machine` |
| `StandardFolder` | `<member>/<part>` | `member part folder` | `OF` → `StandardMember` |
| `StandardLock` | `current` | `version from written_ms files` | `ON` → `Machine` |
| `StandardFile` | `<part>/<path>` | `part path sha256 names` | `IN` → `StandardLock` |
| `StandardCheck` | `last` | `version checked_ms parts[] drift[]` | `ON` → `Machine` |
| `StandardCell` | `<member>/<part>` | `member part state files[] count` | `IN` → `StandardCheck` |

A file, folder or cell that is no longer there is retired, never deleted. The handshake's answer
is not kept: it is true only for the moment it was asked.

## UI

`standard.page` (`page.standard@1`): one row per part, one column per member; the version and
when the files were checked; Check now, Write lock from (a menu of the members with a folder
here), Ask the members, the lock's text; a cell with differing files lists them when chosen; the
members with their folder and the Deferred switch.

## Probe

Writes two small checkouts into the plugin's data folder, makes a lock from one, prints it and
reads it back, checks the other against it (the two differ only by name: `names`), changes one
file and checks again (`differs`), then removes the folder.
