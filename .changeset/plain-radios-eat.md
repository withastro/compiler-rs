---
"@astrojs/astro2tsx": patch
---

Fixes an issue where processing instructions weren't correctly parsed, resulting in an error. Now the following code is correctly transformed:

```html
<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" {...Astro.props}>
	<circle cx="5" cy="5" r="4" />
</svg>
```
