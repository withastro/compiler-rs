---
"@astrojs/compiler-rs": patch
---

Fixes a `ReferenceError` when a slot name is computed from a callback binding, e.g. `{items.map((item, i) => <div slot={`item-${i}`}>{item}</div>)}`. Each item now renders into its matching named slot instead of throwing.

Such expressions are collected into their slot objects at runtime, so it works regardless of how the elements are produced — `.map`, `.flatMap`, `.map(…).filter(…)` chains, `Array.from`, custom helpers, array literals (`i => [<a slot=…/>, <b slot=…/>]`), nesting to any depth, and `async` callbacks (`await` at any level). Arrow, block-body and `function`-expression callbacks and conditional returns are all handled. Shapes that don't yield slot objects (`.forEach`, `.join`) contribute nothing instead of throwing.
