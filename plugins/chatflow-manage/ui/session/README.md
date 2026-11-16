# @cortex-ui/chatflow-manage-session

A request's session: the column head (title, tier, kind, where it reaches, urgent), the thread drawn
from the session's journal, and the box to write in.

The open request comes from the shared selection (`manage.selection`); `manage.request.get` gives its
session, tags and stage, read again on `manage.changed`. The thread has one block per kind of entry
the manage plugin writes: assessment, clarify (a form), manage_plan, package (Start now, Stop it,
Build apart), watch, digest, gate, stage, ask (its decision buttons), beside `user_message`,
`assistant_message`, `error` and `note`.

- **Plugin id:** `chatflow-manage.session`
- **Slot:** `manage.session@1`
- **Calls:** `manage.request.get`, `manage.request.edit {id, answers | urgent}`,
  `manage.package.edit {id, start_now | stop | unbundle}`, `manage.decide {id, attempt, action}`,
  `chatflow-manage.turn | state | cancel {session_id}`
- **Build:** `npm run build` → `dist/` (`index.js`, `style.css`, `manifest.json`)

React, ReactDOM and `@cortex/sdk` are peer dependencies the workbench provides.
