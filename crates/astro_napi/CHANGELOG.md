# @astrojs/compiler-binding

## 0.2.1

### Patch Changes

- eddd417: Fixes CSS scoping incorrectly adding a scope to `&::pseudo-element` selectors (e.g. `&::marker`, `&::before`) and to selectors that only reference `&` inside `:is()`/`:where()`/`:not()`/`:has()`.
- f4adcce: Fixes `Unterminated string literal` error when a quoted attribute on a component contains literal newlines (e.g. multi-line `class`).
- ecb43d3: Fixes JSX not being transformed inside function declarations, class declarations and expressions, `throw` statements, and `for`-loop initializers.

## 0.2.0

### Minor Changes

- 0726e00: Emit `templateEnter` / `templateExit` instructions when printing `<template>` elements for https://github.com/withastro/astro/pull/15980

### Patch Changes

- 0bddba4: Fixes `Unexpected token` error when an HTML attribute has an unquoted value such as a number (`<input maxlength=255>`) or contains characters like `-` or `#`.

## 0.1.10

## 0.1.9

### Patch Changes

- 8b7a46f: Add fallback download for Webcontainers

## 0.1.8

## 0.1.7

### Patch Changes

- 4c9a9ed: Fixes edge cases where certain niche types of expressions wouldn't properly compile
- 1b17201: Fixes slots not working inside parenthesized conditional slots
- cace524: Fixes the compiler sometimes adding extra whitespace between root elements when one of the root elements would be hoisted (e.g. style tags, scripts etc.)
- 4ebb68d: Fixes slots not being collected inside optional chain expressions
- dc9cbe4: Fixes scripts inside template elements not being rendered as-is

## 0.1.6

### Patch Changes

- 6e274fe: Fixes dynamic slots not being collected properly

## 0.1.5

### Patch Changes

- ddf38ff: Fixes dynamic slots not working correctly
- 21b6cd5: Fixes CSS scoping not working correctly when using :global with pseudo elements
- e93a108: Fixes the compiler scoping nested selectors in certain cases
- c8f6dc5: Fixed an issue where define:vars scripts would not be handled correctly
- c8f6dc5: Fixes an issue where set:html did not work correctly in certain cases

## 0.1.4

### Patch Changes

- 30299ab: Fixes an issue where certain compressHTML settings wouldn't work

## 0.1.3

### Patch Changes

- c49b415: Trim body whitespace like the Go compiler does

## 0.1.2

## 0.1.1

### Patch Changes

- ae6e49c: Fixes various issues encountered in Astro tests

## 0.1.0

### Minor Changes

- bc95791: Initial release
