---
"@astrojs/compiler-rs": patch
---

Fixes `define:vars` dropping the injected `style` attribute when the element is a custom element (e.g. `<my-element>`). Custom elements now receive the CSS custom properties like any other HTML element.
