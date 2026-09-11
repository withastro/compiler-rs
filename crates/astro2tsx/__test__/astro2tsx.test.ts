import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { strict as assert } from 'node:assert';
import ts from 'typescript';
import { test } from 'node:test';
import {
	AstroFrontmatterStatus,
	convertToTsx,
	DiagnosticSeverity,
	ExtractedScriptType,
	ExtractedStyleType,
} from '../index.js';

test('emits the TSX prefix and a Fragment-wrapped body', () => {
	const result = convertToTsx('<h1>Hello {value}</h1>');
	assert.ok(result.code.startsWith('/* @jsxImportSource astro */'));
	assert.match(result.code, /<Fragment>[\s\S]*<h1>[\s\S]*<\/h1>[\s\S]*<\/Fragment>/);
});

test('exports enums and converts extracted tag metadata', () => {
	const result = convertToTsx('<script>const x = 1;</script><style>.x { color: red }</style>');

	assert.equal(AstroFrontmatterStatus.Closed, 'closed');
	assert.equal(DiagnosticSeverity.Error, 1);
	assert.equal(ExtractedScriptType.ProcessedModule, 'processed-module');
	assert.equal(ExtractedStyleType.Tag, 'tag');
	const [script] = result.scripts;
	const [style] = result.styles;
	assert.ok(script);
	assert.ok(style);
	assert.equal(script.type, ExtractedScriptType.ProcessedModule);
	assert.equal(style.type, ExtractedStyleType.Tag);
});

test('reports parse errors but still produces output', () => {
	const result = convertToTsx('<div');
	assert.equal(result.hasParseErrors, true);
	assert.ok(result.code.length > 0);
});

test('reports invalid frontmatter without blocking output', () => {
	const source = '---\nconst = ;\n---\n<p/>';
	const result = convertToTsx(source);
	assert.equal(result.hasParseErrors, true);
	assert.ok(result.diagnostics.length > 0);
	assert.ok(result.code.includes('const = ;'));
	for (const diagnostic of result.diagnostics) {
		assert.ok(diagnostic.position.start >= 4);
		assert.ok(diagnostic.position.end <= 13);
	}
});

test('records generated frontmatter and body ranges', () => {
	const result = convertToTsx('---\nlet x = 1;\n---\n<p></p>');
	assert.ok(result.frontmatter.end > result.frontmatter.start);
	assert.ok(result.body.end > result.body.start);
	const frontmatterSlice = result.code.slice(result.frontmatter.start, result.frontmatter.end);
	assert.match(frontmatterSlice, /let x = 1;/);
});

test('returns TypeScript Content Mapper span mappings', () => {
	const source =
		'---\nconst x = 1;\n---\n<article id="main" class=plain data-thing={x}>hello <b>world</b></article>';
	const result = convertToTsx(source, { ambientTypes: true });

	assert.ok(result.mappings.length > 0);
	let previousEnd = 0;
	for (const [i, mapping] of result.mappings.entries()) {
		const [virtualStart, virtualLength, originalStart, originalLength, kind] = mapping;
		assert.ok(virtualLength > 0, `span ${i} is empty`);
		if (i > 0) {
			assert.ok(virtualStart >= previousEnd, `span ${i} overlaps its predecessor`);
		}
		previousEnd = virtualStart + virtualLength;
		if (kind === 0) {
			assert.equal(virtualLength, originalLength);
			assert.equal(
				result.code.slice(virtualStart, virtualStart + virtualLength),
				source.slice(originalStart, originalStart + originalLength),
				`span ${i} is not verbatim`,
			);
		}
	}

	const originalAt = (generated: number): number | null => {
		for (const mapping of [...result.mappings].reverse()) {
			const [virtualStart, virtualLength, originalStart, , kind] = mapping;
			const delta = generated - virtualStart;
			if (kind === 0 && delta >= 0 && delta < virtualLength) return originalStart + delta;
		}
		return null;
	};

	for (const probe of ['id="main"', 'data-thing', 'hello', 'world', 'const x = 1;']) {
		const generated = result.code.indexOf(probe);
		assert.notEqual(generated, -1, `${probe} not in output`);
		const original = originalAt(generated);
		if (original === null) {
			throw new Error(`${probe} is unmapped`);
		}
		assert.equal(
			source.slice(original, original + probe.length),
			probe,
			`${probe} resolves to the wrong source text`,
		);
	}

	assert.equal(originalAt(result.code.indexOf('<Fragment>')), null);
	assert.equal(originalAt(result.code.indexOf('export default function')), null);

	const exportName = result.code.indexOf('__AstroComponent_');
	assert.deepEqual(
		result.mappings.find((mapping) => mapping[0] === exportName),
		[exportName, '__AstroComponent_'.length, 0, 0, 1, (1 << 3) | (1 << 6)],
	);
});

test('every clean-parse fixture emits syntactically valid TSX', async () => {
	// Invalid user frontmatter legitimately produces invalid TSX.
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

test('offsets are UTF-16 code units, not bytes', () => {
	const source = '---\nconst \u{1f984} = 1;\n---\n<style>.a{color:red}</style>';
	const result = convertToTsx(source);

	const [style] = result.styles;
	assert.ok(style);
	assert.equal(source.slice(style.position.start, style.position.end), '.a{color:red}');
	assert.equal(
		source.slice(result.frontmatterSource.start, result.frontmatterSource.end).at(-1),
		'-',
	);

	for (const [i, mapping] of result.mappings.entries()) {
		const [virtualStart, virtualLength, originalStart, originalLength, kind] = mapping;
		if (kind !== 0) continue;
		assert.equal(
			result.code.slice(virtualStart, virtualStart + virtualLength),
			source.slice(originalStart, originalStart + originalLength),
			`span ${i} is not verbatim`,
		);
	}
});

test('ambientTypes appends unmapped Fragment and Astro declarations', () => {
	const source = '---\nconst a = Astro.props.a;\n---\n<p>{a}</p>';

	const plain = convertToTsx(source);
	assert.ok(!plain.code.includes('declare const Fragment'));
	assert.ok(!plain.code.includes('declare const Astro'));

	const ambient = convertToTsx(source, { ambientTypes: true });
	assert.ok(ambient.code.includes('declare const Fragment: any;'));
	assert.match(ambient.code, /declare const Astro: Readonly<import\('astro'\)\.AstroGlobal</);

	assert.ok(ambient.code.startsWith(plain.code));
	assert.deepEqual(ambient.mappings, plain.mappings);
});

test('reports frontmatter status and positioned diagnostics', () => {
	assert.equal(
		convertToTsx('---\nlet x = 1;\n---\n<p/>').frontmatterStatus,
		AstroFrontmatterStatus.Closed,
	);
	assert.equal(convertToTsx('---\nlet x = 1;\n').frontmatterStatus, AstroFrontmatterStatus.Open);
	assert.equal(convertToTsx('<p/>').frontmatterStatus, AstroFrontmatterStatus.DoesntExist);

	const broken = convertToTsx('<div>{x ==}</div>');
	assert.ok(broken.hasParseErrors);
	assert.ok(broken.diagnostics.length > 0);
	for (const diagnostic of broken.diagnostics) {
		assert.ok(diagnostic.message.length > 0);
		assert.equal(diagnostic.severity, 1);
		assert.ok(diagnostic.position.end >= diagnostic.position.start);
	}
});
