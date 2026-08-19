---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes `:global()` not being stripped from the CSS output when nested inside a pseudo-class such as `:has()`, `:is()`, `:where()` or `:not()`.
