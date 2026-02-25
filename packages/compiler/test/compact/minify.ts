import { transform } from '@astrojs/compiler-rs';
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

async function minify(input: string) {
	const code = transform(input, { compact: true }).code;
	return code.replace('${$$maybeRenderHead($$result)}', '');
}

// Note: basic, preservation, collapsing, jsx, and newline tests are covered
// by Rust snapshot fixtures in crates/astro_codegen/tests/fixtures/compact__*.astro.
// Only parametric tests that don't map cleanly to single fixtures remain here.

describe('compact/minify', () => {
	it('space normalization between attributes', async () => {
		assert.ok((await minify('<p title="bar">foo</p>')).includes('<p title="bar">foo</p>'));
		assert.ok((await minify('<img src="test"/>')).includes('<img src="test">'));
		assert.ok((await minify('<p title = "bar">foo</p>')).includes('<p title="bar">foo</p>'));
		assert.ok(
			(await minify('<p title\n\n\t  =\n     "bar">foo</p>')).includes('<p title="bar">foo</p>'),
		);
		assert.ok((await minify('<img src="test" \n\t />')).includes('<img src="test">'));
		assert.ok(
			(await minify('<input title="bar"       id="boo"    value="hello world">')).includes(
				'<input title="bar" id="boo" value="hello world">',
			),
		);
	});

	it('space normalization around text', async () => {
		assert.ok((await minify('   <p>blah</p>\n\n\n   ')).includes('<p>blah</p>'));
		assert.ok((await minify('<p>foo <img> bar</p>')).includes('<p>foo <img> bar</p>'));
		assert.ok((await minify('<p>foo<img>bar</p>')).includes('<p>foo<img>bar</p>'));
		assert.ok((await minify('<p>foo <img>bar</p>')).includes('<p>foo <img>bar</p>'));
		assert.ok((await minify('<p>foo<img> bar</p>')).includes('<p>foo<img> bar</p>'));
		assert.ok((await minify('<p>foo <wbr> bar</p>')).includes('<p>foo <wbr> bar</p>'));
		assert.ok((await minify('<p>foo<wbr>bar</p>')).includes('<p>foo<wbr>bar</p>'));
		assert.ok((await minify('<p>foo <wbr>bar</p>')).includes('<p>foo <wbr>bar</p>'));
		assert.ok((await minify('<p>foo<wbr> bar</p>')).includes('<p>foo<wbr> bar</p>'));
		assert.ok(
			(await minify('<p>foo <wbr baz moo=""> bar</p>')).includes('<p>foo <wbr baz moo=""> bar</p>'),
		);
		assert.ok(
			(await minify('<p>foo<wbr baz moo="">bar</p>')).includes('<p>foo<wbr baz moo="">bar</p>'),
		);
		assert.ok(
			(await minify('<p>foo <wbr baz moo="">bar</p>')).includes('<p>foo <wbr baz moo="">bar</p>'),
		);
		assert.ok(
			(await minify('<p>foo<wbr baz moo=""> bar</p>')).includes('<p>foo<wbr baz moo=""> bar</p>'),
		);
		assert.ok(
			(await minify('<p>  <a href="#">  <code>foo</code></a> bar</p>')).includes(
				'<p> <a href="#"> <code>foo</code></a> bar</p>',
			),
		);
		assert.ok(
			(await minify('<p><a href="#"><code>foo  </code></a> bar</p>')).includes(
				'<p><a href="#"><code>foo </code></a> bar</p>',
			),
		);
		assert.ok(
			(await minify('<p>  <a href="#">  <code>   foo</code></a> bar   </p>')).includes(
				'<p> <a href="#"> <code> foo</code></a> bar </p>',
			),
		);
		assert.ok(
			(await minify('<div> Empty <!-- or --> not </div>')).includes(
				'<div> Empty <!-- or --> not </div>',
			),
		);
		assert.ok(
			(await minify('<div> a <input><!-- b --> c </div>')).includes(
				'<div> a <input><!-- b --> c </div>',
			),
		);
		await Promise.all(
			[
				'a',
				'abbr',
				'acronym',
				'b',
				'big',
				'del',
				'em',
				'font',
				'i',
				'ins',
				'kbd',
				'mark',
				's',
				'samp',
				'small',
				'span',
				'strike',
				'strong',
				'sub',
				'sup',
				'time',
				'tt',
				'u',
				'var',
			].map(async (el) => {
				const [open, close] = [`<${el}>`, `</${el}>`];
				assert.ok(
					(await minify(`foo ${open}baz${close} bar`)).includes(`foo ${open}baz${close} bar`),
				);
				assert.ok((await minify(`foo${open}baz${close}bar`)).includes(`foo${open}baz${close}bar`));
				assert.ok(
					(await minify(`foo ${open}baz${close}bar`)).includes(`foo ${open}baz${close}bar`),
				);
				assert.ok(
					(await minify(`foo${open}baz${close} bar`)).includes(`foo${open}baz${close} bar`),
				);
				assert.ok(
					(await minify(`foo ${open} baz ${close} bar`)).includes(`foo ${open} baz ${close} bar`),
				);
				assert.ok(
					(await minify(`foo${open} baz ${close}bar`)).includes(`foo${open} baz ${close}bar`),
				);
				assert.ok(
					(await minify(`foo ${open} baz ${close}bar`)).includes(`foo ${open} baz ${close}bar`),
				);
				assert.ok(
					(await minify(`foo${open} baz ${close} bar`)).includes(`foo${open} baz ${close} bar`),
				);
				assert.ok(
					(await minify(`<div>foo ${open}baz${close} bar</div>`)).includes(
						`<div>foo ${open}baz${close} bar</div>`,
					),
				);
				assert.ok(
					(await minify(`<div>foo${open}baz${close}bar</div>`)).includes(
						`<div>foo${open}baz${close}bar</div>`,
					),
				);
				assert.ok(
					(await minify(`<div>foo ${open}baz${close}bar</div>`)).includes(
						`<div>foo ${open}baz${close}bar</div>`,
					),
				);
				assert.ok(
					(await minify(`<div>foo${open}baz${close} bar</div>`)).includes(
						`<div>foo${open}baz${close} bar</div>`,
					),
				);
				assert.ok(
					(await minify(`<div>foo ${open} baz ${close} bar</div>`)).includes(
						`<div>foo ${open} baz ${close} bar</div>`,
					),
				);
				assert.ok(
					(await minify(`<div>foo${open} baz ${close}bar</div>`)).includes(
						`<div>foo${open} baz ${close}bar</div>`,
					),
				);
				assert.ok(
					(await minify(`<div>foo ${open} baz ${close}bar</div>`)).includes(
						`<div>foo ${open} baz ${close}bar</div>`,
					),
				);
				assert.ok(
					(await minify(`<div>foo${open} baz ${close} bar</div>`)).includes(
						`<div>foo${open} baz ${close} bar</div>`,
					),
				);
			}),
		);
		// Don't trim whitespace around element, but do trim within
		await Promise.all(
			['bdi', 'bdo', 'button', 'cite', 'code', 'dfn', 'math', 'q', 'rt', 'rtc', 'ruby', 'svg'].map(
				async (el) => {
					const [open, close] = [`<${el}>`, `</${el}>`];
					assert.ok(
						(await minify(`foo ${open}baz${close} bar`)).includes(`foo ${open}baz${close} bar`),
					);
					assert.ok(
						(await minify(`foo${open}baz${close}bar`)).includes(`foo${open}baz${close}bar`),
					);
					assert.ok(
						(await minify(`foo ${open}baz${close}bar`)).includes(`foo ${open}baz${close}bar`),
					);
					assert.ok(
						(await minify(`foo${open}baz${close} bar`)).includes(`foo${open}baz${close} bar`),
					);
					assert.ok(
						(await minify(`foo ${open} baz ${close} bar`)).includes(`foo ${open} baz ${close} bar`),
					);
					assert.ok(
						(await minify(`foo${open} baz ${close}bar`)).includes(`foo${open} baz ${close}bar`),
					);
					assert.ok(
						(await minify(`foo ${open} baz ${close}bar`)).includes(`foo ${open} baz ${close}bar`),
					);
					assert.ok(
						(await minify(`foo${open} baz ${close} bar`)).includes(`foo${open} baz ${close} bar`),
					);
					assert.ok(
						(await minify(`<div>foo ${open}baz${close} bar</div>`)).includes(
							`<div>foo ${open}baz${close} bar</div>`,
						),
					);
					assert.ok(
						(await minify(`<div>foo${open}baz${close}bar</div>`)).includes(
							`<div>foo${open}baz${close}bar</div>`,
						),
					);
					assert.ok(
						(await minify(`<div>foo ${open}baz${close}bar</div>`)).includes(
							`<div>foo ${open}baz${close}bar</div>`,
						),
					);
					assert.ok(
						(await minify(`<div>foo${open}baz${close} bar</div>`)).includes(
							`<div>foo${open}baz${close} bar</div>`,
						),
					);
					assert.ok(
						(await minify(`<div>foo ${open} baz ${close} bar</div>`)).includes(
							`<div>foo ${open} baz ${close} bar</div>`,
						),
					);
					assert.ok(
						(await minify(`<div>foo${open} baz ${close}bar</div>`)).includes(
							`<div>foo${open} baz ${close}bar</div>`,
						),
					);
					assert.ok(
						(await minify(`<div>foo ${open} baz ${close}bar</div>`)).includes(
							`<div>foo ${open} baz ${close}bar</div>`,
						),
					);
					assert.ok(
						(await minify(`<div>foo${open} baz ${close} bar</div>`)).includes(
							`<div>foo${open} baz ${close} bar</div>`,
						),
					);
				},
			),
		);
		await Promise.all(
			[
				['<span> foo </span>', '<span> foo </span>'],
				[' <span> foo </span> ', '<span> foo </span>'],
				['<nobr>a</nobr>', '<nobr>a</nobr>'],
				['<nobr>a </nobr>', '<nobr>a </nobr>'],
				['<nobr> a</nobr>', '<nobr> a</nobr>'],
				['<nobr> a </nobr>', '<nobr> a </nobr>'],
				['a<nobr>b</nobr>c', 'a<nobr>b</nobr>c'],
				['a<nobr>b </nobr>c', 'a<nobr>b </nobr>c'],
				['a<nobr> b</nobr>c', 'a<nobr> b</nobr>c'],
				['a<nobr> b </nobr>c', 'a<nobr> b </nobr>c'],
				['a<nobr>b</nobr> c', 'a<nobr>b</nobr> c'],
				['a<nobr>b </nobr> c', 'a<nobr>b </nobr> c'],
				['a<nobr> b</nobr> c', 'a<nobr> b</nobr> c'],
				['a<nobr> b </nobr> c', 'a<nobr> b </nobr> c'],
				['a <nobr>b</nobr>c', 'a <nobr>b</nobr>c'],
				['a <nobr>b </nobr>c', 'a <nobr>b </nobr>c'],
				['a <nobr> b</nobr>c', 'a <nobr> b</nobr>c'],
				['a <nobr> b </nobr>c', 'a <nobr> b </nobr>c'],
				['a <nobr>b</nobr> c', 'a <nobr>b</nobr> c'],
				['a <nobr>b </nobr> c', 'a <nobr>b </nobr> c'],
				['a <nobr> b</nobr> c', 'a <nobr> b</nobr> c'],
				['a <nobr> b </nobr> c', 'a <nobr> b </nobr> c'],
			].map(async ([input, output]) => {
				assert.ok((await minify(input)).includes(output as string));
			}),
		);
	});
});
