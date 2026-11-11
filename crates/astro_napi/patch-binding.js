import { readFileSync, writeFileSync } from 'node:fs';

const bindingPath = new URL('./index.js', import.meta.url);
let content = readFileSync(bindingPath, 'utf-8');

// Don't try to load the native addon when the user forces WASI. On a CPU that
// doesn't support the compiler's baseline, require() of the .node can raise
// SIGILL before the WASI fallback has a chance to run.
content = content.replace(
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
);

// NAPI_RS_FORCE_WASI is a tri-state flag:
//   unset / any other value → native binding preferred, WASI is only a fallback
//   'true'                   → force WASI fallback even if native loaded
//   'error'                  → force WASI and throw if no WASI binding is found
content = content.replace(
  'if (!nativeBinding || process.env.NAPI_RS_FORCE_WASI) {\n  let wasiBinding = null',
  `// NAPI_RS_FORCE_WASI is a tri-state flag:
//   unset / any other value → native binding preferred, WASI is only a fallback
//   'true'                   → force WASI fallback even if native loaded
//   'error'                  → force WASI and throw if no WASI binding is found
// Treating any non-empty string as truthy (the historical behavior) meant
// NAPI_RS_FORCE_WASI=false, NAPI_RS_FORCE_WASI=0, etc. inadvertently triggered
// the WASI path, causing ENOENT for packages shipped without a .wasi.cjs file.
const forceWasi =
  process.env.NAPI_RS_FORCE_WASI === 'true' || process.env.NAPI_RS_FORCE_WASI === 'error'

if (!nativeBinding || forceWasi) {\n  let wasiBinding = null`,
);

content = content.replace(
  'if (!nativeBinding || process.env.NAPI_RS_FORCE_WASI) {',
  'if (!nativeBinding || forceWasi) {',
);

// Forward WASI load failures only when the flag explicitly requests WASI.
content = content.replaceAll(
  'if (process.env.NAPI_RS_FORCE_WASI) {',
  'if (forceWasi) {',
);

// StackBlitz / WebContainer does not ship the native addon, so load a separate
// browser-compatible bundle before giving up.
content = content.replace(
  `if (process.env.NAPI_RS_FORCE_WASI === 'error' && !wasiBinding) {\n    const error = new Error('WASI binding not found and NAPI_RS_FORCE_WASI is set to error')\n    error.cause = wasiBindingError\n    throw error\n  }\n}\n\nif (!nativeBinding) {\n  if (loadErrors.length > 0) {`,
  `if (process.env.NAPI_RS_FORCE_WASI === 'error' && !wasiBinding) {\n    const error = new Error('WASI binding not found and NAPI_RS_FORCE_WASI is set to error')\n    error.cause = wasiBindingError\n    throw error\n  }\n}\n\nif (!nativeBinding && globalThis.process?.versions?.['webcontainer']) {\n  try {\n    nativeBinding = require('./webcontainer-fallback.cjs');\n  } catch (err) {\n    loadErrors.push(err)\n  }\n}\n\nif (!nativeBinding) {\n  if (loadErrors.length > 0) {`,
);

writeFileSync(bindingPath, content);
