import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { AstroFrontmatterStatus, convertToTsx } from '../index.js';

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
