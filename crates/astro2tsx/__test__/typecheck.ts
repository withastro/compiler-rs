import { strict as assert } from 'node:assert';
import { join } from 'node:path';
import ts from 'typescript';
import { convertToTsx } from '../index.js';

const ambient = `
declare module 'astro' {
  export type AstroGlobal<Props, Component, Params = unknown> = { props: Props; params: Params };
}
declare module 'astro/jsx-runtime' {
  export namespace JSX { interface IntrinsicElements { [name: string]: any } }
}
`;

export function typecheck(code: string) {
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

export function typecheckAstro(frontmatter: string, body = '') {
	const result = convertToTsx(`---\n${frontmatter}\n---\n${body}`, { ambientTypes: true });
	assert.equal(result.hasParseErrors, false, JSON.stringify(result.diagnostics));
	return typecheck(result.code);
}
