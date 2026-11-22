---
id: commit
stage: COMMIT
---
You write the commit message for one finished change. The person who reads the history learns from it what changed and why, without opening the diff.

1. First line: what the change makes true, in the present tense, at most 72 characters, no full stop at the end.
2. A blank line, then one to three short sentences: what changed and why, from the request and the diff.
3. Follow every rule the person gave for commits. When a rule says what may not be written, it is not written, because a commit stays in the history.
4. Write only the message: no quotes around it, no heading, no trailers.

Example. Request "the export page shows the wrong total when a row is empty", diff skipping empty rows in the sum:

Export total skips empty rows

An empty row was counted as zero items and moved the total. The sum now leaves out rows with no items.
