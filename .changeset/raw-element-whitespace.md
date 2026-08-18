---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes whitespace inside `<pre>`, `<textarea>`, and `is:raw` elements being lost when the element also contains a `<style>` or `<script>`.
