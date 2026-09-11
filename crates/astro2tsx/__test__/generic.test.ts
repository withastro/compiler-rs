import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { convertToTsx } from '../index.js';
import { typecheckAstro, typecheck } from './typecheck.js';

test('generic Props retain useful global prop checking', () => {
	for (const declaration of [
		'interface Props<T> { items: T[] }',
		'interface Props<out T> { items: T[] }',
		'interface Props<T extends string> { items: T[] }',
		'interface Props<T = string> { items: T[] }',
		'interface Props<T, U extends T = T> { items: U[] }',
	]) {
		const diagnostics = typecheckAstro(`${declaration}\nconst bad: number = Astro.props.items;`);
		assert.deepEqual(
			diagnostics.map((d) => [d.code, d.text]),
			[[2322, 'bad']],
			JSON.stringify(diagnostics),
		);
	}
});

test('generic component signatures retain inference and constraints', () => {
	const source = '---\ninterface Props<out T extends string> { value: T }\n---';
	const { code } = convertToTsx(source, { ambientTypes: true });
	assert.deepEqual(typecheck(`${code}\nAstroComponent({ value: 'ok' });`), []);
	const diagnostics = typecheck(`${code}\nAstroComponent({ value: 42 });`);
	assert.deepEqual(
		diagnostics.map((d) => d.code),
		[2322],
		JSON.stringify(diagnostics),
	);
});
