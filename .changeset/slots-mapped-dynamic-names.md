---
"@astrojs/compiler-rs": patch
---

Fixes a `ReferenceError` when a slot's name is computed inside a loop, e.g. `{items.map((item, i) => <div slot={`item-${i}`}>{item}</div>)}`. Computed slot names now render into their matching named slot.
