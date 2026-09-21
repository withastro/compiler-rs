import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { transform } from '@astrojs/compiler-rs';
import { transform as transformAsync } from '@astrojs/compiler-rs/async';

describe('inlineComponentAssets', () => {
	it('preserves standard asset emission by default', () => {
		const result = transform(
			`<style>h1 { color: red; }</style><script>console.log('before')</script><h1>Hello</h1>`,
			{ filename: 'Component.astro' },
		);

		assert.match(result.code, /Component\.astro\?astro&type=style&index=0&lang\.css/);
		assert.doesNotMatch(result.code, /<style>/);
		assert.ok(
			result.code.indexOf('Component.astro?astro&type=script&index=0&lang.ts') <
				result.code.indexOf('<h1'),
		);
	});

	it('renders all component asset forms in the expected order', () => {
		const source = `<style>h1 { color: red; }</style>
<style is:inline>.inline { color: blue; }</style>
<script>console.log('before')</script>
<script is:inline>console.log('inline')</script>
<h1 class="inline">Hello</h1>
<script>console.log('after')</script>`;
		const result = transform(source, {
			filename: 'Component.astro',
			inlineComponentAssets: true,
			sourcemap: 'external',
		});

		assert.doesNotMatch(result.code, /\?astro&type=style/);
		const extractedStyle = result.code.indexOf('<style>h1:where');
		const inlineStyle = result.code.indexOf('<style>.inline');
		const inlineScript = result.code.indexOf("<script>console.log('inline')");
		const component = result.code.indexOf('<h1');
		const firstScript = result.code.indexOf('Component.astro?astro&type=script&index=0&lang.ts');
		const secondScript = result.code.indexOf('Component.astro?astro&type=script&index=1&lang.ts');

		assert.ok(extractedStyle !== -1 && extractedStyle < inlineStyle);
		assert.ok(inlineStyle < inlineScript && inlineScript < component);
		assert.ok(component < firstScript && firstScript < secondScript);

		const map = JSON.parse(result.map);
		assert.equal(map.sourcesContent[0], source);
		assert.ok(map.mappings.length > 0);
	});

	it('supports the async compiler entry point', async () => {
		const result = await transformAsync(
			`<style>p { color: red; }</style><p>Async</p><script>console.log('async')</script>`,
			{
				filename: 'Async.astro',
				inlineComponentAssets: true,
			},
		);

		assert.match(result.code, /<style>p:where/);
		assert.ok(
			result.code.indexOf('<p') <
				result.code.indexOf('Async.astro?astro&type=script&index=0&lang.ts'),
		);
	});
});
