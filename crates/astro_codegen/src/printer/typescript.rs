//! TypeScript stripping pass.
//!
//! Parses a TypeScript string, runs `oxc_transformer` (TS-only stripping,
//! no JSX transform, no ES downleveling), and re-emits as plain JavaScript.
//!
//! This is intentionally independent of sourcemap concerns: you can strip
//! TypeScript with or without a sourcemap.  The sourcemap composition that
//! chains this pass's output map back to the original `.astro` source lives
//! in [`super::sourcemap_builder`].

use oxc_allocator::Allocator;
use oxc_codegen::CodegenOptions;

/// Strip TypeScript syntax from `code`.
///
/// Returns `(js_code, Option<SourceMap>)`.  The sourcemap is produced only when
/// `generate_sourcemap` is `true`; it maps positions in the returned JavaScript
/// back to positions in the input `code`.
///
/// If parsing fails the input is returned unchanged (with `None` for the map)
/// so that the downstream consumer can report a better error.
pub fn strip_typescript(
    allocator: &Allocator,
    code: &str,
    generate_sourcemap: bool,
) -> (String, Option<oxc_sourcemap::SourceMap>) {
    let source_type = oxc_span::SourceType::mjs().with_typescript(true);
    let ret = oxc_parser::Parser::new(allocator, code, source_type).parse();

    if !ret.errors.is_empty() {
        // If parsing fails, return the code unchanged — the downstream
        // consumer will report a better error.
        return (code.to_string(), None);
    }

    let mut program = ret.program;
    let scoping = oxc_semantic::SemanticBuilder::new()
        .with_excess_capacity(2.0)
        .build(&program)
        .semantic
        .into_scoping();

    let options = oxc_transformer::TransformOptions::default();
    let _ = oxc_transformer::Transformer::new(allocator, std::path::Path::new(""), &options)
        .build_with_scoping(scoping, &mut program);

    let codegen_options = CodegenOptions {
        single_quote: false,
        source_map_path: if generate_sourcemap {
            Some(std::path::PathBuf::from("intermediate.js"))
        } else {
            None
        },
        ..CodegenOptions::default()
    };
    let result = oxc_codegen::Codegen::new()
        .with_options(codegen_options)
        .build(&program);
    (result.code, result.map)
}
