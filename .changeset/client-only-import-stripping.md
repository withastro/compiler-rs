---
"@astrojs/compiler-rs": patch
---

Fixes `client:only` causing a component's import to be stripped when the same binding is still used elsewhere, such as another plain instance, a `<Scope.Component>` sharing the import, or a reference in the frontmatter.
