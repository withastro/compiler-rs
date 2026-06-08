---
"@astrojs/compiler-binding": patch
"@astrojs/compiler-rs": patch
---

Fixes `compact: 'jsx'` stripping significant same-line whitespace. JSX whitespace is now only trimmed where it borders a line break (matching React/Babel's rules), so a space at a text/expression, text/element, or element/element boundary is preserved. `<h1>Page {n}</h1>` now keeps its space (`Page 1`), as does `<span>hello</span> <em>world</em>`. Whitespace adjacent to newlines is still collapsed.
