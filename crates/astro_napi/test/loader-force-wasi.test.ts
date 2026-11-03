import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';

const bindingPath = fileURLToPath(new URL('../index.js', import.meta.url));

describe('NAPI loader', () => {
	it('does not attempt to load a native .node binding when NAPI_RS_FORCE_WASI is set', async () => {
		const script = `
		import { createRequire } from 'node:module';
		const require = createRequire(import.meta.url);
		const Module = require('node:module');
		const originalLoad = Module._load;
		const attempts = [];
		Module._load = function (request, parent, isMain) {
			if (typeof request === 'string' && request.endsWith('.node')) {
				attempts.push(request);
			}
			return originalLoad.apply(this, arguments);
		};
		try {
			await import(${JSON.stringify(bindingPath)});
		} catch {
			// The binding may not load in every test environment; we only care
			// that a native module was not required.
		}
		console.log(JSON.stringify(attempts));
		`;

		const result = await new Promise<string>((resolve, reject) => {
			const child = spawn(process.execPath, ['--input-type=module', '--eval', script], {
				env: { ...process.env, NAPI_RS_FORCE_WASI: '1' },
				stdio: ['inherit', 'pipe', 'pipe'],
			});
			let stdout = '';
			let stderr = '';
			child.stdout.on('data', (data) => {
				stdout += data;
			});
			child.stderr.on('data', (data) => {
				stderr += data;
			});
			child.on('close', (code) => {
				if (code !== 0) {
					reject(new Error(`child exited ${code}: ${stderr}`));
				} else {
					resolve(stdout);
				}
			});
		});

		const attempts = JSON.parse(result.trim()) as string[];
		assert.deepStrictEqual(
			attempts,
			[],
			'NAPI_RS_FORCE_WASI should prevent any attempt to load a native .node binding',
		);
	});
});
