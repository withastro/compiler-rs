# Fix

Develop and verify a fix for a diagnosed compiler bug.

**CRITICAL: You MUST always read `report.md` and append to it before finishing,
regardless of outcome. Even if the fix fails, record what you tried.**

**SCOPE: Do not spawn sub-agents. Do not commit or push — the orchestrator
stages, commits, and (when a fix lands) opens the PR.**

## Prerequisites

- **`triageDir`** — Fixed scratch directory `triage/current` (gitignored). The
  harness does not pass this; always use `triage/current`.
- **`report.md`** — At `triage/current/report.md`. Root cause and suggested
  approach.
- **`issueDetails`** — The GitHub issue payload, if needed.

Follow the repo conventions in `AGENTS.md` (Rust edition 2024, 4-space indent)
and the [`writing-comments`](../writing-comments/SKILL.md) skill for any comment
you add.

## Step 1: Decide the path

Read `report.md`.

- **Skip if unusable:** not reproduced, skipped, or the cause is in the `oxc`
  parser fork (out of this repo) → append `FIX SKIPPED: <reason>`, return
  `fixed: false`. Do not guess a fix you cannot verify here.
- **Low confidence (`low`/`null`) or no clear root cause → breadcrumb path**
  (Step 6). Do not force a speculative code change.
- **Medium/high confidence with a clear cause → fix path** (Steps 2-5).

The tree may be dirty from earlier steps. Run `git status`; revert any leftover
instrumentation before starting (`git checkout -- <file>`).

## Step 2: Implement the fix

Edit source under `crates/astro_codegen/src/`. Keep it minimal:

- Change only what the root cause requires; do not refactor unrelated code or
  restyle adjacent lines.
- Respect the 380+ existing snapshot fixtures and the unit tests — they encode
  behavior you must not regress.
- Watch edge cases the diagnosis flagged (unusual attributes, nesting, options).

## Step 3: Verify against the reproduction

Rebuild and re-run the reproduction:

```bash
INSTA_UPDATE=new cargo test -p astro_codegen snapshots   # regenerates the fixture's .snap.new
cat crates/astro_codegen/tests/fixtures/_<issue#>_<desc>.snap.new
```

Confirm the output is now correct. For a wrapper-layer fix, rebuild the addon
and exercise the JS path:

```bash
pnpm run build:napi && pnpm test
```

## Step 4: Lock in the regression test

The `.astro` fixture from reproduce becomes the permanent regression test.
Accept its snapshot once the output is correct:

```bash
cargo insta accept                            # writes .snap from the pending .snap.new
cargo test -p astro_codegen                   # confirm green
```

Check `git status` shows only your intended `.astro` + `.snap` (and source
changes) — no stray accepted snapshots from unrelated tests. If a fixture cannot
capture the bug (e.g. it needs options the harness lacks), add a unit test with
`compile_astro(...)` in the relevant `crates/astro_codegen/src/printer/*_tests.rs`
module, or a `packages/compiler/test/**/*.ts` case instead, and say why in
`report.md`.

## Step 5: Guard the whole suite

Do not break anything else:

```bash
cargo test                     # full Rust suite
cargo clippy -- -D warnings    # CI treats any warning as an error
cargo fmt                      # format Rust
```

If you touched the TypeScript wrapper, also run `pnpm run build:napi && pnpm test`
(the addon must be rebuilt for the JS layer to see Rust changes) and
`pnpm lint` / `pnpm format` for Biome.

Fix any failure before proceeding. If your change legitimately alters other
snapshots, re-examine whether that is correct — an unexpected snapshot flip is
usually a sign the fix is too broad.

## Step 6: Breadcrumb path (low confidence / no fix)

When you cannot land a confident fix, leave a useful trail instead of a wrong
patch:

1. Keep the reproduce fixture **without** an accepted `.snap` so the snapshot
   test fails and documents the bug, **or** add a `#[test]` asserting the
   expected output that currently fails.
2. Add up to 2-3 `// TRIAGE:` comments at the most relevant source lines to
   orient a maintainer.
3. Append the suspect files/paths and your reasoning to `report.md`.
4. Return `fixed: false`. Skip Step 7.

## Step 7: Changeset (fix path only)

Only when the fix succeeded, add `.changeset/<two-random-words>.md` bumping both
fixed-group packages at `patch` (bug fixes are patches):

```md
---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

<One sentence describing the user-facing fix.>
```

## Step 8: Append to `report.md`

Record: what changed and why, the `git diff` of the source, verification results
(does the reproduction now pass?), the regression test added (path + what it
asserts, or why none), the changeset filename, any alternatives/trade-offs, and
— if `fixed: false` — what you tried and why it did not work. Return `fixed` and
a short `commitMessage`.

## Step 9: Clean the working tree

1. `git status` and review every changed file.
2. Revert anything not part of the fix: `eprintln!`/`dbg!` instrumentation,
   scratch files, changes made only for diagnosis (`git checkout -- <file>`).
3. Confirm only the fix, the regression fixture/test, and the changeset remain.
4. Do NOT commit or push. `triage/` is gitignored and will not appear.
