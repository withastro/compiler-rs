---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes several cases of adjacent JSX elements inside a `{...}` expression acting weirdly, including:

- HTML comments between the elements now render instead of being dropped, including when a comment is on its own line.
- Whitespace between the elements is preserved.
- A `<script>` that is not the first element no longer fails to parse when its body contains an HTML closing tag inside a template literal.
