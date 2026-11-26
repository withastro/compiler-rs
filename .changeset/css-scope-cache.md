---
'@astrojs/compiler-binding': minor
'@astrojs/compiler-rs': minor
---

Adds a `cssScopeCache` option to `transform()`. With `cssScopeCache: { maxBytes }`, the scoped text of each `<style>` is kept in a process-wide cache keyed by its CSS, its scope hash and the scoped style strategy, so recompiling a component whose styles did not change skips parsing and printing them. The cache is cleared when it holds more than `maxBytes`. It is off by default (absent or `maxBytes: 0`) and the output is byte-identical either way.
