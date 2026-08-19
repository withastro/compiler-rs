---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes escape sequences such as `\n` not being interpreted in quoted component props, so `<Component prop="a\nb" />` passes a real line break again. A backslash that forms a JavaScript escape needs doubling to stay literal (`prop="C:\\temp"`), while one that does not, such as `pattern="^example\.com$"`, is left alone.
