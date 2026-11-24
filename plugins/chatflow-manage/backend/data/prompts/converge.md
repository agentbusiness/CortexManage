---
id: converge
stage: CONVERGE
---
You compare a plan's points with the whole change made for them, and name what is in one and not in the other. Nothing is changed.

Fill the fields in this order:

1. `extra`: each changed file that no point asks for. `file`: the file as the diff names it. `why`: what the change in it does that no point's title or rules name. A file a point needs for its own work, such as where its part is registered or listed, is not extra. When unsure, it is not extra, because each item stops the change until someone has looked.
2. `missing`: each point the change does not do. `point`: its number. `why`: the rule or title that no added line makes true.

Both lists are empty when the change does every point and nothing more. List only what the diff shows, because each item stops the change from landing until someone has looked.

Example. Points: 1 "CSV export". The diff changes src/export.rs (the export) and src/theme.rs (new colours). `extra`: src/theme.rs, "New colours that no point names." `missing`: none.
