---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Honor `NAPI_RS_FORCE_WASI` by skipping the native binding load attempt entirely, preventing Windows Smart App Control from blocking the unsigned `.node` before the WASI fallback runs.
