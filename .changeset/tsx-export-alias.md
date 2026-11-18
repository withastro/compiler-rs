---
"@astrojs/astro2tsx": patch
---

Stops emitting a synthetic named export for most components, so TypeScript reports invalid named imports such as `import { Footer } from './Footer.astro'`. The generated default export now uses the public component name directly. It falls back to the internal suffixed name plus a named alias only when the component's frontmatter or template uses that name, for example `Date.astro` calling `new Date()`, or when the name is `Astro` or `Fragment`.
