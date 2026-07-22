---
name: triage
description: Triage a compiler-rs bug report. Reproduces the bug as a minimal `.astro` input, diagnoses the root cause in the Rust codegen, verifies whether the behavior is intentional, and attempts a fix. Use when asked to "triage issue #1234", "triage this bug", or similar.
---

# Triage

Triage a `@astrojs/compiler-rs` bug report: reproduce the bug, diagnose the root
cause, verify whether the behavior is intentional, and attempt a fix.

This project is the Astro compiler: it transforms `.astro` source into
JavaScript. A bug is almost always one of two things:

- **Wrong output** — `transform` returns, but the emitted JS (or `map`, `scope`,
  component metadata) differs from what it should be.
- **A crash** — `transform` panics on some input (in a release build this
  aborts the process; in tests it fails the test).

There is no dev server, no browser, no user application to run. The whole bug
lives in the `.astro` input and the compiler output.

## How this skill is invoked

The triage harness runs this skill **once per stage**, passing a `step` arg and
an `instructions` arg. Run **only** the stage named by `step`, then return its
structured result. Do not advance to the next stage yourself, and do not spawn
sub-agents — the harness drives the sequence and calls you again for the next
stage.

| `step`      | Follow                       | Return |
| ----------- | ---------------------------- | ------ |
| `reproduce` | [reproduce.md](reproduce.md) | `reproducible` (bool), `skipped` (bool), `skippedReason` (one of the exact tokens in reproduce.md, or null) |
| `diagnose`  | [diagnose.md](diagnose.md)   | `confidence` (`high`/`medium`/`low`, or null if not attempted) |
| `verify`    | [verify.md](verify.md)       | `verdict` (`bug`/`intended-behavior`/`unclear`), `confidence` (`high`/`medium`/`low`) |
| `fix`       | [fix.md](fix.md)             | `fixed` (bool), `commitMessage` (string, or null) |

The stages share one working tree that persists between calls, but each stage
starts with no memory of the previous one. Carry context forward by writing and
re-reading **`triage/current/report.md`**: reproduce creates it, and every later
stage reads it and appends to it. `triage/` is gitignored, so anything scratched
there never lands on the fix branch.

## Args

- **`step`** — Which stage to run (see the table above).
- **`issueDetails`** — The GitHub issue payload (title, body, comments). Use it
  directly as the bug report. If absent, fetch it with
  `gh issue view <number> --comments`.
- **`issueNumber`** — The issue number. Passed for `reproduce` (it goes in the
  regression-fixture filename); later stages read paths from `report.md`.
- **`instructions`** — A harness reminder to run only the current stage.

## What the harness does around you

The harness has already checked out the repo on a fix branch. After the `fix`
stage it stages, commits, and pushes any worktree changes, and — when a fix
landed — opens the pull request (via the `pr-writer` skill) and manages the
issue/PR labels. **Never commit, push, or open a PR yourself.** Leave the correct
files in the working tree (see fix.md) and return your result; anything left in
the tree that is not gitignored will be committed, so clean up scratch work.

## Don't get stuck on infrastructure

If the toolchain won't build, a dependency won't install, or a tool is missing —
bail out after 2 attempts and record what you have in `report.md`. A partial
report with solid findings beats burning the time budget fighting the
environment.
