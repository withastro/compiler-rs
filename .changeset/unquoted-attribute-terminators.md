---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes unquoted attribute values containing a `/` failing to compile, such as URLs like `<a href=https://example.com/path>` or root-relative paths like `<img src=/logo.png>`. Values containing `=`, quotes, or backticks are also no longer cut short.
