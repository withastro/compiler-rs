---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixed several cases where `await` failed to make the generated code `async`, producing invalid JavaScript:

- `await` inside a `<>...</>` fragment used within an expression (e.g. `{cond && <>...</>}` or `{items.map((item) => <>...</>)}`) no longer lands in a non-`async` slot callback (issue #46).
- `for await (... of ...)` and `await using` in frontmatter now correctly make the component factory `async` (issue #47). Previously only standalone `await` expressions were detected.

Slot callbacks are now marked `async` based on whether that slot's own body uses `await`, instead of whenever the file contains `await` anywhere, avoiding redundant `async` on sibling slots that don't need it.
