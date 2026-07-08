---
"@astrojs/compiler-rs": patch
---

Fixes a `${...}` inside a `set:text` or `transition:*` attribute value (e.g. `set:text="${x}"`) being treated as code instead of literal text, which could break the build or run unintended expressions. The value is now emitted verbatim.
