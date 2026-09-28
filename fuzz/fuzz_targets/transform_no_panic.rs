#![no_main]

use astro_codegen::{TransformOptions, transform};
use libfuzzer_sys::fuzz_target;
use oxc_allocator::Allocator;
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;

fuzz_target!(|data: &str| {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, data, SourceType::astro())
        .with_options(ParseOptions::default())
        .parse_astro();
    // Compiler must never panic on any input, valid or not.
    let _ = transform(&allocator, data, TransformOptions::default(), &ret.root);
});
