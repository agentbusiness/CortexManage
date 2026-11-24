---
id: plan_fields
---
Fill the plan's fields in this order:

1. `analysis`: your thinking, six short lines at most, no code: what must be true when the work is done (every behaviour, value, error and limit the request states, in its words); the order to build it in; how each part is shown to work; what is unknown and how to find out.
2. `goal`: what the finished work achieves, in one or two sentences.
3. `approach`: how, and why this way.
4. `architecture`: the parts, data and interfaces it adds or changes, by their names in the repository. Empty unless the line about this change's size, right before the step, asks for a design note.
5. `risks`: what could go wrong or is unknown, each with how to find out.
6. `tasks`: one task per part that can be built and checked on its own: a new file or module, a function or a small group of functions, a command, an option. A task is small: one part, its few rules, its own files. A request with one such part is one task. Two tasks that change the same file depend on each other. A task changes code: never a task only to verify, test or review. Only what the request asks: name anything more in `note` instead. Existing names stay: a task builds on the functions, types and files that are there. For each task:
   - `title`: a few words naming what it makes happen ("Retry failed uploads"), never only a function's name and never the method.
   - `brief`: the edits to make, as orders to the builder: in which file, what is there now and what it becomes. Name each function it adds or changes with what it takes and returns. Signatures, no function bodies. Only this task's own part.
   - `known`: what the reads above established that this task needs, one line each: a file and line with what is there ("src/export.rs:40 writes the header before the rows"). Empty when nothing is.
   - `rules`: each thing the request says must hold that this task makes true, one line each in the request's own words ("an empty list gives only the header"). At most four: a part with more rules is two tasks. A rule is a check, never a method.
   - `depends_on`: the numbers of the earlier tasks it needs (tasks are numbered from 1 in the order written).
   - `touches`: the files or folders it may change. The builder is held to them, so name every one it needs and no other.
   - `walks`: how a person sees that the task works. Each walk:
     - `do`: the steps to take in the running app, in order ("Open the report page, press Export, choose CSV").
     - `see`: what is seen when it works ("A file is saved; its first line is the header").
     - `method`: who can take the steps. `computer`: someone operating the screen. `command`: one command in a shell. `journal`: reading what the app recorded. `mesh`: calling the app from another app.
     - `evidence_kind`: what is kept of it. `shot`: a picture of the screen. `recording`: the screen over time. `log`: the command's output. `trace`: the recorded calls.
     - `command`: for `method` command, the one command to run; empty otherwise. `expect`: for `method` command, a text its output contains when it works; empty otherwise.
     A walk shows a rule of its task holding, in the request's terms. Never "the tests pass" and never a look at the code.
7. `note`: one short sentence for the person: what to review, and what more should be done that the request did not ask for.

Example. "Add a CSV export to the report page" is one task, "CSV export", touching the report page's file and one new file, with one walk: do "Open the report page, press Export, choose CSV", see "A file is saved; its first line names the columns", method computer, evidence_kind shot. "Add a CSV export and a JSON export" is two tasks; when both change the page's file, the second depends on the first.
