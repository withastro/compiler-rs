# Reproduce

Reproduce a `@astrojs/compiler-rs` bug: turn the report into a minimal `.astro`
input that demonstrates wrong compiler output (or a crash).

**CRITICAL: You MUST always write `report.md` to the triage directory before
finishing, regardless of outcome. Even if you cannot reproduce, hit errors, or
need to skip — always write `report.md`. Downstream skills have NO access to the
original issue; `report.md` is their only source of context. If you finish
without writing it, the pipeline fails silently.**

**SCOPE: Reproduction only. Do NOT diagnose, fix, or add a permanent regression
test here. Do not spawn sub-agents.**

## Prerequisites

- **`triageDir`** — Fixed scratch directory `triage/current` (gitignored, shared
  across stages). The harness does not pass this; always use `triage/current` so
  the later stages, which are not given the issue number, find your `report.md`.
- **`issueDetails`** — The GitHub issue payload. If missing, run
  `gh issue view <number> --comments` to load it.

## What a reproduction looks like here

The product is a compiler: `.astro` in, JavaScript out. A reproduction is a
**minimal `.astro` source** plus a statement of what is wrong with the output.
Classify the bug:

- **Wrong output** — the emitted `code` (or `map`, `scope`, `css`, or a
  `*Components`/`containsHead`/`propagation` field) differs from what it should
  be.
- **Crash** — `transform` panics on the input.

Reporters often attach a whole Astro project or a StackBlitz link. That is a
symptom carrier, not the reproduction. **Reduce it** to the smallest `.astro`
snippet whose compiler output shows the defect. If the failure depends on a
`TransformOptions` value (e.g. `compact`, `filename`, `resolvePath`), record it.

## Step 1: Confirm the report

Read `issueDetails`. Extract:

- The `.astro` source (or the snippet inside the linked project) that triggers it
- What the reporter expected the output/behavior to be
- What actually happens (error text, wrong JS, panic message)
- Any `TransformOptions` in play (from `astro.config`, the reporter's call, etc.)

## Step 2: Check for early-exit conditions

If any condition below is met, skip to Step 5, write `report.md`, and stop.
Report the reason using **one of these exact tokens** (the orchestrator only
understands this fixed set):

- **`not-actionable`** — Not a bug report: feature request, question, or a
  request to change intended output. (→ `triage: not actionable`)
- **`missing-details`** — No `.astro` input is given and none can be derived,
  or there is no statement of the expected output. Both are required to
  reproduce and later verify. (→ `triage: needs reproduction`)
- **`host-specific`** — Reused here to mean **out of scope for the compiler**:
  the defect is in the Astro runtime/renderer, an adapter, a bundler
  integration, or another package — the compiler's JS output is correct. If you
  can show the emitted JS is right and the fault is downstream, exit with this.
  (→ `triage: skipped`)
- **`unsupported-version`** — The issue targets the legacy Go `@astrojs/compiler`
  (not `@astrojs/compiler-rs`), or a version/surface this repo does not produce.
  (→ `triage: skipped`)
- **`maintainer-override`** — A maintainer commented that this should not be
  auto-triaged. Check `authorAssociation` on comments for `MEMBER`,
  `COLLABORATOR`, or `OWNER`. (→ `triage: skipped`)

**Comment handling:** an early exit is only valid if no later comment
invalidates it (e.g. a follow-up comment adds the missing `.astro`, or shows the
fault really is in compiler output). Re-read the full thread before exiting.

## Step 3: Build the minimal input

Prefer the **Rust snapshot harness** — it is the fastest path and needs no
native-addon rebuild.

1. Write the minimal source to a fixture:
   `crates/astro_codegen/tests/fixtures/_<issue#>_<short_desc>.astro`
   (the `_<issue#>_` prefix is this repo's convention for regression fixtures).
2. If the bug requires non-default options, add `// @config` lines at the very
   top. Only `compact=html|jsx|false` is supported by the harness today; for any
   other option, reproduce through the JS API instead (see below).

Keep the source as small as possible — only what is needed to trigger the
defect.

## Step 4: Observe the actual output

Run the snapshot test to generate output without committing to a baseline.
Under CI, `insta` will not write pending snapshots unless you force it, so set
`INSTA_UPDATE=new`:

```bash
INSTA_UPDATE=new cargo test -p astro_codegen snapshots
```

A brand-new fixture has no `.snap`, so the test reports a new snapshot (it exits
non-zero — that is expected) and writes a pending `<name>.snap.new` next to the
fixture. `*.snap.new` is gitignored. Read it to see the **actual** compiler
output:

```bash
cat crates/astro_codegen/tests/fixtures/_<issue#>_<short_desc>.snap.new
```

- **Wrong output:** compare the actual output to the reporter's expected output.
  Confirm the difference is real and matches the report.
- **Crash:** a panic in `transform` aborts the test run (the harness only
  converts *parse* errors into text). Re-run with `RUST_BACKTRACE=1` and capture
  the panic message and backtrace.

**Do not accept the snapshot** (`cargo insta accept` / `INSTA_UPDATE`) here —
that belongs to the fix step. Leave the `.astro` fixture in place for downstream
steps; the pending `.snap.new` is ignored by git.

### When the Rust harness is not enough

Some bugs live in the TypeScript wrapper (`resolvePath`, `preprocessStyles`) or
need options the harness cannot set. Reproduce through the JS API instead:

```bash
pnpm run build:napi   # rebuild the native addon; it does NOT hot-reload
node --input-type=module -e "import { transform } from '@astrojs/compiler-rs'; \
  const r = await transform(SRC, { /* options */ }); console.log(r.code);"
```

The addon must be rebuilt after any Rust change before the JS layer sees it.

### False-positive guard

Confirm the defect is the reported one: a trivially-correct variant of the input
should compile cleanly, so you know the difference is caused by the reported
construct and not your setup.

## Step 5: Write `report.md`

Write `report.md` to `triageDir`. This is context for the next stages, not a
human comment — include too much rather than too little. Cover:

- Original issue title, body, and any clarifying comments
- The minimal `.astro` source and any required `TransformOptions` / `@config`
- The fixture path you created (if any)
- **Actual** output (from `.snap.new` or the API) and the **expected** output,
  or the full panic message + backtrace for a crash
- Outcome: **reproduced**, **not reproduced**, or **skipped** — and for skipped,
  the exact token from Step 2 and why
- If you early-exit as `not-actionable`, `missing-details`, `host-specific`, or
  `unsupported-version`, delete the stray `.astro` fixture you created so it does
  not land on the branch.
