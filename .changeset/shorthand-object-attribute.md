---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes a parse error on `{{ ... }}` shorthand attributes containing an object expression, such as `<Debug {{ answer: sum(2, 4) }} />`. These now compile correctly instead of suggesting to use a spread attribute.
