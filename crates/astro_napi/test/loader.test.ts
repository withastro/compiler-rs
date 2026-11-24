import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';

const indexUrl = new URL('../index.js', import.meta.url).href;
const nativeStubPath = new URL('./stubs/native-stub.cjs', import.meta.url).pathname;

function runLoaderScript(script: string, env: Record<string, string>) {
	return new Promise<{ code: number | null; stdout: string; stderr: string }>((resolve) => {
		const child = spawn(process.execPath, ['--input-type=module', '-e', script], {
			env: { ...process.env, NAPI_RS_NATIVE_LIBRARY_PATH: nativeStubPath, ...env },
			cwd: new URL('..', import.meta.url).pathname,
		});
		let stdout = '';
		let stderr = '';
		child.stdout.setEncoding('utf8');
		child.stderr.setEncoding('utf8');
		child.stdout.on('data', (chunk) => {
			stdout += chunk;
		});
		child.stderr.on('data', (chunk) => {
			stderr += chunk;
		});
		child.on('close', (code) => resolve({ code, stdout, stderr }));
	});
}

const loadScript = `import(${JSON.stringify(indexUrl)}).then(m => {
	console.log(m.compileAstroSync?.() ?? 'no-compileAstroSync');
}).catch(e => {
	console.error(e.message);
	process.exit(1);
});`;

describe('NAPI_RS_FORCE_WASI loader behavior', () => {
	it("'true' skips the native addon and falls back to WASI", async () => {
		const { code, stdout, stderr } = await runLoaderScript(loadScript, {
			NAPI_RS_FORCE_WASI: 'true',
		});
		assert.notStrictEqual(code, 0, 'expected WASI fallback to fail without a stub');
		assert.ok(!stdout.includes('native-stub-loaded'), 'native stub should not be loaded');
		assert.ok(stderr.includes('Cannot find native binding'), stderr);
	});

	it("'false' does not force WASI and loads the native addon", async () => {
		const { code, stdout } = await runLoaderScript(loadScript, {
			NAPI_RS_FORCE_WASI: 'false',
		});
		assert.strictEqual(code, 0);
		assert.ok(stdout.includes('native-stub-loaded'), stdout);
	});

	it("'0' does not force WASI and loads the native addon", async () => {
		const { code, stdout } = await runLoaderScript(loadScript, {
			NAPI_RS_FORCE_WASI: '0',
		});
		assert.strictEqual(code, 0);
		assert.ok(stdout.includes('native-stub-loaded'), stdout);
	});

	it("'error' skips the native addon and throws when WASI is missing", async () => {
		const { code, stderr } = await runLoaderScript(loadScript, {
			NAPI_RS_FORCE_WASI: 'error',
		});
		assert.notStrictEqual(code, 0);
		assert.ok(stderr.includes('WASI binding not found'), stderr);
	});
});
