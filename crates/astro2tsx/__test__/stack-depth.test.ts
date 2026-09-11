import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { convertToTsx } from '../index.js';

test('release WASM handles long expression trees and nested markup', () => {
	for (const source of [
		`{${'a+'.repeat(32_000)}a}`,
		`{a${'.a'.repeat(16_000)}}`,
		`${'<div>'.repeat(128)}text${'</div>'.repeat(128)}`,
	]) {
		const result = convertToTsx(source);
		assert.equal(result.hasParseErrors, false);
		assert.ok(result.code.includes(source));
	}
	const frontmatter = `const value = ${'a+'.repeat(16_000)}a;`;
	const result = convertToTsx(`---\n${frontmatter}\n---`);
	assert.equal(result.hasParseErrors, false);
	assert.ok(result.code.includes(frontmatter));
});
