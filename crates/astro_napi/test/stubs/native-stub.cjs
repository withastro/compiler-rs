'use strict';

// Minimal native binding stub used by loader tests to verify that the native
// addon is loaded only when NAPI_RS_FORCE_WASI does not force WASI.
module.exports = {
	compileAstroSync() {
		return 'native-stub-loaded';
	},
};
