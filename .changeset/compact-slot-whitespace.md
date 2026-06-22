---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes compact mode not collapsing whitespace inside component and custom-element slot content. With `compact: 'jsx'` (Astro 7's `compressHTML`), leading newlines and indentation in slotted children are now trimmed the same way as regular template children, while meaningful same-line spacing is preserved.
