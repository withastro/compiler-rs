import { readFileSync, writeFileSync } from 'node:fs';

const bindingPath = new URL('./index.js', import.meta.url);
let content = readFileSync(bindingPath, 'utf-8');

content = content.replace(
  'nativeBinding = requireNative()',
  `// Don't initialize the native addon when WASI is forced. Loading the native
// .node on an unsupported CPU triggers SIGILL before the WASI fallback can run.
// See https://github.com/withastro/compiler/issues/220
if (
  process.env.NAPI_RS_FORCE_WASI !== 'true' &&
  process.env.NAPI_RS_FORCE_WASI !== 'error'
) {
  nativeBinding = requireNative()
}`,
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
