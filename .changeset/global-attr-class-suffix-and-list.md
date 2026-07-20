---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes two `:global()` scoping mismatches with the Go compiler. An attribute or class suffixed onto a leading `:global()` (`:global(a)[role="button"]`, `:global(a).cls`) is no longer mis-scoped and now stays global. A selector list inside `:global()` (`.x :global(ul, ol)`) no longer drops later items; the scope prefix is distributed across every item instead.
