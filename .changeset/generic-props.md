---
"@astrojs/astro2tsx": patch
"@astrojs/compiler-rs": patch
"@astrojs/compiler-binding": patch
---

Use `Props<any>` instead of `Parameters<typeof ...>[0]` for generic `Props` in `Astro` global declarations, fixing `astro check` errors when using the `Polymorphic` type.
