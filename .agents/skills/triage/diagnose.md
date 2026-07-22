# Diagnose

Find the root cause of a reproduced bug in the compiler's Rust source.

**CRITICAL: You MUST always read `report.md` and append to it before finishing,
regardless of outcome. Even if the investigation is inconclusive, always record
what you found. Downstream skills depend on `report.md`.**

**SCOPE: Diagnosis only. Do NOT fix the bug or write a regression test. Do not
spawn sub-agents.**

## Prerequisites

- **`triageDir`** — Fixed scratch directory `triage/current` (gitignored). The
  harness does not pass this; always use `triage/current`.
- **`report.md`** — At `triage/current/report.md`, written by reproduce. Full
  prior context.
- **`issueDetails`** — The GitHub issue payload, if you need to re-read it.

## The compilation pipeline

Trace the defect to the stage that owns it:

1. **Parse** — Astro's fork of `oxc_parser` turns `.astro` into an AST. This
   lives in the pinned [`withastro/oxc`](https://github.com/withastro/oxc) fork,
   **not in this repo**. A parse-stage bug (malformed AST, a parse error the
   harness prints as `Parse errors: ...`, wrong spans) cannot be fixed here —
   note it and hand off; the fix step documents the out-of-repo cause.
2. **Scan** — `AstroScanner` (`crates/astro_codegen/src/scanner.rs`) walks the
   AST once and collects metadata: hydrated / client-only / server components,
   hoisted scripts, styles, `uses_astro_global`, `has_await`, etc.
3. **Print** — `AstroCodegen` (`crates/astro_codegen/src/printer/`) emits the
   runtime JS from the AST + scan results.
4. **Reprint** — `build()` runs the printed JS back through `oxc_codegen` to
   strip TypeScript and normalize formatting. **The final `code` string comes
   from this reprint**, so tab indentation and multi-line object shape are set
   here, not by the printer's own `print`/`println`. Do not chase a formatting
   difference into the printer when the reprint produced it.

## Step 1: Review the reproduction

Read `report.md`. **Skip if not reproduced:** if it shows the bug was not
reproduced or was skipped, append `DIAGNOSIS SKIPPED: no reproduction` and
return `confidence: null`.

Re-run to see the failure yourself:

```bash
INSTA_UPDATE=new cargo test -p astro_codegen snapshots   # then read the fixture's .snap.new
```

## Step 2: Locate the source

Map the symptom to the owning module under `crates/astro_codegen/src/`:

- **`scanner.rs`** — wrong/missing component metadata, hoisted scripts, `has_await`
- **`printer/mod.rs`** — top-level codegen, `$$metadata`, component wrapper, the
  `oxc_codegen` reprint
- **`printer/elements.rs`** — element/attribute/tag emission
- **`printer/expressions.rs`** — `{...}` expressions, template literals
- **`printer/escape.rs`** — string/attribute escaping
- **`printer/whitespace.rs`** — whitespace collapsing
- **`css_scoping.rs`**, **`style.rs`** — scope hashing and `<style>` handling
- **`options.rs`** — how a `TransformOptions` value changes output
- **`diagnostic.rs`** — diagnostics (note: codegen currently emits none of its
  own; a missing-diagnostic bug is an unimplemented feature, not a regression)

## Step 3: Instrument

Add temporary tracing and run the narrowest test:

```rust
eprintln!("[TRIAGE] node = {node:#?}");
dbg!(&scan_result.hydrated_components);
```

```bash
cargo test -p astro_codegen <specific_test_name> -- --nocapture
```

Iterate until you understand which code path runs, what data flows through it,
and where it diverges from correct behavior. Then **revert every instrumentation
edit** with `git checkout -- <file>` — debug output must not leak downstream.

## Step 4: Identify the root cause

Document:

1. **Which file(s)** and line(s) hold the defect
2. **What the code does wrong** — the specific logic error
3. **Why it produces the observed output/crash**
4. **What the fix should be** — high level

Consider whether it is a recently introduced regression, whether it affects
adjacent constructs, and what edge cases a fix must respect (the 380+ existing
snapshot fixtures encode behavior you must not break).

**Tone:** describe the cause factually. A missing branch is a missing branch,
not a "critical flaw." The goal is to orient a maintainer, not to alarm.

## Step 5: Append to `report.md`

Add a diagnosis section: root cause, affected files with line numbers, the code
path, what your instrumentation showed, the suggested fix approach, and a
**confidence** level (`high`, `medium`, or `low`) with any caveats. If the cause
is in the `oxc` parser fork, say so explicitly and set confidence accordingly.
