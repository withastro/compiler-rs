---
"@astrojs/compiler-rs": patch
---

Fix two conditional-slot cases:

- `{a && (b ? <x slot="s"/> : <y slot="s"/>)}` now wraps its branches as slot objects (instead of rendering them as raw HTML into `$$mergeSlots`) and preserves the parentheses so it is not re-bound as `(a && b) ? ...`.
- A bare `<>` fragment in an expression (`{cond && <>…</>}`) is now treated as the parent's default content, matching the previous compiler, instead of producing broken `$$mergeSlots` output.
