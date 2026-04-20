# @astrojs/compiler-rs

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
