---
"@astrojs/compiler-rs": patch
---

Fix two sibling expression slots with the same name (e.g. `{a && <div slot="x"/>}{b && <div slot="x"/>}`) emitting a duplicate object key, which dropped all but the last at runtime. They now group under a single slot key, matching direct element slots.
