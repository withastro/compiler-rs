import { defineConfig } from 'tsdown';

export default defineConfig((inlineConfig) => ({
	entry: ['src/**'],
	outDir: 'dist',
	format: ['esm'],
	dts: true,
	clean: true,
	minify: !inlineConfig.watch,
	sourcemap: Boolean(inlineConfig.watch),
	watch: inlineConfig.watch,
	shims: true,
	deps: {
		neverBundle: ['@astrojs/compiler-binding'],
	},
}));
