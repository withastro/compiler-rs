---
name: pr-writer
description: Write pull request titles and bodies for compiler-rs that follow the repository PR template. Trigger whenever asked to create, open, or draft a PR, or to write or update a PR title, body, summary, or description.
---

# PR Writer

Produce a reviewer-friendly title and a body that matches this repository's
[`PULL_REQUEST_TEMPLATE.md`](../../../.github/PULL_REQUEST_TEMPLATE.md) exactly.

When invoked by the triage bot, you are given the issue and the fix branch and
asked to **return only the title and body** — do not open the PR yourself.

## Title

- Plain-language description of the outcome, the way a person writes it in a
  review queue.
- No conventional-commit prefixes (`fix:`, `feat:`) and no scopes
  (`fix(codegen): ...`).
- Concise and specific.

## Body

Use these three sections, in this order, and keep all three even when a section
is short — the template requires it:

```md
## Changes

- <What behavior changed and why it matters>
- <Key implementation detail a reviewer should know>

Closes #<issue-number>

## Testing

- <The regression fixture or test added, and what it asserts>

## Docs

- <Docs impact, or one sentence on why none is needed>
```

### Changes

Describe the behavior change and how the fix works at a reviewer-useful level:
what `.astro` input was miscompiled (or crashed) and what the compiler now emits
instead. Include `Closes #<issue-number>` so merging closes the issue. Do not
list "added a test" or "added a changeset" here — those are process, not
behavior.

### Testing

Name the regression test that guards the fix and what it covers — e.g. the
snapshot fixture `crates/astro_codegen/tests/fixtures/_<issue#>_<desc>.astro`, a
`compile_astro(...)` unit test, or a `packages/compiler/test/**` case. State
what output it now asserts. Do not report that the suite passes (CI shows that)
or which commands you ran.

### Docs

`docs/SYNTAX_SPEC.md` is the only in-repo documentation. If the fix changes the
documented `.astro` surface, note the update; otherwise say why none is needed
(e.g. "bug fix only, emitted output now matches the spec").

## Brevity

Default to 1-2 bullets per section. A reviewer should absorb the whole body in
under 30 seconds for a typical patch.

## Self-check before returning

- Title is reviewer-friendly, not commit-style.
- All three template sections are present.
- `Closes #<issue-number>` is in the body.
- `Testing` names the regression test; `Docs` states an explicit decision.
- A changeset already exists in `.changeset/` (created during the fix). If it is
  missing, flag that rather than describing it in the body.
