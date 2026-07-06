---
"@astrojs/compiler-rs": patch
---

Fixes `<slot>` fallback content ignoring `compressHTML`, so whitespace around an expression (e.g. `<slot name="canonical">\n  {cond ? '' : <link />}\n</slot>`) is now collapsed like regular template children instead of being emitted verbatim.
