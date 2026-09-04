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

test('preserves returns in default-exported functions', () => {
	const result = convertToTsx('---\nexport default function f() { return 1 }\n---\n', {
		sourcemap: false,
	});
	assert.match(result.code, /function f\(\) \{ return 1 \}/);
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

test('detects escaped getStaticPaths bindings without matching nested bindings', () => {
	for (const [frontmatter, expected] of [
		['export const get\\u0053taticPaths = () => [];', true],
		['export function f(get\\u0053taticPaths) {}', false],
		['export function f() { const get\\u0053taticPaths = () => []; }', false],
	] as const) {
		const result = convertToTsx(`---\n${frontmatter}\n---\n`, { sourcemap: false });
		assert.equal(result.code.includes('ReturnType<typeof getStaticPaths>'), expected, frontmatter);
	}
});

test('reports parse errors but still produces output', () => {
	const result = convertToTsx('<div');
	assert.equal(result.hasParseErrors, true);
	assert.ok(result.code.length > 0);
});

test('reports invalid frontmatter without blocking output', () => {
	const source = '---\nconst = ;\n---\n<p/>';
	const result = convertToTsx(source, { sourcemap: false });
	assert.equal(result.hasParseErrors, true);
	assert.ok(result.diagnostics.length > 0);
	assert.ok(result.code.includes('const = ;'));
	for (const diagnostic of result.diagnostics) {
		assert.ok(diagnostic.position.start >= 4);
		assert.ok(diagnostic.position.end <= 13);
	}
});

test('emits hyphenated Astro attributes as ordinary TSX attributes', () => {
	for (const source of ['<div v-if />', '<div v-if=visible />', '<Component v-if={visible} />']) {
		const result = convertToTsx(source, { sourcemap: false });
		assert.equal(result.hasParseErrors, false, source);
		assert.deepEqual(result.diagnostics, [], source);
		assert.ok(result.code.includes('v-if'), result.code);
	}
});

test('reports invalid expressions in hyphenated Astro attributes', () => {
	const result = convertToTsx('<Component v-if={visible ==} />', { sourcemap: false });
	assert.equal(result.hasParseErrors, true);
	assert.ok(result.diagnostics.length > 0);
});

test('preserves complete v-for attributes and validates expression values', () => {
	for (const [source, expected] of [
		['<div v-for="item in items" />', 'v-for="item in items"'],
		['<div v-for=items />', 'v-for="items"'],
		['<div v-for={items} />', 'v-for={items}'],
		['<div v-for />', 'v-for'],
		['<Component v-for={items} />', 'v-for={items}'],
	]) {
		const result = convertToTsx(source, { sourcemap: false });
		assert.equal(result.hasParseErrors, false, source);
		assert.deepEqual(result.diagnostics, [], source);
		assert.ok(result.code.includes(expected), result.code);
		assert.ok(!result.code.includes('v-for=""'), result.code);
	}

	const malformed = convertToTsx('<Component v-for={items ==} />', { sourcemap: false });
	assert.equal(malformed.hasParseErrors, true);
	assert.ok(malformed.diagnostics.length > 0);
	assert.ok(malformed.code.includes('v-for={items ==}'), malformed.code);
});

test('suppresses only recovery diagnostics within reconstructed v-for attributes', () => {
	for (const source of [
		'<div v-for="a&amp;b" />',
		'<div v-for=a&amp;b />',
		'<Component v-for="a&amp;b" />',
		'<Component v-for=a&amp;b />',
	]) {
		const result = convertToTsx(source, { sourcemap: false });
		assert.equal(result.hasParseErrors, false, source);
		assert.deepEqual(result.diagnostics, [], source);
		assert.ok(result.code.includes('v-for="a&amp;b"'), result.code);
	}

	const malformed = '<div v-for="a&amp;b" /';
	const result = convertToTsx(malformed, { sourcemap: false });
	assert.equal(result.hasParseErrors, true);
	assert.ok(result.diagnostics.length > 0);
	assert.ok(
		result.diagnostics.every((diagnostic) => diagnostic.position.start >= malformed.length - 2),
	);
});

test('reports reconstructed v-for expression diagnostics at document offsets', () => {
	const source = '<main><Component v-for={items ==} /></main>';
	const result = convertToTsx(source, { sourcemap: false });
	const expected = source.indexOf('}');
	const diagnostic = result.diagnostics.at(-1);
	assert.ok(diagnostic);
	assert.deepEqual(diagnostic.position, { start: expected, end: expected });
	assert.equal(source.slice(diagnostic.position.start, diagnostic.position.end), '');
});

test('preserves complete nested v-for expression boundaries', () => {
	for (const [source, expected] of [
		['<Component v-for={{a: 1}} data-after="yes" />', 'v-for={{a: 1}}'],
		[
			'<Component v-for={items.map(x => ({x}))} data-after="yes" />',
			'v-for={items.map(x => ({x}))}',
		],
		['<Component v-for={fn("}")} data-after="yes" />', 'v-for={fn("}")}'],
		[
			'<Component v-for={`item-${items.map(x => ({x}))}`} data-after="yes" />',
			'v-for={`item-${items.map(x => ({x}))}`}',
		],
		[
			'<Component v-for={items.map(/* } */ x => ({x}))} data-after="yes" />',
			'v-for={items.map(/* } */ x => ({x}))}',
		],
		[
			'<Component v-for={items.filter(x => /}/.test(x))} data-after="yes" />',
			'v-for={items.filter(x => /}/.test(x))}',
		],
	] as const) {
		const result = convertToTsx(source, { sourcemap: false });
		assert.equal(result.hasParseErrors, false, source);
		assert.deepEqual(result.diagnostics, [], source);
		assert.ok(result.code.includes(expected), result.code);
		assert.ok(result.code.includes('data-after="yes"'), result.code);
	}

	const malformed = '<Component v-for={items.map(x => ({x})} data-after="yes" />';
	const result = convertToTsx(malformed, { sourcemap: false });
	assert.equal(result.hasParseErrors, true);
	assert.ok(result.diagnostics.length > 0);
	assert.ok(result.code.includes('v-for={items.map(x => ({x})}'), result.code);
	assert.ok(result.code.includes('data-after="yes"'), result.code);
});

test('escapes invalid attribute string values as JavaScript strings', () => {
	const source =
		"<Component @event='quote\" slash\\ line\n\u2028\u2029\t &NotEqualTilde; &#128; &#0; &#xD800; &#x110000; &copy &amp; &quot; &copycat &amp= &bogus;' />";
	const result = convertToTsx(source, { sourcemap: false });
	const sourceFile = ts.createSourceFile(
		'x.tsx',
		result.code,
		ts.ScriptTarget.Latest,
		false,
		ts.ScriptKind.TSX,
	);
	const diagnostics = (sourceFile as unknown as { parseDiagnostics: unknown[] }).parseDiagnostics;
	assert.deepEqual(diagnostics, []);
	assert.ok(
		result.code.includes(
			'"quote\\\" slash\\\\ line\\n\\u2028\\u2029\\t ≂̸ € � � � © & \\\" &copycat &amp= &bogus;"',
		),
	);
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

	assert.equal(originalAt(result.code.indexOf('<Fragment>')), null);
	assert.equal(originalAt(result.code.indexOf('export default function')), null);
});

test('returns a self-contained source map v3', () => {
	const input = '---\nlet x = 1;\n---\n<p>Hi</p>';
	const map = JSON.parse(convertToTsx(input).map!);
	assert.equal(map.version, 3);
	assert.deepEqual(map.sources, ['input.astro']);
	assert.deepEqual(map.sourcesContent, [input]);
	assert.deepEqual(map.names, []);
	assert.ok(map.mappings.length > 0);
});

test('names the source after the filename option', () => {
	const map = JSON.parse(convertToTsx('<p></p>', { filename: 'Index.astro' }).map!);
	assert.deepEqual(map.sources, ['Index.astro']);
});

test('appends the inline source map comment by default', () => {
	const { code, map } = convertToTsx('<p>Hi</p>');
	const marker = '\n//# sourceMappingURL=data:application/json;charset=utf-8;base64,';
	assert.ok(code.includes(marker));
	const blob = code.slice(code.indexOf(marker) + marker.length);
	assert.deepEqual(JSON.parse(Buffer.from(blob, 'base64').toString('utf8')), JSON.parse(map!));
});

test("sourcemap: 'external' leaves the code without the comment", () => {
	const inline = convertToTsx('<p>Hi</p>');
	const external = convertToTsx('<p>Hi</p>', { sourcemap: 'external' });
	assert.doesNotMatch(external.code, /sourceMappingURL/);
	assert.ok(inline.code.startsWith(external.code));
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
	const result = convertToTsx(source, { sourcemap: 'external' });

	const style = result.styles[0];
	assert.equal(source.slice(style.position.start, style.position.end), '.a{color:red}');
	assert.equal(
		source.slice(result.frontmatterSource.start, result.frontmatterSource.end).at(-1),
		'-',
	);

	const { generatedOffsets, sourceOffsets, lengths } = result;
	for (let i = 0; i < generatedOffsets.length; i++) {
		assert.equal(
			result.code.slice(generatedOffsets[i], generatedOffsets[i] + lengths[i]),
			source.slice(sourceOffsets[i], sourceOffsets[i] + lengths[i]),
			`run ${i} is not verbatim`,
		);
	}
});

test('strips the doctype and leaves its source range unmapped', () => {
	const source =
		'---\nconst a = 1;\n---\n\n<!doctype html>\n<html lang="en"><body>{a}</body></html>\n';
	const result = convertToTsx(source, { filename: 'X.astro', sourcemap: false });
	assert.ok(!result.code.includes('<!'), result.code);
	assert.ok(result.code.includes('<html lang="en">'));

	const { generatedOffsets, sourceOffsets, lengths } = result;
	for (let i = 0; i < generatedOffsets.length; i++) {
		const original = source.slice(sourceOffsets[i], sourceOffsets[i] + lengths[i]);
		assert.equal(
			result.code.slice(generatedOffsets[i], generatedOffsets[i] + lengths[i]),
			original,
		);
		assert.ok(!original.includes('doctype'), `run ${i} maps into the doctype`);
	}
});

test('ambientTypes appends unmapped Fragment and Astro declarations', () => {
	const source = '---\nconst a = Astro.props.a;\n---\n<p>{a}</p>';

	const plain = convertToTsx(source, { sourcemap: false });
	assert.ok(!plain.code.includes('declare const Fragment'));
	assert.ok(!plain.code.includes('declare const Astro'));

	const ambient = convertToTsx(source, { sourcemap: false, ambientTypes: true });
	assert.ok(ambient.code.includes('declare const Fragment: any;'));
	assert.match(ambient.code, /declare const Astro: Readonly<import\('astro'\)\.AstroGlobal</);

	assert.ok(ambient.code.startsWith(plain.code));
	assert.deepEqual(Array.from(ambient.generatedOffsets), Array.from(plain.generatedOffsets));
	assert.deepEqual(Array.from(ambient.sourceOffsets), Array.from(plain.sourceOffsets));
	assert.deepEqual(Array.from(ambient.lengths), Array.from(plain.lengths));
});

test('everyday inputs that used to break TS parsing now emit valid TSX', () => {
	const inputs = [
		'<p>a < b</p>',
		'<div>5 < 10 is true</div>',
		'---\nif (cond) {\n\treturn;\n}\n---\n<p/>',
		'---\nconst a = 1;\n---\n<!doctype html>\n<html><body>{a}</body></html>',
		'<div>hi</div>\n<!DOCTYPE html>\n<p>after</p>',
		"<div data-x='a\"b'></div>",
		'<Comp\n  foo={bar}\n/>',
	];
	for (const input of inputs) {
		const result = convertToTsx(input, { sourcemap: false });
		const sourceFile = ts.createSourceFile(
			'x.tsx',
			result.code,
			ts.ScriptTarget.Latest,
			false,
			ts.ScriptKind.TSX,
		);
		const diagnostics = (sourceFile as unknown as { parseDiagnostics: { messageText: unknown }[] })
			.parseDiagnostics;
		assert.deepEqual(
			diagnostics.map((d) => `${JSON.stringify(input)}: ${JSON.stringify(d.messageText)}`),
			[],
			`invalid TSX for ${JSON.stringify(input)}:\n${result.code}`,
		);
	}
});

test('sourcemap accepts every documented string form', () => {
	assert.equal(convertToTsx('<p/>', { sourcemap: 'none' }).map, undefined);
	assert.equal(convertToTsx('<p/>', { sourcemap: 'false' }).map, undefined);
	assert.ok(convertToTsx('<p/>', { sourcemap: 'external' }).map);

	const garbage = convertToTsx('<p/>', { sourcemap: 'bananas' });
	assert.ok(garbage.map);
	assert.match(garbage.code, /sourceMappingURL/);
});

test('reports frontmatter status and positioned diagnostics', () => {
	assert.equal(convertToTsx('---\nlet x = 1;\n---\n<p/>').frontmatterStatus, 'closed');
	assert.equal(convertToTsx('---\nlet x = 1;\n').frontmatterStatus, 'open');
	assert.equal(convertToTsx('<p/>').frontmatterStatus, 'doesnt-exist');

	const broken = convertToTsx('<div>{x ==}</div>');
	assert.ok(broken.hasParseErrors);
	assert.ok(broken.diagnostics.length > 0);
	for (const diagnostic of broken.diagnostics) {
		assert.ok(diagnostic.message.length > 0);
		assert.equal(diagnostic.severity, 1);
		assert.ok(diagnostic.position.end >= diagnostic.position.start);
	}
});

test('sourcemap: false skips building the map', () => {
	const source = '---\nconst x = 1;\n---\n<p>{x}</p>';
	const skipped = convertToTsx(source, { sourcemap: false });
	assert.equal(skipped.map, undefined);
	assert.doesNotMatch(skipped.code, /sourceMappingURL/);

	const external = convertToTsx(source, { sourcemap: 'external' });
	assert.equal(skipped.code, external.code);
	assert.deepEqual(Array.from(skipped.generatedOffsets), Array.from(external.generatedOffsets));
	assert.deepEqual(Array.from(skipped.lengths), Array.from(external.lengths));
	assert.ok(external.map);
});
