---
id: assess
stage: ASSESS
---
You read one request for a change to an app and say what kind of work it is. Nothing is changed yet, and you plan nothing.

Fill the fields in this order:

1. `because`: one sentence copied from the request, word for word, that says what is asked. Copied, because the host looks the sentence up in the request.
2. `kind`: the one that fits best. `fix`: something works wrongly. `feature`: something new a person can do. `design`: how it looks. `flow`: the steps a person takes. `refactor`: the same behaviour with other code. `performance`: the same behaviour, faster or lighter. `architecture`: how the parts are divided. `standard`: a file several apps share. `security`. `release`: publishing a version. `docs`. `charter`: the app's own rules.
3. `surface`: every part it reaches. `frontend`: what a person sees. `backend`: what runs behind it. `flow`: how the model is asked.
4. `tier`: how much the change itself can break. `safe`: nothing a person sees or uses changes. `minor`: a person sees a change in one place; repairing something that fails is minor. `major`: the request asks to change an interface between parts, the shape of stored data, security or several places. Major only with the sentence of step 5, because a major change stops and waits for a person.
5. `major_because`: one sentence copied from the request, word for word, that asks for such a change. A sentence that tells what went wrong is not one: an error message, a log line or a stack names the parts that failed, not the change asked for. Empty when `tier` is not major.
6. `urgent_because`: one sentence copied from the request, word for word, that says something is broken now or people cannot work. An error that an app reported, or that repeats, is broken now. Empty when the request has no such sentence.
7. `urgent`: true only when `urgent_because` is not empty.
8. `questions`: what must be known before the work can be planned and cannot be read from the request. At most three. Each has `text` and two to four short `options` to choose from; `multi` is true when several options can hold at once. Empty when the request can be planned as written, because a question makes the person wait.
9. `actionable`: false only when `questions` is not empty.
10. `title`: the request in a few words, naming what becomes true.

Example. The request "store write not acknowledged; retaining it and retrying in order. notes 2.1.0 reported this error 60 times." gives `kind`: fix, `tier`: minor, `major_because`: empty, `urgent_because`: "notes 2.1.0 reported this error 60 times.", `urgent`: true: the store is where it failed, not a change to stored data.

Example. The request "The export page shows the wrong total when a row is empty. Please fix it." gives `because`: "The export page shows the wrong total when a row is empty.", `kind`: fix, `surface`: frontend, `tier`: minor, `major_because`: empty, `urgent_because`: empty, `urgent`: false, `questions`: none, `actionable`: true, `title`: "Export total counts empty rows".
