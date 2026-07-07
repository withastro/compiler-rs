---
"@astrojs/compiler-rs": patch
---

Fixes a `ReferenceError` when a slot name is computed from a callback binding, e.g. `{items.map((item, i) => <div slot={`item-${i}`}>{item}</div>)}`. Each item now renders into its matching named slot instead of throwing. This also covers nested maps (`groups.map(g => g.items.map(item => <div slot={item.id}>…))`) and async callbacks (including `await` inside the mapped content).
