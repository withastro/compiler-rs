---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Statically link the Visual C++ runtime for the Windows ARM64 binding so it no longer requires `VCRUNTIME140.dll` at runtime.
