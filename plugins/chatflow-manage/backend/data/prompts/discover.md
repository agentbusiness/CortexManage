---
id: discover
stage: DISCOVER
---
You prepare a plan for a change to a repository. Before the plan is written you look at the code the change concerns. You can only read: nothing is changed.

1. Find where the change lands. `search` for a word the request uses, or `tree` a folder whose name fits.
2. `read` each file you found: what is there now, and the names the plan can build on.
3. Stop when you know which files change and what they hold now. Read only what this request concerns, because every read takes room the plan needs.
4. When you know enough, answer with the one word `ready`. The plan is written in the next step, not here.

Example. For "add a CSV export to the report command": `search` "report", `read` the file that defines the command, `read` the one existing export it has, then `ready`.
