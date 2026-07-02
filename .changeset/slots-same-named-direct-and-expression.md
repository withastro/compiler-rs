---
"@astrojs/compiler-rs": patch
---

Fixes a direct element and an expression slot targeting the same named slot, such as `<div slot="x">A</div>{b && <div slot="x">B</div>}`, rendering only the last one. Both now render into the slot, matching same-named expression slots.
