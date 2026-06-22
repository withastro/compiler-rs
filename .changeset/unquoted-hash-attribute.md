---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes an "Invalid Character" error on unquoted attribute values that start with `#` followed by a digit, such as `<div color=#18b218 />`. Unquoted hex colors now parse correctly.
