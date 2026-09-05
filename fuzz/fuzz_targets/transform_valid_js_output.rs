#![no_main]

use astro_codegen::{TransformOptions, transform};
use libfuzzer_sys::{Corpus, fuzz_target};
use oxc_allocator::Allocator;
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;

/// For any Astro input that parses without errors, the compiler's JavaScript output
/// must itself be valid JavaScript. If it isn't, the compiler has a bug.
///
/// This is analogous to the ezno parser's roundtrip test: if parsing succeeds,
/// the printed form must also parse successfully.
fn do_fuzz(data: &str) -> Corpus {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, data, SourceType::astro())
        .with_options(ParseOptions::default())
        .parse_astro();

    // If the Astro input isn't valid, discard it from the corpus.
    if !ret.errors.is_empty() {
        return Corpus::Reject;
    }

    let result = transform(&allocator, data, TransformOptions::default(), &ret.root);

    // The JS output must parse without errors.
    let js_allocator = Allocator::default();
    let js_ret = Parser::new(&js_allocator, &result.code, SourceType::mjs())
        .with_options(ParseOptions::default())
        .parse();

    if !js_ret.errors.is_empty() {
        panic!(
            "compiler produced invalid JS for valid Astro input\n\
             input:  {data:?}\n\
             output: {:?}\n\
             errors: {:?}",
            result.code, js_ret.errors,
        );
    }

    Corpus::Keep
}

fuzz_target!(|data: &str| -> Corpus { do_fuzz(data) });
