---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes several cases where `await` failed to make the generated code `async`, producing invalid JavaScript.
