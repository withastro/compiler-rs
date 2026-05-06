---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes `Unexpected token` error when an HTML attribute has an unquoted value such as a number (`<input maxlength=255>`) or contains characters like `-` or `#`.
