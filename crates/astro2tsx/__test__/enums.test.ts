import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
	AstroFrontmatterStatus,
	convertToTsx,
	DiagnosticSeverity,
	ExtractedScriptType,
	ExtractedStyleType,
} from '../index.js';

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
