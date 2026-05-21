---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes `Unterminated string literal` error when a quoted attribute on a component contains literal newlines (e.g. multi-line `class`).
