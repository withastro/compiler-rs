---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes backslashes being dropped from attribute values, such as `<input pattern="^example\.com$" />` rendering as `^example.com$`. Backslashes are now kept in element attributes, component props, and `set:html`.
