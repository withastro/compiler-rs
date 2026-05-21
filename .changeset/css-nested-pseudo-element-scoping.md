---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes CSS scoping incorrectly adding a scope to `&::pseudo-element` selectors (e.g. `&::marker`, `&::before`) and to selectors that only reference `&` inside `:is()`/`:where()`/`:not()`/`:has()`.
