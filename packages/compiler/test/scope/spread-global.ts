import { transform } from '@astrojs/compiler-rs';
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

// Regression test for https://github.com/withastro/compiler-rs/issues/43
// Spreading an object containing `class` onto an element/component must NOT
// receive a scoped class when the only `<style>` present is `is:global`.

const SCOPE_RE = /astro-[0-9A-Za-z]+/;

// Exact reproduction from the issue.
const REPRO = `---
const linkAttributes = { class: "big", href: "#", rel: "noopener" };
---

<html lang="en">
	<head>
		<meta charset="utf-8" />
		<link rel="icon" type="image/svg+xml" href="/favicon.svg" />
		<meta name="viewport" content="width=device-width" />
		<meta name="generator" content={Astro.generator} />
		<title>Astro</title>
	</head>
	<body>
		<p>
			<a {...linkAttributes}>Astro</a>
		</p>
	</body>
</html>

<style is:global>
	.big { font-size: 3rem; font-weight: 700; }
</style>`;

describe('scope/spread-global', () => {
	it('Does not add a scoped class to a spread when only is:global styles exist', async () => {
		const { code } = await transform(REPRO, {
			normalizedFilename: '/src/pages/index.astro',
		});

		// The spread must be emitted with no scope-class third argument.
		assert.match(code, /\$\$spreadAttributes\(linkAttributes\)/);
		assert.doesNotMatch(code, SCOPE_RE, `Expected no scoped class in output, got:\n${code}`);
	});

	for (const scopedStyleStrategy of ['where', 'class', 'attribute'] as const) {
		it(`Omits the scope for is:global spreads with scopedStyleStrategy="${scopedStyleStrategy}"`, async () => {
			const { code } = await transform(REPRO, {
				normalizedFilename: '/src/pages/index.astro',
				scopedStyleStrategy,
			});

			assert.match(code, /\$\$spreadAttributes\(linkAttributes\)/);
			assert.doesNotMatch(code, SCOPE_RE);
			assert.doesNotMatch(code, /data-astro-cid/);
		});
	}

	it('Does not add a scoped class to a component spread when only is:global styles exist', async () => {
		const input = `---
import Card from '../components/Card.astro';
const props = { class: "big" };
---
<Card {...props} />
<style is:global>div { color: red; }</style>`;
		const { code } = await transform(input, {
			normalizedFilename: '/src/pages/index.astro',
		});

		assert.doesNotMatch(
			code,
			SCOPE_RE,
			`Expected no scoped class in component output, got:\n${code}`,
		);
	});

	it('Still scopes spreads when a non-global style is also present', async () => {
		const input = `---
const props = { class: "big" };
---
<a {...props}>Astro</a>
<style is:global>.g { color: red; }</style>
<style>.s { color: blue; }</style>`;
		const { code } = await transform(input, {
			normalizedFilename: '/src/pages/index.astro',
		});

		// A real scoped style exists, so the scope class must be injected.
		assert.match(code, SCOPE_RE);
		assert.match(code, /\$\$spreadAttributes\(props, undefined, \{ "class": "astro-/);
	});
});
