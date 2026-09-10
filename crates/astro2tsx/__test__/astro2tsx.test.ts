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
	const result = convertToTsx('---\nexport default function f() { return 1 }\n---\n');
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
		const result = convertToTsx(`---\n${frontmatter}\n---\n`);
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
	const result = convertToTsx(source);
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
		const result = convertToTsx(source);
		assert.equal(result.hasParseErrors, false, source);
		assert.deepEqual(result.diagnostics, [], source);
		assert.ok(result.code.includes('v-if'), result.code);
	}
});

test('reports invalid expressions in hyphenated Astro attributes', () => {
	const result = convertToTsx('<Component v-if={visible ==} />');
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
		const result = convertToTsx(source);
		assert.equal(result.hasParseErrors, false, source);
		assert.deepEqual(result.diagnostics, [], source);
		assert.ok(result.code.includes(expected), result.code);
		assert.ok(!result.code.includes('v-for=""'), result.code);
	}

	const malformed = convertToTsx('<Component v-for={items ==} />');
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
		const result = convertToTsx(source);
		assert.equal(result.hasParseErrors, false, source);
		assert.deepEqual(result.diagnostics, [], source);
		assert.ok(result.code.includes('v-for="a&amp;b"'), result.code);
	}

	const malformed = '<div v-for="a&amp;b" /';
	const result = convertToTsx(malformed);
	assert.equal(result.hasParseErrors, true);
	assert.ok(result.diagnostics.length > 0);
	assert.ok(
		result.diagnostics.every((diagnostic) => diagnostic.position.start >= malformed.length - 2),
	);
});

test('reports reconstructed v-for expression diagnostics at document offsets', () => {
	const source = '<main><Component v-for={items ==} /></main>';
	const result = convertToTsx(source);
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
		const result = convertToTsx(source);
		assert.equal(result.hasParseErrors, false, source);
		assert.deepEqual(result.diagnostics, [], source);
		assert.ok(result.code.includes(expected), result.code);
		assert.ok(result.code.includes('data-after="yes"'), result.code);
	}

	const malformed = '<Component v-for={items.map(x => ({x})} data-after="yes" />';
	const result = convertToTsx(malformed);
	assert.equal(result.hasParseErrors, true);
	assert.ok(result.diagnostics.length > 0);
	assert.ok(result.code.includes('v-for={items.map(x => ({x})}'), result.code);
	assert.ok(result.code.includes('data-after="yes"'), result.code);
});

test('escapes invalid attribute string values as JavaScript strings', () => {
	const source =
		"<Component @event='quote\" slash\\ line\n\u2028\u2029\t &NotEqualTilde; &#128; &#0; &#xD800; &#x110000; &copy &amp; &quot; &copycat &amp= &bogus;' />";
	const result = convertToTsx(source);
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

test('returns TypeScript Content Mapper span mappings', () => {
	const source =
		'---\nconst x = 1;\n---\n<article id="main" class=plain data-thing={x}>hello <b>world</b></article>';
	const result = convertToTsx(source, { ambientTypes: true });

	assert.ok(result.mappings.length > 0);
	for (let i = 0; i < result.mappings.length; i++) {
		const [virtualStart, virtualLength, originalStart, originalLength, kind] = result.mappings[i];
		assert.ok(virtualLength > 0, `span ${i} is empty`);
		if (i > 0) {
			const previous = result.mappings[i - 1];
			assert.ok(virtualStart >= previous[0] + previous[1], `span ${i} overlaps its predecessor`);
		}
		if (kind === 0) {
			assert.equal(virtualLength, originalLength);
			assert.equal(
				result.code.slice(virtualStart, virtualStart + virtualLength),
				source.slice(originalStart, originalStart + originalLength),
				`span ${i} is not verbatim`,
			);
		}
	}

	const originalAt = (generated) => {
		for (let i = result.mappings.length - 1; i >= 0; i--) {
			const [virtualStart, virtualLength, originalStart, , kind] = result.mappings[i];
			const delta = generated - virtualStart;
			if (kind === 0 && delta >= 0 && delta < virtualLength) return originalStart + delta;
		}
		return null;
	};

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

	const style = result.styles[0];
	assert.equal(source.slice(style.position.start, style.position.end), '.a{color:red}');
	assert.equal(
		source.slice(result.frontmatterSource.start, result.frontmatterSource.end).at(-1),
		'-',
	);

	for (let i = 0; i < result.mappings.length; i++) {
		const [virtualStart, virtualLength, originalStart, originalLength, kind] = result.mappings[i];
		if (kind !== 0) continue;
		assert.equal(
			result.code.slice(virtualStart, virtualStart + virtualLength),
			source.slice(originalStart, originalStart + originalLength),
			`span ${i} is not verbatim`,
		);
	}
});

test('strips the doctype and leaves its source range unmapped', () => {
	const source =
		'---\nconst a = 1;\n---\n\n<!doctype html>\n<html lang="en"><body>{a}</body></html>\n';
	const result = convertToTsx(source, { filename: 'X.astro' });
	assert.ok(!result.code.includes('<!'), result.code);
	assert.ok(result.code.includes('<html lang="en">'));

	for (let i = 0; i < result.mappings.length; i++) {
		const [virtualStart, virtualLength, originalStart, originalLength, kind] = result.mappings[i];
		if (kind !== 0) continue;
		const original = source.slice(originalStart, originalStart + originalLength);
		assert.equal(result.code.slice(virtualStart, virtualStart + virtualLength), original);
		assert.ok(!original.includes('doctype'), `span ${i} maps into the doctype`);
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
		const result = convertToTsx(input);
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
