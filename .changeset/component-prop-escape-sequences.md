---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes escape sequences such as `\n` not being interpreted in quoted component props, so `<Component prop="a\nb" />` passes a real line break again. Backslashes in a quoted prop are read as JavaScript escapes, so a literal one may need doubling (`prop="C:\\temp"`).
