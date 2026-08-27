---
"@astrojs/compiler-rs": patch
---

Removes a stray `slot="…"` attribute the compiler left on elements passed to a component with a computed slot name. (Custom elements keep it, since the browser needs it for slotting.)
