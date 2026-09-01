---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Include `@emnapi/core` and `@emnapi/runtime` as dependencies of the generated wasm32-wasi binding package so strict peer-dependency checks no longer fail.
