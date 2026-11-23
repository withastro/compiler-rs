import { strict as assert } from 'node:assert';
import { join } from 'node:path';
import ts from 'typescript';
import { convertToTsx } from '../index.js';

const astroGlobal = `
declare module 'astro' {
  export type AstroGlobal<Props, Component, Params = unknown> = {
    props: Props;
    params: Params;
    redirect(path: string): Response;
  };
}
`;

const looseElements = `
declare module 'astro/jsx-runtime' {
  export namespace JSX { interface IntrinsicElements { [name: string]: any } }
}
`;

export const strictElements = `
declare module 'astro/jsx-runtime' {
  export namespace JSX {
    interface DefinedIntrinsicElements {
      a: { href?: string | null; type?: string | null; children?: any };
      button: { type?: 'button' | 'submit' | 'reset' | null; disabled?: boolean | null; children?: any };
      slot: { children?: any };
    }
    interface IntrinsicElements extends DefinedIntrinsicElements {
      [name: string]: { [name: string]: any };
    }
  }
}
declare module 'astro/types' {
  type Defined = import('astro/jsx-runtime').JSX.DefinedIntrinsicElements;
  type OmitIndexSignature<T> = {
    [K in keyof T as object extends Record<K, unknown> ? never : K]: T[K];
  };
  export type HTMLTag = keyof Defined;
  type PolymorphicAttributes<P extends { as: HTMLTag }> = Omit<P, 'as'> & {
    as?: P['as'];
  } & Omit<OmitIndexSignature<Defined[P['as']]>, never>;
  export type Polymorphic<P extends { as: HTMLTag }> = PolymorphicAttributes<
    Omit<P, 'as'> & { as: NonNullable<P['as']> }
  >;
}
`;

export function typecheck(code: string, elements = looseElements) {
	const ambient = astroGlobal + elements;
	const filename = join(import.meta.dirname, 'consumer.tsx');
	const ambientFilename = join(import.meta.dirname, 'ambient.d.ts');
	const files = new Map([
		[filename, code],
		[ambientFilename, ambient],
	]);
	const options: ts.CompilerOptions = {
		noEmit: true,
		strict: true,
		isolatedModules: true,
		target: ts.ScriptTarget.ESNext,
		module: ts.ModuleKind.ESNext,
		moduleResolution: ts.ModuleResolutionKind.Bundler,
		jsx: ts.JsxEmit.Preserve,
		types: [],
	};
	const host = ts.createCompilerHost(options);
	const fileExists = host.fileExists;
	host.fileExists = (name) => files.has(name) || fileExists(name);
	const getSourceFile = host.getSourceFile;
	host.getSourceFile = (name, languageVersion, onError, shouldCreateNewSourceFile) => {
		const text = files.get(name);
		return text === undefined
			? getSourceFile(name, languageVersion, onError, shouldCreateNewSourceFile)
			: ts.createSourceFile(name, text, languageVersion, true);
	};
	const program = ts.createProgram([...files.keys()], options, host);
	return ts.getPreEmitDiagnostics(program).map((diagnostic) => ({
		code: diagnostic.code,
		message: ts.flattenDiagnosticMessageText(diagnostic.messageText, '\n'),
		text: diagnostic.file?.text.slice(
			diagnostic.start,
			(diagnostic.start ?? 0) + (diagnostic.length ?? 0),
		),
	}));
}

export function typecheckAstro(frontmatter: string, body = '', elements?: string) {
	const result = convertToTsx(`---\n${frontmatter}\n---\n${body}`, { ambientTypes: true });
	assert.equal(result.hasParseErrors, false, JSON.stringify(result.diagnostics));
	return typecheck(result.code, elements);
}
