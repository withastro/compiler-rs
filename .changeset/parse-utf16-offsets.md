---
"@astrojs/compiler-binding": minor
"@astrojs/compiler-rs": minor
---

`parse()` now reports node and comment positions as UTF-16 offsets, so they line up with JavaScript string indices. Previously they were UTF-8 byte offsets, which drifted on any source containing non-ASCII characters.
