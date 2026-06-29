import assert from 'node:assert/strict';
import { before, describe, it } from 'node:test';
import { type TransformResult, transform, preprocessStyles } from '@astrojs/compiler-rs';
import { preprocessStyle } from '../utils.js';

const FIXTURE = `
---
---

<article class="panel"></article>

<style lang="scss">
.panel {
  :global(> .item:not(.hidden)) {
    display: block;
  }

  :global(> li:has(> a)) {
    list-style: none;
  }

  :global(> .nav:not(.is-active):has(.icon)) {
    opacity: 1;
  }

  :global(> [data-state="(open)"]) {
    visibility: visible;
  }

  :global(> .a + .b) {
    margin-top: 1rem;
  }
}
</style>
`;

describe('styles/global-immediate-child-complex', () => {
	let result: TransformResult;
	before(async () => {
		const preprocessedStyles = await preprocessStyles(FIXTURE, preprocessStyle);
		result = transform(FIXTURE, {
			sourcemap: 'external',
			scopedStyleStrategy: 'attribute',
			preprocessedStyles,
		});
	});

	it('rewrites and scopes complex :global(> …) selectors from Sass nesting', () => {
		const css = result.css[0];
		assert.ok(css, 'Expected scoped CSS output');
		assert.doesNotMatch(css, /:global\(>/);

		assert.match(css, /\.panel\[data-astro-cid-[^\]]+\]\s*>\s*\.item:not\(\.hidden\)/);
		assert.match(css, /\.panel\[data-astro-cid-[^\]]+\]\s*>\s*li:has\(\s*>\s*a\)/);
		assert.match(
			css,
			/\.panel\[data-astro-cid-[^\]]+\]\s*>\s*\.nav:not\(\.is-active\):has\(\.icon\)/,
		);
		assert.match(css, /\.panel\[data-astro-cid-[^\]]+\]\s*>\s*\[data-state="\(open\)"\]/);
		assert.match(css, /\.panel\[data-astro-cid-[^\]]+\]\s*>\s*\.a\s*\+\s*\.b/);

		assert.match(css, /display:\s*block/);
		assert.match(css, /list-style:\s*none/);
		assert.match(css, /visibility:\s*visible/);
		assert.match(css, /margin-top:\s*1rem/);
	});
});
