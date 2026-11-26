import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { transform } from '@astrojs/compiler-rs';

const source = `---
const x = 1;
---
<div class="a"><p>{x}</p></div>
<style>
.a > p:hover { color: red }
:global(body) .a { margin: 0 }
@media (min-width: 600px) { .a { padding: 4px } }
</style>`;

const base = { filename: '/src/components/Cache.astro', scopedStyleStrategy: 'attribute' } as const;

describe('cssScopeCache', () => {
	const off = transform(source, base);

	it('is off by default and with maxBytes 0: same output', () => {
		assert.deepEqual(transform(source, { ...base, cssScopeCache: {} }).code, off.code);
		assert.deepEqual(transform(source, { ...base, cssScopeCache: { maxBytes: 0 } }).code, off.code);
	});

	it('gives byte-identical code and css when on, first call and reuse', () => {
		const opts = { ...base, cssScopeCache: { maxBytes: 1 << 20 } };
		for (let i = 0; i < 3; i++) {
			const on = transform(source, opts);
			assert.equal(on.code, off.code);
			assert.deepEqual(on.css, off.css);
			assert.equal(on.scope, off.scope);
		}
	});

	it('keeps output identical when the budget is too small to keep anything', () => {
		const tiny = transform(source, { ...base, cssScopeCache: { maxBytes: 8 } });
		assert.equal(tiny.code, off.code);
	});

	it('does not mix scopes or strategies', () => {
		const opts = { cssScopeCache: { maxBytes: 1 << 20 } };
		const a = transform(source, {
			...opts,
			filename: '/a.astro',
			scopedStyleStrategy: 'attribute',
		});
		const b = transform(source, {
			...opts,
			filename: '/b.astro',
			scopedStyleStrategy: 'attribute',
		});
		const c = transform(source, { ...opts, filename: '/a.astro', scopedStyleStrategy: 'class' });
		assert.notEqual(a.scope, b.scope);
		assert.equal(
			a.code,
			transform(source, { filename: '/a.astro', scopedStyleStrategy: 'attribute' }).code,
		);
		assert.equal(
			b.code,
			transform(source, { filename: '/b.astro', scopedStyleStrategy: 'attribute' }).code,
		);
		assert.equal(
			c.code,
			transform(source, { filename: '/a.astro', scopedStyleStrategy: 'class' }).code,
		);
	});
});
