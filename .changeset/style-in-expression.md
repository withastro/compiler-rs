---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes a `<style>` nested in a `{...}` expression (e.g. `{cond && <style>…</style>}`) being silently dropped from the output.
