---
"@astrojs/astro2tsx": patch
---

Stop emitting a synthetic named export for components: the generated default export now uses the public component name directly, and only falls back to the internal suffixed name plus an alias when the frontmatter declares or imports that public name as a value binding.
