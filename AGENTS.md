# Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:

- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

# Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

# Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:

- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:

- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

# Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:

- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:

```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

# Style Guide

- This is a mixed Rust + TypeScript repo. Follow the conventions and patterns you detect in the surrounding code.
- **Rust** is formatted by `cargo fmt` (rustfmt) and linted by `cargo clippy`. Indentation is 4 spaces; the edition is `2024`. CI runs `cargo fmt -- --check` and `cargo clippy -- -D warnings`, so clippy warnings fail the build.
- **JavaScript/TypeScript** is formatted and linted by [Biome](./biome.jsonc): tabs at width 2, line width 100, single quotes, semicolons, trailing commas. `.astro` files are excluded from Biome formatting.
- Run `pnpm format` to auto-format the whole repo (`biome format --write` + `cargo fmt` + import sorting).
- Run `pnpm lint` to lint the whole repo (`biome lint` + `cargo clippy`). `pnpm lint:fix` applies safe fixes.
- The Rust toolchain is pinned in [`rust-toolchain.toml`](./rust-toolchain.toml); do not override the channel.

# Writing Comments

These rules apply to **every** comment you write, including ones added incidentally while fixing a bug. Full guidance with examples: [`.agents/skills/writing-comments/SKILL.md`](./.agents/skills/writing-comments/SKILL.md).

- Write for a contributor reading the code at HEAD, months later, with no access to this conversation, the PR, or the diff.
- Never narrate change history ("now", "previously", "no longer") and never address the reviewer ("this correctly handles..."). State how the code works, not how it came to be or why the change is right.
- Deletion test: a comment must state something the reader cannot recover from the code. If names or types already carry it, don't write it.
- `///` item docs and `//!` module docs state the contract (behavior, inputs/outputs, invariants, panics, errors); `//` comments carry rationale only. Anchor a workaround to the GitHub issue or PR that motivates it.
- Group long files or `impl`/`trait` blocks with paired `// #region` / `// #endregion` markers instead of ad-hoc `// ---- section ----` banners.
- When your change alters documented behavior, extend or correct the existing prose — never replace specific docs with generic text.
- No emojis anywhere in source, comments included.
- Exception: the generated NAPI type definitions at `crates/astro_napi/index.d.ts` are generated — don't hand-edit them. Doc comments on the public `@astrojs/compiler-rs` API in `packages/compiler` are surfaced to npm users through IntelliSense; write those for compiler **users**, not contributors.

# Environment Guide

- Use `node -e` for scripting tasks, not `python` or `python3`.
- After changing Rust source, rebuild the native addon with `pnpm run build:napi` before the TypeScript tests can exercise it — the `.node` binary does not hot-reload.

# Monorepo Structure

This directory is a Git monorepo containing both Rust crates and a `pnpm` workspace. It is the Astro compiler rewritten in Rust.

- **Rust crates** live in `crates/`:
  - `crates/astro_codegen/` — the core engine that transforms an Astro AST into JavaScript.
  - `crates/astro_napi/` — Node.js [NAPI](https://napi.rs/) bindings that expose the compiler to JavaScript. Building it emits an `astro.*.node` (or `astro.*.wasm`) native addon.
- **npm packages** live in `packages/`:
  - `packages/compiler/` — the `@astrojs/compiler-rs` package, a TypeScript wrapper around the NAPI bindings.
- The `pnpm` workspace spans `packages/*` and `crates/astro_napi` (see [`pnpm-workspace.yaml`](./pnpm-workspace.yaml)).
- The parser and code generator come from Astro's [oxc fork](https://github.com/withastro/oxc) (`oxc_*` crates), pinned to a single git revision in [`Cargo.toml`](./Cargo.toml). Several of those crates carry Astro-specific changes; keep every `oxc_*` dependency on the same revision to avoid duplicate-crate errors.

Edits to Rust source take effect in the JS API only after rebuilding the NAPI addon (`pnpm run build:napi`).

# Building

```shell
# Build the NAPI native addon (debug mode)
pnpm run build:napi

# Build the TypeScript package
pnpm run build:compiler

# Build everything
pnpm run build:all
```

# Running Tests

## Rust tests

```shell
# Run all Rust tests (unit + snapshot)
cargo test

# Run only astro_codegen tests
cargo test -p astro_codegen

# Review snapshot changes after a diff
cargo insta review
```

Snapshot tests use [`insta`](https://insta.rs/). To add a case, drop a new `.astro` fixture in `crates/astro_codegen/tests/fixtures/`, run `cargo test -p astro_codegen`, then review the generated `.snap` with `cargo insta review`. Benchmarks use `divan` under `crates/*/benches/`.

## TypeScript tests

```shell
# Build the addon first, then run the package tests
pnpm run build:napi
pnpm test
```

`pnpm test` runs the `@astrojs/compiler-rs` suite (`node --import tsx --test 'packages/compiler/test/**/*.ts'`). The NAPI crate has its own binding tests, run with `pnpm test` from `crates/astro_napi`.

# Compiler Quick Reference

The compilation pipeline has three stages:

1. **Parsing** — Astro's fork of `oxc_parser` (the [oxc fork](https://github.com/withastro/oxc) linked above) parses a `.astro` file into an AST. Upstream oxc only parses JS/TS; the fork adds the `.astro` grammar.
2. **Scanning** — `AstroScanner` pre-analyzes the AST to collect metadata (hydrated components, hoisted scripts, styles).
3. **Printing** — `AstroCodegen` generates Astro-runtime-compatible JavaScript from the AST.

The public entry points are re-exported from [`crates/astro_codegen/src/lib.rs`](./crates/astro_codegen/src/lib.rs) (`transform`, `AstroCodegen`, `AstroScanner`, `TransformOptions`).

# Deep Dives

- The `.astro` language surface the compiler accepts is specified in [`docs/SYNTAX_SPEC.md`](./docs/SYNTAX_SPEC.md). Read it before changing parsing or printing behavior.
