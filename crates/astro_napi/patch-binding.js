import { readFileSync, writeFileSync } from 'node:fs';

const bindingPath = new URL('./index.js', import.meta.url);
let content = readFileSync(bindingPath, 'utf-8');

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
