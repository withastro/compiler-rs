import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { convertToTsx } from '../index.js';

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

	const exportName = result.code.indexOf('AstroComponent');
	assert.deepEqual(
		result.mappings.find((mapping) => mapping[0] === exportName),
		[exportName, 'AstroComponent'.length, 0, 0, 1, (1 << 3) | (1 << 6)],
	);
});

test('component export mappings and metadata use UTF-16 offsets', () => {
	const source = '---\nconst text = "𝒳";\n---';
	for (const [filename, name] of [
		['MyPage.astro', 'MyPageAstroComponent'],
		['Ünicorn.astro', 'ÜnicornAstroComponent'],
		['[slug].astro', '_Slug_AstroComponent'],
		['404.astro', 'FourOhFourAstroComponent'],
	]) {
		const result = convertToTsx(source, { filename });
		const start =
			result.code.indexOf('export default function ') + 'export default function '.length;
		assert.deepEqual(
			result.mappings.find((mapping) => mapping[0] === start),
			[start, name.length, 0, 0, 1, (1 << 3) | (1 << 6)],
		);
		const range = result.generatedComponentExport;
		if (filename === 'MyPage.astro') {
			assert.ok(range);
			assert.equal(
				result.code.slice(range.start, range.end),
				'export { MyPageAstroComponent as MyPage };\n',
			);
			assert.equal(range.end, result.code.length);
		} else {
			assert.equal(range, undefined);
		}
	}
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
