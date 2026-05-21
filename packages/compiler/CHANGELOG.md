# @astrojs/compiler-rs

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
