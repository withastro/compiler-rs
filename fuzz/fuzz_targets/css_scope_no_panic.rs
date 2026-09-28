#![no_main]

use astro_codegen::ScopedStyleStrategy;
use astro_codegen::css_scoping::scope_css;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // CSS scoper must never panic on any input.
    let _ = scope_css(data, "astro-xxxx", ScopedStyleStrategy::Where);
});
