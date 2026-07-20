# Verify

Decide whether the reported behavior is an actual bug or the compiler working as
intended.

**CRITICAL: You MUST always read `report.md` and append your findings before
finishing, regardless of outcome. Downstream skills depend on `report.md`.**

**SCOPE: Verification only. Do NOT fix anything. Do not spawn sub-agents.**

## Prerequisites

- **`triageDir`** — Fixed scratch directory `triage/current` (gitignored). The
  harness does not pass this; always use `triage/current`.
- **`report.md`** — At `triage/current/report.md`. Full prior context.
- **`issueDetails`** — The GitHub issue payload, if you need to re-read it.

There is **no reference compiler** to diff against. Do not consult or clone the
legacy Go `@astrojs/compiler`. Correctness is judged from the sources below.

## Step 1: State the claim

From `report.md`, extract the reporter's **expected output** and the **actual
output**. The expected output is the claim you are testing: is it correct, or a
misunderstanding of how the compiler is designed to emit code?

## Step 2: Research intent

Do not assume the reporter is right. Weigh these sources:

### 2a. The syntax spec

[`docs/SYNTAX_SPEC.md`](../../../docs/SYNTAX_SPEC.md) defines the `.astro`
surface the compiler accepts and how it is meant to be emitted. If the spec
describes the current output, the behavior is intended. If it promises the
reporter's expected output, that supports a bug verdict.

### 2b. The existing test corpus

`crates/astro_codegen/tests/fixtures/*.snap` and the unit tests in
`crates/astro_codegen/src/**` encode expected output that maintainers have
already accepted. Check whether a fixture or test already asserts the current
behavior:

- If an existing snapshot deliberately shows the "wrong" output, it was likely a
  conscious choice — lean toward intended-behavior.
- If making the reporter's expected output pass would flip many unrelated
  snapshots, the current behavior is probably a deliberate trade-off, not an
  accident.

### 2c. Source intent signals

In `crates/astro_codegen/src/`, look for:

- **Comments explaining "why"** — strong evidence of a deliberate choice,
  especially ones citing the `oxc` fork or a linked issue.
- **Explicit conditionals** that handle the reported case on purpose.
- **Named options/constants** that gate the behavior.

### 2d. History

Run `git blame` on the lines `report.md` identified, read the introducing commit
(`git show --no-patch <sha>`) and any referenced PR (`gh pr view <n>`). A
rationale from the author is strong evidence of intent. Search prior discussion:

```bash
gh search issues "<keywords>"
gh search prs "<keywords>"
```

A prior issue closed "by design" for the same behavior settles it.

### 2e. Bug vs non-bug

- A **bug** is behavior the author **did not know about or did not choose** — an
  oversight, an unhandled construct, a regression.
- A **non-bug** is behavior the author **knew about and chose** — a documented
  limitation or trade-off — even if imperfect and even if the reporter's request
  is reasonable. That is an enhancement, not a bug fix.

The test is not "is the output nice?" but "did the author know about and choose
it?" A `.astro` construct the compiler was never taught to handle is a bug; an
output the spec or a code comment deliberately prescribes is intended.

## Step 3: Verdict

- **bug** — no comment/rationale, contradicts the spec, a clear regression, or a
  construct that falls through unhandled by accident.
- **intended-behavior** — the spec prescribes it, a comment/commit explains it,
  an explicit branch handles it, or a prior issue closed it "by design." Reframe
  the reporter's concern as a possible enhancement.
- **unclear** — intent is genuinely ambiguous. Prefer this over guessing.

## Step 4: Confidence

- **high** — explicit spec text, code comments, or prior maintainer statements.
- **medium** — reasonable evidence, some ambiguity.
- **low** — mostly inference.

## Step 5: Append to `report.md`

Record: the claim, the verdict (`bug` / `intended-behavior` / `unclear`),
confidence, and the specific evidence (spec sections, snapshot/test names, code
comments, commits, prior issues/PRs). For `intended-behavior`, note the design
rationale and the possible enhancement framing. For `bug`, explain why the
behavior looks accidental.
