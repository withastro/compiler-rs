import { readFileSync, writeFileSync } from 'node:fs';

const bindingPath = new URL('./index.js', import.meta.url);
let content = readFileSync(bindingPath, 'utf-8');

function countOccurrences(str, search) {
  let count = 0;
  let index = str.indexOf(search);
  while (index !== -1) {
    count += 1;
    index = str.indexOf(search, index + search.length);
  }
  return count;
}

function applyPatch(content, search, replacement, { name, expected }) {
  const actual = countOccurrences(content, search);
  if (actual !== expected) {
    throw new Error(
      `patch-binding.js: expected ${expected} occurrence(s) of ${name}, found ${actual}. ` +
        'The generated index.js may have changed; update patch-binding.js.',
    );
  }
  return content.replace(search, replacement);
}

// Don't try to load the native addon when the user forces WASI. On a CPU that
// doesn't support the compiler's baseline, require() of the .node can raise
// SIGILL before the WASI fallback has a chance to run.
content = applyPatch(
  content,
  '\nnativeBinding = requireNative()\n',
  `\n// Don't initialize the native addon when WASI is forced. Loading the native
// .node on an unsupported CPU triggers SIGILL before the WASI fallback can run.
// See https://github.com/withastro/compiler/issues/220
if (
  process.env.NAPI_RS_FORCE_WASI !== 'true' &&
  process.env.NAPI_RS_FORCE_WASI !== 'error'
) {
  nativeBinding = requireNative()
}\n`,
  { name: 'native binding assignment', expected: 1 },
);

// @napi-rs/cli@3.7.4 already emits the forceWasi tri-state, but its comment
// still describes the historical any-truthy behavior (load native first, then
// force fallback). Update it to reflect that the loader now skips native
// initialization entirely when the flag is 'true' or 'error'.
content = applyPatch(
  content,
  `//   'true'                   → force WASI fallback even if native loaded\n//   'error'                  → force WASI and throw if no WASI binding is found`,
  `//   'true'                   → skip the native addon and force the WASI fallback\n//   'error'                  → skip the native addon and throw if no WASI binding is found`,
  { name: 'tri-state comment', expected: 1 },
);

// StackBlitz / WebContainer does not ship the native addon, so load a separate
// browser-compatible bundle before giving up.
content = applyPatch(
  content,
  `if (process.env.NAPI_RS_FORCE_WASI === 'error' && !wasiBinding) {\n    const error = new Error('WASI binding not found and NAPI_RS_FORCE_WASI is set to error')\n    error.cause = wasiBindingError\n    throw error\n  }\n}\n\nif (!nativeBinding) {\n  if (loadErrors.length > 0) {`,
  `if (process.env.NAPI_RS_FORCE_WASI === 'error' && !wasiBinding) {\n    const error = new Error('WASI binding not found and NAPI_RS_FORCE_WASI is set to error')\n    error.cause = wasiBindingError\n    throw error\n  }\n}\n\nif (!nativeBinding && globalThis.process?.versions?.['webcontainer']) {\n  try {\n    nativeBinding = require('./webcontainer-fallback.cjs');\n  } catch (err) {\n    loadErrors.push(err)\n  }\n}\n\nif (!nativeBinding) {\n  if (loadErrors.length > 0) {`,
  { name: 'webcontainer fallback anchor', expected: 1 },
);

writeFileSync(bindingPath, content);
