---
"@astrojs/astro2tsx": patch
---

Fixes `<?xml ... ?>` and other processing instructions in templates. astro2tsx used to copy them into the TSX as-is, which TypeScript can't parse. They now print as JSX comments.
