---
name: writing-comments
description: How to write inline (//), rustdoc item (///), and module (//!) comments in the compiler-rs codebase, plus JSDoc in the TypeScript wrapper — for contributors reading the source, not end users. Use whenever writing or editing comments in Rust or TypeScript source, including comments added incidentally while fixing a bug or building a feature.
---

# Writing Comments

## Purpose

Comments and doc comments in this repository are read by contributors, months
or years after they were written, with none of the context you have right now.
This skill defines who that reader is, what each kind of comment is for, and
which patterns are banned. It applies to both the Rust crates (`crates/`) and
the TypeScript wrapper (`packages/compiler/`).

## Scope Boundary

This skill governs **contributor-facing** comments in the source a contributor
reads at HEAD. It does **not** apply to end-user documentation:

- The NAPI type definitions at
  [`crates/astro_napi/index.d.ts`](../../../crates/astro_napi/index.d.ts) are
  **generated** by `napi build`. Never hand-edit them or their comments; change
  the `#[napi]` Rust source and rebuild instead.
- Doc comments on the public `@astrojs/compiler-rs` API in
  [`packages/compiler`](../../../packages/compiler) are surfaced to npm users
  through editor IntelliSense. Write those for compiler **users** calling
  `transform`, not for contributors reading the source.

Everything below is about the source a contributor reads at HEAD.

## The Reader

Write for a compiler-rs contributor who is competent in Rust (and TypeScript for
the wrapper) but has **no access to your current context**: not this
conversation, not the pull request, not the issue, not the diff. They see only
the repository at HEAD.

Two consequences follow directly:

1. **Never narrate change history.** Words like "now", "previously", "no
   longer", "the new approach" are meaningless at HEAD, where only one approach
   exists. State how the code works, not how it came to be.
2. **Never address the reviewer.** A comment that argues your change is correct
   ("this properly handles X") belongs in the PR description, not in the source.
   The comment must justify the code as it stands, permanently.

## Three Kinds of Comments, Three Different Jobs

| Kind                 | Syntax                             | Job         | Contains                                                                                       |
| -------------------- | ---------------------------------- | ----------- | ---------------------------------------------------------------------------------------------- |
| Module docs          | `//!` at the top of a file/module  | Explanation | Why the module exists, the concepts and terms it defines, how the pieces relate, design rationale |
| Item docs            | `///` directly above a declaration | Reference   | The contract: behavior, inputs/outputs, invariants, panics, errors. Neutral and factual        |
| Inline comments      | `//` inside a body                 | Rationale   | Only what the code cannot say: constraints, workarounds (with issue links), non-obvious coupling |

In the TypeScript wrapper the mapping is the same, with `/** */` playing the
role of `///`/`//!` and `//` for rationale.

Do not mix the jobs. Implementation details do not belong in the `///` contract
— put them as `//` comments inside the body. The contract does not belong
scattered across inline comments — put it on the declaration.

The `//!` docs at the top of
[`crates/astro_codegen/src/lib.rs`](../../../crates/astro_codegen/src/lib.rs)
show the target register: they state what the crate produces and give the shape
of the generated output, in the present tense, without defending a change. The
`AwaitDetector` in
[`crates/astro_codegen/src/printer/mod.rs`](../../../crates/astro_codegen/src/printer/mod.rs)
shows inline rationale doing its job — it explains why scanning the whole
subtree is sound ("can over-mark but never under-mark") rather than restating
what the visitor does.

## The Deletion Test

Before writing any comment, ask: **does this state something the reader cannot
recover from the code itself?**

- If the information is already carried by names, types, or structure, do not
  write the comment. If the name fails to carry it, improve the name.
- Information that legitimately needs a comment: an invariant, a rationale, a
  coupling to code elsewhere, a workaround with a link, surprising behavior of a
  dependency, a term of art the module defines.

When editing later, the same test applies in reverse: a comment that no longer
passes it should be deleted, not left to rot.

## Link to the Issue for Workarounds

Any comment that explains a workaround, a `HACK`, a regression guard, or
surprising behavior of a dependency **must link the GitHub issue or PR** that
motivates it. The link is what lets a future reader tell whether the workaround
is still needed. Much of the parser and codegen here is constrained by Astro's
[oxc fork](https://github.com/withastro/oxc); when its behavior forces your
hand, cite the source.

```rust
// oxc parses the fragment shorthand as a JSX namespace; unwrap it back to a
// fragment so downstream printing matches Astro's runtime. See
// https://github.com/withastro/compiler-rs/issues/123
```

A workaround with no link is indistinguishable from a mistake.

## Banned Patterns

**Narrating the next line.** Delete these on sight:

```rust
// Increment the generation counter
generation += 1;
```

**Change-history narration.** Rewrite as present-tense rationale:

```rust
// BAD: We now intern the scope hash instead of recomputing it.
// GOOD: The scope hash is interned because every element lookup reads it.
```

**Reviewer-addressed justification.** Move the argument to the PR:

```rust
// BAD: This correctly handles the self-closing component from the bug report.
// GOOD: Components self-close in the AST but must emit an explicit closing tag,
//       so the runtime can inject slotted children.
```

**Restated rustdoc.** A `///` doc that rewords the item name says nothing:

```rust
// BAD:
/// Scans the styles.
fn scan_styles(...)

// GOOD:
/// Collects every `<style>` block in the template into `ScanResult`, recording
/// whether each is scoped so the printer can hash the scope id once.
fn scan_styles(...)
```

**Vague hedging.** "Some cases", "various reasons", "handles edge cases",
"etc." — either name them or drop the sentence.

**Emojis.** Banned everywhere in this repository, comments included.

**Ad-hoc section banners** (`// ----- helpers -----`, `// ==== TYPES ====`).
Use the region markers below instead.

## Region Comments

Long files and large `impl`/`trait` blocks group related items with paired
region markers, which editors fold on:

```rust
// #region printing components
...
// #endregion
```

This is the convention for navigation in this codebase. Rules:

- Every `// #region` has a matching `// #endregion`. An unpaired marker breaks
  editor folding silently.
- The name states what the group contains. It can be a plain label
  (`shared helpers`) or anchored to a function
  (`#region collapse_html`) when the region holds one entry point and its
  private support code.
- Use regions only where they earn their keep: files or `impl`/`trait` blocks
  long enough that folding helps navigation. A file that fits on two screens
  does not need them.
- A region name is organization, not documentation. It never substitutes for
  rustdoc on the items inside it.

## Conventions in This Codebase

**Rustdoc sections.** State the contract with the standard headings when they
apply: `# Errors` for what a `Result`-returning function returns on failure,
`# Panics` for the conditions that trigger a panic, `# Safety` for the
invariants an `unsafe fn` requires, and `# Examples` with a fenced ` ```rust `
block for non-obvious usage. A doctest in `# Examples` is compiled and run by
`cargo test`, so keep it building.

**Intra-doc links.** Reference other items with bracketed paths
(`` [`AstroCodegen`] ``, `` [`ScanResult`] ``) rather than a bare name, so a
rename breaks the build instead of rotting silently and editors can jump to the
target.

**TODO.** Use `// TODO:` for deferred work and link an issue when one tracks it.
There is no `FIXME` idiom in this codebase — do not introduce one.

**TypeScript wrapper.** In `packages/compiler`, `@param name - description`,
`@returns`, and `@throws` state the contract; `{@link Symbol}` cross-references;
`@deprecated` states the migration and the removal horizon. Remember the scope
boundary: JSDoc on the exported API is end-user documentation.

## Editing Existing Code

- Preserve existing comments. If your change alters behavior, extend or correct
  the specific prose — never replace it with generic text. Deleting hard-won
  context is worse than leaving a comment slightly stale.
- When your change makes a comment false, fix it in the same diff. A stale
  comment is worse than none.
- Match the surrounding density. A heavily documented module deserves the same
  level on new items; do not blanket a sparse module with comments.

## Self-Check Before Finishing

After completing any task that touched comments, re-read **only the comments in
your diff**, in isolation from the code changes:

1. Does each one pass the deletion test?
2. Does any reference the conversation, the change itself, or the reviewer?
3. Does every workaround link its issue or PR?
4. Would a reader without access to the diff understand each one?

Fix or delete what fails. Deletion is the default; a missing comment is cheaper
than a misleading one.

## References

- [Diátaxis](https://diataxis.fr/) — the framework behind the explanation /
  reference / rationale split above.
- [The rustdoc book](https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html)
  — conventions for `///` and `//!` documentation.
- [TSDoc](https://tsdoc.org/) — the tag reference for the TypeScript wrapper.
