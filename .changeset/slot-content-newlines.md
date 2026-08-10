---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes newlines being lost in text passed directly between a component's tags, so a `<slot />` wrapped in `<pre>` keeps its line breaks.
