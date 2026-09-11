import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { typecheckAstro } from './typecheck.ts';

test('comment-only attributes type-check as undefined', () => {
	const declarations = `declare function C(props: { foo?: string; '@foo'?: string }): any;`;
	for (const value of ['/* comment */', '// comment\n', '/* first */ /* second */']) {
		assert.deepEqual(typecheckAstro(declarations, `<C foo={${value}} @foo={${value}} />`), []);
	}
	const diagnostics = typecheckAstro(
		`declare function C(props: { foo: string }): any;`,
		'<C foo={/* comment */} />',
	);
	assert.deepEqual(
		diagnostics.map((diagnostic) => diagnostic.code),
		[2322],
	);
});

test('transformed attributes preserve spread precedence for type checking', () => {
	const declarations = `
		declare function C(props: { '@foo': 42 }): any;
		const attrs = { '@foo': 42 as const };
	`;
	for (const wrap of [false, true]) {
		const body = '<C @foo="bad" {...attrs} />';
		assert.deepEqual(typecheckAstro(declarations, wrap ? `{${body}}` : body), []);
		const invalid = '<C {...attrs} @foo="bad" />';
		const diagnostics = typecheckAstro(declarations, wrap ? `{${invalid}}` : invalid);
		assert.deepEqual(
			diagnostics.map((d) => d.code),
			[2322],
			JSON.stringify(diagnostics),
		);
	}
});
