---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes an issue where a `<script define:vars={...}>` element lost all of its other attributes (such as `data-astro-rerun`, `id`, or `nonce`) in the compiled output.
