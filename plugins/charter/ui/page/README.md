# @cortex-ui/charter-page

The Charter view of a repository (or of the shared standard when no repository is selected).

- **No charter yet:** paste the repository's `AGENTS.md`; it becomes a draft, one rule per paragraph
  or list item.
- **A draft:** set each rule's level with the segmented control, edit texts, say how each rule is
  kept, then accept the draft as version 1.0.
- **An accepted charter:** left, the rules by level with their id, text, how each is kept (a law or
  constraint kept only by text is marked) and why. Right, the proposal panel: pick a rule or a new
  rule, write the new text and why, press Propose. It shows old and new side by side, what the
  change needs and the rules it may conflict with. *Open request* hands it to `manage.request.create`
  with the proposal's digest in the text. A law has *Edit law* instead, which asks for the law's id.

- **Plugin id:** `charter.page`
- **Slot:** `page.charter@1`
- **Reads:** the shared selection `manage.selection` (`repo: null` shows the standard charter)
- **Calls:** `charter.*`, `manage.request.create`
- **Build:** `npm run build` → `dist/` (`index.js`, `style.css`, `manifest.json`)

React, ReactDOM and `@cortex/sdk` are peer dependencies the workbench provides.
