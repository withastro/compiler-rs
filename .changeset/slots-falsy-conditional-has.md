---
"@astrojs/compiler-rs": patch
---

Fix `Astro.slots.has()` reporting a named slot as present for an all-falsy conditional (e.g. a nested ternary whose branches all carry `slot="x"`). Multiple slotted elements in one expression now route through `$$mergeSlots` so the runtime conditional decides presence, matching the previous compiler.
