---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Avoid illegal CPU instructions on pre-Haswell x86_64 and respect `NAPI_RS_FORCE_WASI` before loading the native binding.
