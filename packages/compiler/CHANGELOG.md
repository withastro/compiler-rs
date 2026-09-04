# @astrojs/compiler-rs

## 0.4.1

### Patch Changes

- a9c4e94: Fixes an extra space appearing next to an HTML comment when `compressHTML` is enabled.
- e9a5c37: Fixes escape sequences such as `\n` not being interpreted in quoted component props, so `<Component prop="a\nb" />` passes a real line break again. A backslash that forms a JavaScript escape needs doubling to stay literal (`prop="C:\\temp"`), while one that does not, such as `pattern="^example\.com$"`, is left alone.
- a9c4e94: Fixes an extra space appearing next to components that contain a `<script>` when `compressHTML` is enabled.
- 3d6ee6d: Fixes invalid JavaScript being generated for elements with a backtick in an attribute name.
- 07160da: Fixes `:global()` not being stripped from the CSS output when nested inside a pseudo-class such as `:has()`, `:is()`, `:where()` or `:not()`.
- a9c4e94: Fixes whitespace inside `<pre>`, `<textarea>`, and `is:raw` elements being lost when the element also contains a `<style>` or `<script>`.
- efb16c9: Fixes newlines being lost in text passed directly between a component's tags, so a `<slot />` wrapped in `<pre>` keeps its line breaks.
- Updated dependencies [a9c4e94]
- Updated dependencies [e9a5c37]
- Updated dependencies [a9c4e94]
- Updated dependencies [35e2a04]
- Updated dependencies [3d6ee6d]
- Updated dependencies [07160da]
- Updated dependencies [a9c4e94]
- Updated dependencies [efb16c9]
  - @astrojs/compiler-binding@0.4.1

## 0.4.0

### Minor Changes

- bc10d43: Adds a `comments` array to the AST returned by `parse()`, covering comments in the frontmatter, in `<script>` blocks, and in the template.
- bc10d43: `parse()` now reports node and comment positions as UTF-16 offsets, so they line up with JavaScript string indices. Previously they were UTF-8 byte offsets, which drifted on any source containing non-ASCII characters.

### Patch Changes

- 90af674: Fixes an issue where a `<script define:vars={...}>` element lost all of its other attributes (such as `data-astro-rerun`, `id`, or `nonce`) in the compiled output.
- Updated dependencies [90af674]
- Updated dependencies [bc10d43]
- Updated dependencies [bc10d43]
  - @astrojs/compiler-binding@0.4.0

## 0.3.2

### Patch Changes

- 2264c9a: Fixes an issue where `define:vars` didn't work correctly for custom components.
- 4006641: Fixes two `:global()` scoping mismatches with the Go compiler. An attribute or class suffixed onto a leading `:global()` (`:global(a)[role="button"]`, `:global(a).cls`) is no longer mis-scoped and now stays global. A selector list inside `:global()` (`.x :global(ul, ol)`) no longer drops later items; the scope prefix is distributed across every item instead.
- Updated dependencies [2264c9a]
- Updated dependencies [4006641]
  - @astrojs/compiler-binding@0.3.2

## 0.3.1

### Patch Changes

- 1cbdc51: Fixes `client:only` causing a component's import to be stripped when the same binding is still used elsewhere, such as another plain instance, a `<Scope.Component>` sharing the import, or a reference in the frontmatter.
- f9cf9f2: Fixes empty script tags (i.e. `<script></script>`) causing the build to fail
- 584d1c0: Fixes a `${...}` inside a `set:text` or `transition:*` attribute value (e.g. `set:text="${x}"`) being treated as code instead of literal text, which could break the build or run unintended expressions. The value is now emitted verbatim.
- d9b63c4: Speeds up code generation by removing redundant string allocations in the printer hot path.
- 919fbb2: Improves code generation performance by reducing string and vector allocations.
- 911b54b: Fixes two conditional slot patterns that produced broken output:

  - A guarded conditional such as `{show && (cond ? <a slot="s">A</a> : <b slot="s">B</b>)}` now places its content in the right slot and keeps the original condition intact.
  - A conditional fragment such as `{cond && <>…</>}` now renders as the component's default content instead of emitting broken output.

- 19569d4: Fixes `<slot>` fallback content ignoring `compressHTML`, so whitespace around an expression (e.g. `<slot name="canonical">\n  {cond ? '' : <link />}\n</slot>`) is now collapsed like regular template children instead of being emitted verbatim.
- 911b54b: Fixes `Astro.slots.has()` reporting a slot as filled when its only `slot="..."` element sits inside a conditional that doesn't render it, such as `{cond ? <span slot="aside" /> : ...}` where every branch is currently false. Components that switch their layout based on `Astro.slots.has()` no longer render a spurious empty wrapper.
- 2085fd9: Fixes a direct element and an expression slot targeting the same named slot, such as `<div slot="x">A</div>{b && <div slot="x">B</div>}`, rendering only the last one. Both now render into the slot, matching same-named expression slots.
- 911b54b: Fixes multiple conditional elements targeting the same named slot, such as `{a && <div slot="x">A</div>}{b && <div slot="x">B</div>}`, rendering only the last one. All of them now render into the slot.
  - @astrojs/compiler-binding@0.3.1

## 0.3.0

### Minor Changes

- 7116c0c: The compiler now needs Node.js v22.12.0 a minimum version.
- 7116c0c: The compiler now ships only ESM modules.

### Patch Changes

- 6bde0ca: Fixes backslashes being dropped from attribute values, such as `<input pattern="^example\.com$" />` rendering as `^example.com$`. Backslashes are now kept in element attributes, component props, and `set:html`.
- 9abf779: Fix handling JSX in object literal
- 0ec1a02: Fix `:global()` being silently ignored when its selector starts with a combinator
- 6bde0ca: Fixes unquoted attribute values containing a `/` failing to compile, such as URLs like `<a href=https://example.com/path>` or root-relative paths like `<img src=/logo.png>`. Values containing `=`, quotes, or backticks are also no longer cut short.
- Updated dependencies [6bde0ca]
- Updated dependencies [6bde0ca]
  - @astrojs/compiler-binding@0.3.0

## 0.2.3

### Patch Changes

- 6a56dea: Fixes several cases of adjacent JSX elements inside a `{...}` expression acting weirdly, including:

  - HTML comments between the elements now render instead of being dropped, including when a comment is on its own line.
  - Whitespace between the elements is preserved.
  - A `<script>` that is not the first element no longer fails to parse when its body contains an HTML closing tag inside a template literal.

- c5b8921: Fixes compact mode not collapsing whitespace inside component and custom-element slot content.
- fd756ce: Fixes several cases where `await` failed to make the generated code `async`, producing invalid JavaScript.
- 7bd7e32: Fixes a parse error on `{{ ... }}` shorthand attributes containing an object expression, such as `<Debug {{ answer: sum(2, 4) }} />`. These now compile correctly instead of suggesting to use a spread attribute.
- 6a56dea: Fixes a `<style>` nested in a `{...}` expression (e.g. `{cond && <style>…</style>}`) being silently dropped from the output.
- 8b6d424: Fixes an "Invalid Character" error on unquoted attribute values that start with `#` followed by a digit, such as `<div color=#18b218 />`. Unquoted hex colors now parse correctly.
- Updated dependencies [6a56dea]
- Updated dependencies [c5b8921]
- Updated dependencies [fd756ce]
- Updated dependencies [7bd7e32]
- Updated dependencies [6a56dea]
- Updated dependencies [8b6d424]
  - @astrojs/compiler-binding@0.2.3

## 0.2.2

### Patch Changes

- 6133672: Improved diagnostics when Astro sees a stray closing tag. Now Astro correctly shows the closing tag that doesn't match an open tag.
- 23038db: Fixes `compact: 'jsx'` stripping significant same-line whitespace. JSX whitespace is now only trimmed where it borders a line break (matching React/Babel's rules), so a space at a text/expression, text/element, or element/element boundary is preserved. `<h1>Page {n}</h1>` now keeps its space (`Page 1`), as does `<span>hello</span> <em>world</em>`. Whitespace adjacent to newlines is still collapsed.
- Updated dependencies [6133672]
- Updated dependencies [23038db]
  - @astrojs/compiler-binding@0.2.2

## 0.2.1

### Patch Changes

- eddd417: Fixes CSS scoping incorrectly adding a scope to `&::pseudo-element` selectors (e.g. `&::marker`, `&::before`) and to selectors that only reference `&` inside `:is()`/`:where()`/`:not()`/`:has()`.
- f4adcce: Fixes `Unterminated string literal` error when a quoted attribute on a component contains literal newlines (e.g. multi-line `class`).
- ecb43d3: Fixes JSX not being transformed inside function declarations, class declarations and expressions, `throw` statements, and `for`-loop initializers.
- Updated dependencies [eddd417]
- Updated dependencies [f4adcce]
- Updated dependencies [ecb43d3]
  - @astrojs/compiler-binding@0.2.1

## 0.2.0

### Minor Changes

- 0726e00: Emit `templateEnter` / `templateExit` instructions when printing `<template>` elements for https://github.com/withastro/astro/pull/15980

### Patch Changes

- 0bddba4: Fixes `Unexpected token` error when an HTML attribute has an unquoted value such as a number (`<input maxlength=255>`) or contains characters like `-` or `#`.
- Updated dependencies [0bddba4]
- Updated dependencies [0726e00]
  - @astrojs/compiler-binding@0.2.0

## 0.1.10

### Patch Changes

- 551f3e0: Fixed invalid CSS output when using `::part()` or `::slotted()` pseudo-elements in scoped styles
  - @astrojs/compiler-binding@0.1.10

## 0.1.9

### Patch Changes

- 8b7a46f: Add fallback download for Webcontainers
- Updated dependencies [8b7a46f]
  - @astrojs/compiler-binding@0.1.9

## 0.1.8

### Patch Changes

- 6945d30: Fixed linux-gnu binaries requiring glibc 2.35+, which broke on Vercel, Amazon Linux 2023, and other environments with older glibc. Binaries now target glibc 2.17.
  - @astrojs/compiler-binding@0.1.8

## 0.1.7

### Patch Changes

- 4c9a9ed: Fixes edge cases where certain niche types of expressions wouldn't properly compile
- 1b17201: Fixes slots not working inside parenthesized conditional slots
- cace524: Fixes the compiler sometimes adding extra whitespace between root elements when one of the root elements would be hoisted (e.g. style tags, scripts etc.)
- 4ebb68d: Fixes slots not being collected inside optional chain expressions
- dc9cbe4: Fixes scripts inside template elements not being rendered as-is
- Updated dependencies [4c9a9ed]
- Updated dependencies [1b17201]
- Updated dependencies [cace524]
- Updated dependencies [4ebb68d]
- Updated dependencies [dc9cbe4]
  - @astrojs/compiler-binding@0.1.7

## 0.1.6

### Patch Changes

- 6e274fe: Fixes dynamic slots not being collected properly
- Updated dependencies [6e274fe]
  - @astrojs/compiler-binding@0.1.6

## 0.1.5

### Patch Changes

- ddf38ff: Fixes dynamic slots not working correctly
- 21b6cd5: Fixes CSS scoping not working correctly when using :global with pseudo elements
- e93a108: Fixes the compiler scoping nested selectors in certain cases
- c8f6dc5: Fixed an issue where define:vars scripts would not be handled correctly
- c8f6dc5: Fixes an issue where set:html did not work correctly in certain cases
- Updated dependencies [ddf38ff]
- Updated dependencies [21b6cd5]
- Updated dependencies [e93a108]
- Updated dependencies [c8f6dc5]
- Updated dependencies [c8f6dc5]
  - @astrojs/compiler-binding@0.1.5

## 0.1.4

### Patch Changes

- 30299ab: Fixes an issue where certain compressHTML settings wouldn't work
- Updated dependencies [30299ab]
  - @astrojs/compiler-binding@0.1.4

## 0.1.3

### Patch Changes

- c49b415: Trim body whitespace like the Go compiler does
- Updated dependencies [c49b415]
  - @astrojs/compiler-binding@0.1.3

## 0.1.2

### Patch Changes

- efed4ed: Fixes further issues found in the Astro tests, especially around HTML escaping in set:html
  - @astrojs/compiler-binding@0.1.2

## 0.1.1

### Patch Changes

- ae6e49c: Fixes various issues encountered in Astro tests
- Updated dependencies [ae6e49c]
  - @astrojs/compiler-binding@0.1.1

## 0.1.0

### Minor Changes

- bc95791: Initial release

### Patch Changes

- Updated dependencies [bc95791]
  - @astrojs/compiler-binding@0.1.0
