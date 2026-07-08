---
"@astrojs/compiler-rs": patch
---

Strips the redundant `slot="…"` attribute from dynamically-named slots on regular components — a single slot (`{items.map(item => <div slot={outerName}>…</div>)}`, `{cond ? <div slot={name}/> : null}`) or a multi-slot ternary (`{cond ? <a slot={x}/> : <b slot={y}/>}`). The name already lives in the generated slot object's key, so the attribute was dead output. Custom elements and root-level slots still keep the attribute, since there the browser does the slotting. (Go strips the single-slot cases but leaves it on the multi-slot ternary — an inconsistency with its own handling that we don't mirror.)
