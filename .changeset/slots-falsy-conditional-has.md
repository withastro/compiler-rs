---
"@astrojs/compiler-rs": patch
---

Fixes `Astro.slots.has()` reporting a slot as filled when its only `slot="..."` element sits inside a conditional that doesn't render it, such as `{cond ? <span slot="aside" /> : ...}` where every branch is currently false. Components that switch their layout based on `Astro.slots.has()` no longer render a spurious empty wrapper.
