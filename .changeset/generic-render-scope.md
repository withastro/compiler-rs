---
"@astrojs/astro2tsx": patch
---

Fixes `astro check` and TypeScript errors when using the `Polymorphic` type, without weakening the types of `Astro.props` in components with generic `Props`. The frontmatter and template of these components are now checked inside a scope where the type parameters stay generic.
