---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Strip `:global()` wrappers nested inside `:has()`, `:is()`, `:where()`, and `:not()` in scoped CSS.
