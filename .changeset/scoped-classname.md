---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Merge scoped styles into `className` for component tags

`scopedStyleStrategy` values that emit a scope class ("class" and "where") now merge the scope class into a component's `className` prop, matching the legacy compiler's behavior for React components.
