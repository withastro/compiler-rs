---
"@astrojs/compiler-rs": patch
---

Fixes multiple conditional elements targeting the same named slot, such as `{a && <div slot="x">A</div>}{b && <div slot="x">B</div>}`, rendering only the last one. All of them now render into the slot.
