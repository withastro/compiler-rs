import assert from 'node:assert/strict';
import { before, describe, it } from 'node:test';
import { type TransformResult, transform, preprocessStyles } from '@astrojs/compiler-rs';
import { preprocessStyle } from '../utils.js';

const FIXTURE = `
---
---

<article></article>

<style lang="scss">
article {
  :global(> *) {
    flex-shrink: 0;
    flex-basis: 100%;
    scroll-snap-align: center;
  }
}
</style>
`;

describe('styles/global-immediate-child', () => {
	let result: TransformResult;
	before(async () => {
		const preprocessedStyles = await preprocessStyles(FIXTURE, preprocessStyle);
		result = transform(FIXTURE, {
			sourcemap: 'external',
			scopedStyleStrategy: 'attribute',
			preprocessedStyles,
		});
	});

	it('scopes article and keeps direct child selectors global', () => {
		const css = result.css[0];
		assert.ok(css, 'Expected scoped CSS output');
		assert.match(css, /article\[data-astro-cid-[^\]]+\]\s*>\s*\*/);
		assert.match(css, /flex-basis:\s*100%/);
		assert.doesNotMatch(css, /:global\(> \*\)/);
	});
});
