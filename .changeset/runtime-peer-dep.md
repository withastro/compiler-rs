---
"@astrojs/astro2tsx": patch
---

Moves `@emnapi/runtime` from `devDependencies` to `dependencies` so package managers that do not auto-install peer dependencies (e.g. Yarn 1) can load `@astrojs/astro2tsx` without a manual install.
