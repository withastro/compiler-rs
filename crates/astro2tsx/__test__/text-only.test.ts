import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { typecheckAstro } from './typecheck.ts';

test('expressions inside literal tags in title and textarea retain type checking', () => {
	for (const tag of ['title', 'textarea']) {
		const markup = `<${tag}><Widget><b>{missingName}</b></Widget></${tag}>`;
		for (const body of [markup, `{${markup}}`]) {
			const diagnostics = typecheckAstro('', body);
			assert.deepEqual(
				diagnostics.map(({ code, text }) => ({ code, text })),
				[{ code: 2304, text: 'missingName' }],
			);
			assert.deepEqual(typecheckAstro('const missingName = "ok";', body), []);
		}
	}
});
