---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes the compiler sometimes adding extra whitespace between root elements when one of the root elements would be hoisted (e.g. style tags, scripts etc.)
