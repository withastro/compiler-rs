import { strict as assert } from 'node:assert';
import { join } from 'node:path';
import { test } from 'node:test';
import ts from 'typescript';
import { convertToTsx } from '../index.js';
import { typecheck } from './typecheck.ts';

test('default and clean-name exports preserve generic props without shadowing local names', () => {
	const { code } = convertToTsx(
		'---\ndeclare const MyPage: 42;\ninterface Props<T extends string> { value: T }\n---',
		{ filename: 'MyPage.astro', ambientTypes: true },
	);
	const imports = `import DefaultComponent, { MyPage as NamedComponent } from './consumer.js';`;
	assert.deepEqual(
		typecheck(`${code}\n${imports}
			const local: 42 = MyPage;
			const same: typeof DefaultComponent = NamedComponent;
			DefaultComponent({ value: 'ok' });
			NamedComponent({ value: 'ok' });
		`),
		[],
	);
	const diagnostics = typecheck(`${code}\n${imports}
		DefaultComponent({ value: 42 });
		NamedComponent({ value: 42 });
	`);
	assert.deepEqual(
		diagnostics.map((diagnostic) => diagnostic.code),
		[2322, 2322],
	);
});

test('explicit exports keep their types when their name matches the component filename', () => {
	for (const declaration of [
		'export const MyPage = 42;',
		'const value = 42; export { value as MyPage };',
		'export const { value: MyPage } = { value: 42 as const };',
		'export declare const MyPage: 42;',
	]) {
		const { code } = convertToTsx(`---\n${declaration}\n---`, { filename: 'MyPage.astro' });
		assert.deepEqual(
			typecheck(`${code}
			import DefaultComponent, { MyPage as NamedValue } from './consumer.js';
			const valueFromExport: 42 = NamedValue;
			DefaultComponent({});
		`),
			[],
		);
	}
});

test('TypeScript offers the clean component export for JSX auto-imports', () => {
	const component = join(import.meta.dirname, 'MyPage.tsx');
	const consumer = join(import.meta.dirname, 'consumer.tsx');
	const source = 'export {};\nconst view = <MyP';
	const files = new Map([
		[component, convertToTsx('', { filename: 'MyPage.astro' }).code],
		[consumer, source],
	]);
	const service = ts.createLanguageService({
		getCompilationSettings: () => ({
			module: ts.ModuleKind.ESNext,
			moduleResolution: ts.ModuleResolutionKind.Bundler,
			jsx: ts.JsxEmit.Preserve,
			types: [],
		}),
		getScriptFileNames: () => [...files.keys()],
		getScriptVersion: () => '0',
		getScriptSnapshot: (name) => {
			const text = files.get(name) ?? ts.sys.readFile(name);
			return text === undefined ? undefined : ts.ScriptSnapshot.fromString(text);
		},
		getCurrentDirectory: () => import.meta.dirname,
		getDefaultLibFileName: ts.getDefaultLibFilePath,
		fileExists: (name) => files.has(name) || ts.sys.fileExists(name),
		readFile: (name) => files.get(name) ?? ts.sys.readFile(name),
		readDirectory: ts.sys.readDirectory,
	});
	try {
		const completions = service.getCompletionsAtPosition(consumer, source.length, {
			includeCompletionsForModuleExports: true,
			includeCompletionsWithInsertText: true,
		});
		assert.ok(completions?.entries.some((entry) => entry.name === 'MyPage' && entry.source));
	} finally {
		service.dispose();
	}
});
