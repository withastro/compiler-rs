# @astrojs/astro2tsx

## 0.1.2

### Patch Changes

- 22b0de0: Add a completion mapping for Volar when an Astro document has no frontmatter.
- d61b277: Fix incorrect parsing when transforming Astro files with regex literals starting with `>` in frontmatter.
- 60ba11a: Map generated frontmatter terminator text to its source boundary for editor tooling.

## 0.1.1

### Patch Changes

- 7819910: Emit `ExtractedStyleType` as a runtime `enum` instead of a `const enum` in the generated `.d.ts`
