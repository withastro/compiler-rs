import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { NO_ORIGINAL, convertToTsx } from '../index.js';

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

test('mappings are packed pairs ascending by generated offset', () => {
	const result = convertToTsx('<h1>Hi</h1>');
	assert.ok(result.mappings instanceof Uint32Array);
	assert.equal(result.mappings.length % 2, 0);
	let mapped = 0;
	for (let i = 0; i < result.mappings.length; i += 2) {
		if (i > 0) assert.ok(result.mappings[i] >= result.mappings[i - 2], 'generated offsets ascend');
		if (result.mappings[i + 1] !== NO_ORIGINAL) mapped++;
	}
	assert.ok(mapped > 0, 'expected at least one mapped byte');
	const h1 = result.code.indexOf('<h1>');
	const pair = [...Array(result.mappings.length / 2).keys()].find(
		(n) => result.mappings[2 * n] === h1,
	);
	assert.notEqual(result.mappings[2 * pair + 1], NO_ORIGINAL, '<h1> maps back to the source');
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
