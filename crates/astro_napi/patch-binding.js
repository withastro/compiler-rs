import { readFileSync, writeFileSync } from 'node:fs';

const bindingPath = new URL('./index.js', import.meta.url);
let content = readFileSync(bindingPath, 'utf-8');

// Honor NAPI_RS_FORCE_WASI by skipping the native binding attempt entirely.
// The generated NAPI-RS loader calls requireNative() before checking the env
// var, so unsigned native modules can be blocked by Windows Smart App Control
// before the WASI fallback is reached.
content = content.replace(
  '\nnativeBinding = requireNative()',
  (s) =>
    '\nif (!process.env.NAPI_RS_FORCE_WASI) {' + s.replace('\n', '\n  ') + '\n}',
);

content = content.replace(
  '\nif (!nativeBinding) {',
  (s) =>
    `
if (!nativeBinding && globalThis.process?.versions?.['webcontainer']) {
  try {
    nativeBinding = require('./webcontainer-fallback.cjs');
  } catch (err) {
    loadErrors.push(err)
  }
}
` + s,
);

writeFileSync(bindingPath, content);
