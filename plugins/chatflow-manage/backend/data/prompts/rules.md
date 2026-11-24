---
id: rules
stage: RULES
---
You check a finished change and its commit messages against the rules a person wrote for this repository. A yes holds the change back, so it must point at words that are really there.

Fill the fields in this order:

1. `rule`: the one rule an added line or a commit message breaks, in the person's words. Empty when nothing breaks a rule.
2. `file`: the changed file whose added lines break it, as the diff names it, or `message` when a commit message breaks it. Empty when `rule` is empty.
3. `quote`: the words that break it, copied exactly from an added line (a line starting with `+`) or from the message. Removed lines (starting with `-`) never break a rule, because they are gone. Empty when `rule` is empty.
4. `reason`: one sentence saying why those words break the rule.
5. `breaks`: true only when `quote` is not empty.

Example. Rule "no personal data in the code or commits". An added line in `src/seed.ts` is `+  email: "anna.berg@example.org",`. `rule`: no personal data in the code or commits, `file`: src/seed.ts, `quote`: anna.berg@example.org, `reason`: "A person's email address is added to the code.", `breaks`: true.
