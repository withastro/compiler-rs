# @astrojs/astro2tsx

## 0.1.3

### Patch Changes

- 266acf4: Fixes an issue where processing instructions weren't correctly parsed, resulting in an error. Now the following code is correctly transformed:
  
  ```html
  <?xml version="1.0" encoding="UTF-8"?>
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" {...Astro.props}>
  	<circle cx="5" cy="5" r="4" />
  </svg>
  ```
- 089b077: Updates Astro’s `oxc` fork to the latest commit

## 0.1.2

### Patch Changes

- 22b0de0: Add a completion mapping for Volar when an Astro document has no frontmatter.
- d61b277: Fix incorrect parsing when transforming Astro files with regex literals starting with `>` in frontmatter.
- 60ba11a: Map generated frontmatter terminator text to its source boundary for editor tooling.

## 0.1.1

### Patch Changes

- 7819910: Emit `ExtractedStyleType` as a runtime `enum` instead of a `const enum` in the generated `.d.ts`
