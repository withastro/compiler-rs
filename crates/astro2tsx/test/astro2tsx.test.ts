import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { strict as assert } from 'node:assert';
import ts from 'typescript';
import { test } from 'node:test';
import { convertToTsx } from '../index.js';

test('emits the TSX prefix and a Fragment-wrapped body', () => {
	const result = convertToTsx('<h1>Hello {value}</h1>');
	assert.ok(result.code.startsWith('/* @jsxImportSource astro */'));
	assert.match(result.code, /<Fragment>[\s\S]*<h1>[\s\S]*<\/h1>[\s\S]*<\/Fragment>/);
});

test('rewrites top-level returns to throws', () => {
	const result = convertToTsx("---\nif (cond) {\n\treturn Astro.redirect('/x');\n}\n---\n");
	assert.match(result.code, /throw\s+Astro\.redirect/);
	assert.doesNotMatch(result.code, /return Astro\.redirect/);
});

test('detects `Props` interface and emits the Astro global declaration', () => {
	const input = '---\ninterface Props {}\n---\n<div></div>';
	const result = convertToTsx(input, { filename: 'Index.astro' });
	assert.match(result.code, /_props: Props/);
	assert.match(
		result.code,
		/declare const Astro: Readonly<import\('astro'\)\.AstroGlobal<Props,\s+typeof Index__AstroComponent_>>/,
	);
});

test('reports parse errors but still produces output', () => {
	const result = convertToTsx('<div'); // unclosed tag
	assert.equal(result.hasParseErrors, true);
	assert.ok(result.code.length > 0);
});

test('records frontmatter and body byte ranges', () => {
	const result = convertToTsx('---\nlet x = 1;\n---\n<p></p>');
	assert.ok(result.frontmatter.end > result.frontmatter.start);
	assert.ok(result.body.end > result.body.start);
	const frontmatterSlice = result.code.slice(result.frontmatter.start, result.frontmatter.end);
	assert.match(frontmatterSlice, /let x = 1;/);
});

test('mapped runs interpolate to exact source positions, Volar-style', () => {
	const source =
		'---\nconst x = 1;\n---\n<article id="main" class=plain data-thing={x}>hello <b>world</b></article>';
	const result = convertToTsx(source, { sourcemap: 'external' });
	const { generatedOffsets, sourceOffsets, lengths } = result;

	assert.ok(generatedOffsets instanceof Uint32Array);
	assert.equal(generatedOffsets.length, sourceOffsets.length);
	assert.equal(generatedOffsets.length, lengths.length);
	assert.ok(generatedOffsets.length > 0);
	for (let i = 0; i < generatedOffsets.length; i++) {
		assert.ok(lengths[i] > 0, `run ${i} is empty`);
		if (i > 0) {
			assert.ok(
				generatedOffsets[i] >= generatedOffsets[i - 1] + lengths[i - 1],
				`run ${i} overlaps its predecessor`,
			);
		}
	}

	const originalAt = (generated) => {
		for (let i = generatedOffsets.length - 1; i >= 0; i--) {
			const delta = generated - generatedOffsets[i];
			if (delta >= 0 && delta < lengths[i]) return sourceOffsets[i] + delta;
		}
		return null;
	};

	// Hover-style probes, deliberately mid-run rather than at run starts.
	for (const probe of ['id="main"', 'data-thing', 'hello', 'world', 'const x = 1;']) {
		const generated = result.code.indexOf(probe);
		assert.notEqual(generated, -1, `${probe} not in output`);
		const original = originalAt(generated);
		assert.notEqual(original, null, `${probe} is unmapped`);
		assert.equal(
			source.slice(original, original + probe.length),
			probe,
			`${probe} resolves to the wrong source text`,
		);
	}

	// Synthetic output stays unmapped instead of resolving somewhere misleading.
	assert.equal(originalAt(result.code.indexOf('<Fragment>')), null);
	assert.equal(originalAt(result.code.indexOf('export default function')), null);
});

test('returns a self-contained source map v3', () => {
	const input = '---\nlet x = 1;\n---\n<p>Hi</p>';
	const map = JSON.parse(convertToTsx(input).map);
	assert.equal(map.version, 3);
	assert.deepEqual(map.sources, ['input.astro']);
	assert.deepEqual(map.sourcesContent, [input]);
	assert.deepEqual(map.names, []);
	assert.ok(map.mappings.length > 0);
});

test('names the source after the filename option', () => {
	const map = JSON.parse(convertToTsx('<p></p>', { filename: 'Index.astro' }).map);
	assert.deepEqual(map.sources, ['Index.astro']);
});

test('appends the inline source map comment by default', () => {
	const { code, map } = convertToTsx('<p>Hi</p>');
	const marker = '\n//# sourceMappingURL=data:application/json;charset=utf-8;base64,';
	assert.ok(code.includes(marker));
	const blob = code.slice(code.indexOf(marker) + marker.length);
	assert.deepEqual(JSON.parse(Buffer.from(blob, 'base64').toString('utf8')), JSON.parse(map));
});

test("sourcemap: 'external' leaves the code without the comment", () => {
	const inline = convertToTsx('<p>Hi</p>');
	const external = convertToTsx('<p>Hi</p>', { sourcemap: 'external' });
	assert.doesNotMatch(external.code, /sourceMappingURL/);
	assert.equal(external.map, inline.map);
	assert.ok(inline.code.startsWith(external.code));
});

test('every clean-parse fixture emits syntactically valid TSX', async () => {
	// Frontmatter is user JS emitted verbatim, so a fixture whose frontmatter is
	// itself invalid TS legitimately produces invalid output.
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
