---
"@astrojs/compiler-rs": patch
---

Fixes two conditional slot patterns that produced broken output:

- A guarded conditional such as `{show && (cond ? <a slot="s">A</a> : <b slot="s">B</b>)}` now places its content in the right slot and keeps the original condition intact.
- A conditional fragment such as `{cond && <>…</>}` now renders as the component's default content instead of emitting broken output.
