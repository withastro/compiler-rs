import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { typecheckAstro } from './typecheck.js';

test('TS-to-TSX rewrites preserve assertion precedence and generic inference', () => {
	assert.deepEqual(
		typecheckAstro(`
		const id = <T>(x: T) => x;
		const value = (<{ name: string }>{ name: 'ok' }).name;
		const text: string = id(value);
		const nested: number = <number><unknown>id(42);
	`),
		[],
	);
});
