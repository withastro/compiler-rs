import { strict as assert } from 'node:assert';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import ts from 'typescript';
import { convertToTsx } from '../index.js';

test('every clean-parse fixture emits syntactically valid TSX', async () => {
	const invalidUserCode = new Set(['props_generic_invalid']);

	const dir = join(import.meta.dirname, '../tests/fixtures');
	let checked = 0;
	for (const file of readdirSync(dir)) {
		if (!file.endsWith('.astro')) continue;
		const name = file.slice(0, -'.astro'.length);
		if (invalidUserCode.has(name)) continue;

		let source = readFileSync(join(dir, file), 'utf8');
		while (source.startsWith('// @config ')) {
			source = source.slice(source.indexOf('\n') + 1);
		}

		const result = convertToTsx(source, { filename: `${name}.astro` });
		if (result.hasParseErrors) continue;

		const sourceFile = ts.createSourceFile(
			`${name}.tsx`,
			result.code,
			ts.ScriptTarget.Latest,
			false,
			ts.ScriptKind.TSX,
		);
		const diagnostics = (
			sourceFile as unknown as { parseDiagnostics: { messageText: unknown; start: number }[] }
		).parseDiagnostics;
		assert.deepEqual(
			diagnostics.map((d) => `${name}: ${JSON.stringify(d.messageText)} at ${d.start}`),
			[],
			`invalid TSX emitted for ${name}:\n${result.code}`,
		);
		checked++;
	}
	assert.ok(checked > 50, `expected to check most fixtures, checked ${checked}`);
});
