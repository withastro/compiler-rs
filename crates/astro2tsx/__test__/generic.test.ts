import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { convertToTsx } from '../index.js';
import { strictElements, typecheckAstro, typecheck } from './typecheck.ts';

test('generic Props retain useful global prop checking', () => {
	for (const declaration of [
		'interface Props<T> { items: T[] }',
		'interface Props<out T> { items: T[] }',
		'interface Props<T extends string> { items: T[] }',
		'interface Props<T = string> { items: T[] }',
		'interface Props<T, U extends T = T> { items: U[] }',
	]) {
		const diagnostics = typecheckAstro(`${declaration}\nconst bad: number = Astro.props.items;`);
		assert.deepEqual(
			diagnostics.map((d) => [d.code, d.text]),
			[[2322, 'bad']],
			JSON.stringify(diagnostics),
		);
	}
});

test('generic component signatures retain inference and constraints', () => {
	const source = '---\ninterface Props<out T extends string> { value: T }\n---';
	const { code } = convertToTsx(source, { ambientTypes: true });
	assert.deepEqual(typecheck(`${code}\nAstroComponent({ value: 'ok' });`), []);
	const diagnostics = typecheck(`${code}\nAstroComponent({ value: 42 });`);
	assert.deepEqual(
		diagnostics.map((d) => d.code),
		[2322],
		JSON.stringify(diagnostics),
	);
});

const codes = (diagnostics: { code: number; text?: string }[]) =>
	diagnostics.map((d) => [d.code, d.text]);

test('generic Props stay generic inside the frontmatter and template', () => {
	const diagnostics = typecheckAstro(
		[
			'interface Props<T extends string> { item: T; count: number }',
			'const { item, count } = Astro.props;',
			'item.toUpperCase();',
			'item.toFixed();',
			'const n: number = item;',
			'count.toUpperCase();',
		].join('\n'),
		'<p>{item.toFixed()}</p>',
	);
	assert.deepEqual(codes(diagnostics), [
		[2551, 'toFixed'],
		[2322, 'n'],
		[2339, 'toUpperCase'],
		[2551, 'toFixed'],
	]);
});

test('structured and unconstrained type parameters keep their bounds', () => {
	const structured = typecheckAstro(
		[
			'interface Props<T extends { id: string }> { item: T; items: T[] }',
			'Astro.props.item.id.toUpperCase();',
			'Astro.props.item.id.toFixed();',
			'Astro.props.item.nonexistent();',
			'Astro.props.items[0]!.id.toFixed();',
		].join('\n'),
	);
	assert.deepEqual(codes(structured), [
		[2551, 'toFixed'],
		[2339, 'nonexistent'],
		[2551, 'toFixed'],
	]);

	const unconstrained = typecheckAstro('interface Props<T> { item: T }\nAstro.props.item.foo;');
	assert.deepEqual(codes(unconstrained), [[2339, 'foo']]);
});

test('dependent type parameters are not collapsed to their constraints', () => {
	const diagnostics = typecheckAstro(
		[
			'interface Props<T, U extends T = T> { a: T; b: U }',
			'const widened: typeof Astro.props.a = Astro.props.b;',
			'const narrowed: typeof Astro.props.b = Astro.props.a;',
		].join('\n'),
	);
	assert.deepEqual(codes(diagnostics), [[2322, 'narrowed']]);
});

test('hoisted module items stay usable from the generic render scope', () => {
	const diagnostics = typecheckAstro(
		[
			'export interface Props<T extends string> { value?: T }',
			'const { value } = Astro.props;',
			"if (!value) return Astro.redirect('/');",
			'export const prerender = true;',
			'const loaded = await Promise.resolve(value);',
			'loaded.toFixed();',
		].join('\n'),
		'<p>{loaded.toUpperCase()}</p>',
	);
	assert.deepEqual(codes(diagnostics), [[2551, 'toFixed']]);
});

const polymorphic = (tag: string, binding = 'as: Tag') =>
	[
		"import type { HTMLTag, Polymorphic } from 'astro/types';",
		`type Props<Tag extends ${tag}> = Polymorphic<{ as: Tag }>;`,
		`const { ${binding}, ...props } = Astro.props;`,
	].join('\n');

test('polymorphic components correlate their tag and attributes', () => {
	for (const [frontmatter, body] of [
		[polymorphic('HTMLTag'), '<Tag {...props} />'],
		[polymorphic("'a' | 'button'"), '<Tag {...props}><slot /></Tag>'],
		[polymorphic("'a' | 'button'", "as: Tag = 'button'"), '<Tag {...props}><slot /></Tag>'],
		[polymorphic('HTMLTag') + '\nconst xs = [1];', '{xs.map(() => <Tag {...props} />)}'],
	]) {
		const diagnostics = typecheckAstro(frontmatter, body, strictElements);
		assert.deepEqual(codes(diagnostics), [], `${frontmatter}\n${body}`);
	}

	const invalid = typecheckAstro(
		`${polymorphic("'a' | 'button'")}\nprops.nonexistent;`,
		'<Tag {...props} type="bogus" href={42} disabled={{}} />',
		strictElements,
	);
	assert.deepEqual(codes(invalid), [
		[2339, 'nonexistent'],
		[2322, 'type'],
		[2322, 'href'],
		[2322, 'disabled'],
	]);
});

test('only render-scope component tags are narrowed', () => {
	const nonGeneric = typecheckAstro(
		"interface Props { as?: 'a' }\nconst { as: Tag } = Astro.props;",
		'<Tag />',
		strictElements,
	);
	assert.deepEqual(codes(nonGeneric), [[2604, 'Tag']]);

	const notATag = typecheckAstro(
		'interface Props<T> { value: T }\nconst Tag = 42;',
		'<Tag />',
		strictElements,
	);
	assert.deepEqual(codes(notATag), [[2604, 'Tag']]);
});

test('polymorphic callers keep tag-specific attribute checks', () => {
	const source = `---\n${polymorphic("'a' | 'button'")}\n---\n<Tag {...props} />`;
	const { code } = convertToTsx(source, { ambientTypes: true });
	assert.deepEqual(
		typecheck(
			`${code}\nAstroComponent({ as: 'a', href: '/x' });\nAstroComponent({ as: 'button', type: 'submit' });`,
			strictElements,
		),
		[],
	);
	const diagnostics = typecheck(
		`${code}\nAstroComponent({ as: 'button', disabled: 'yes' });`,
		strictElements,
	);
	assert.deepEqual(
		diagnostics.map((d) => d.code),
		[2322],
		JSON.stringify(diagnostics),
	);
});

test('values queried by hoisted types stay declared at module scope', () => {
	const diagnostics = typecheckAstro(
		[
			"const sizes = ['sm', 'md'] as const;",
			'enum Variant { Plain }',
			'class Model { id = "" }',
			'interface Props<T> { value: T; size?: (typeof sizes)[number]; variant?: Variant; model?: Model }',
			"const { size = 'md', model } = Astro.props;",
			'size.toUpperCase();',
			'model?.id.toFixed();',
		].join('\n'),
	);
	assert.deepEqual(codes(diagnostics), [[2551, 'toFixed']]);
});

test('runtime exports cannot read render-scope values', () => {
	// Astro hoists exports out of the render function, so this throws at runtime.
	const diagnostics = typecheckAstro(
		'interface Props<T> { value: T }\nconst local = 1;\nexport const leaked = local;',
	);
	assert.deepEqual(codes(diagnostics), [[2304, 'local']]);
});

test('narrowed polymorphic tags still allow explicit undefined checks', () => {
	const diagnostics = typecheckAstro(
		polymorphic('HTMLTag'),
		'{Tag !== undefined ? <Tag {...props} /> : <div />}{Tag === undefined && <p />}',
		strictElements,
	);
	assert.deepEqual(codes(diagnostics), []);
});

test('member tags are not narrowed', () => {
	const diagnostics = typecheckAstro(
		'interface Props<T> { ns?: { Item: (props: { value?: T }) => any } }\nconst { ns } = Astro.props;',
		'<ns.Item />',
		strictElements,
	);
	assert.deepEqual(
		diagnostics.map((d) => d.code),
		[18048],
		JSON.stringify(diagnostics),
	);
});
