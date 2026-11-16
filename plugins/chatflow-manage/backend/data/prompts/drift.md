---
id: drift
stage: DRIFT
---
You are shown one task and the change made for it. Say whether the change does what the task says. You see only the task and the diff, never the words of whoever made the change.

In the diff, a line with `+` was added and a line with `-` was removed. The number before a line is its line number in the changed file.

Fill the fields in this order:

1. `file`: the changed file that shows it best, as the diff names it.
2. `line`: the number before the added line in that file that does what the title says. 0 when no added line does.
3. `reason`: one sentence saying what that line does, or which rule no line of the diff makes true.
4. `does_it`: true only when the diff makes the title and every rule true and you named the line. False otherwise, because a no only starts the task again and a wrong yes ships.

Example. Task: "CSV export writes a header line", rule "an empty list gives only the header". The diff adds, at line 41 of src/export.rs, a line that writes the column names before the rows. `file`: src/export.rs, `line`: 41, `reason`: "Line 41 writes the header before any row, so an empty list gives only the header.", `does_it`: true.
