//! Astro codegen snapshot tests.
//!
//! Uses `insta::glob!` to discover `.astro` fixture files and compare
//! codegen output against co-located `.snap` snapshot files.
//!
//! ## Per-fixture configuration
//!
//! A fixture file may start with one or more `// @config key=value` lines to
//! configure the compiler options used for that fixture.  These lines are
//! stripped before the source is handed to the compiler.
//!
//! Supported keys:
//!
//! | key       | values                  | default    |
//! |-----------|-------------------------|------------|
//! | `compact` | `html`, `jsx`, `false`  | `false`    |

use std::fs;

use astro_codegen::{CompactMode, TransformOptions, transform};
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

/// Parse `// @config` directives from the top of a fixture file.
///
/// Returns `(source_without_directives, options)`.
fn parse_fixture(raw: &str) -> (String, TransformOptions) {
    let mut options = TransformOptions::new()
        .with_internal_url("http://localhost:3000/")
        .with_astro_global_args("\"https://astro.build\"");

    let mut remaining = raw;
    loop {
        let line = remaining.lines().next().unwrap_or("");
        let Some(config) = line.strip_prefix("// @config ") else {
            break;
        };
        if let Some(value) = config.strip_prefix("compact=") {
            options.compact = match value.trim() {
                "html" => CompactMode::Html,
                "jsx" => CompactMode::Jsx,
                _ => CompactMode::Disabled,
            };
        }
        // Advance past this line (including the newline)
        remaining = remaining[line.len()..].trim_start_matches('\n');
    }

    (remaining.to_string(), options)
}

fn compile_astro(source: &str, options: TransformOptions) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::astro();

    let ret = Parser::new(&allocator, source, source_type).parse_astro();

    if !ret.errors.is_empty() {
        let errors: Vec<String> = ret.errors.iter().map(|e| format!("{e}")).collect();
        return format!("Parse errors:\n{}", errors.join("\n"));
    }

    transform(&allocator, source, options, &ret.root).code
}

#[test]
fn snapshots() {
    insta::glob!("fixtures/*.astro", |path| {
        let raw = fs::read_to_string(path).unwrap();
        let name = path.file_stem().unwrap().to_str().unwrap();
        let (source, options) = parse_fixture(&raw);
        let output = compile_astro(&source, options);

        insta::with_settings!({
            snapshot_path => path.parent().unwrap(),
            prepend_module_to_snapshot => false,
            snapshot_suffix => "",
            omit_expression => true,
        }, {
            insta::assert_snapshot!(name, output);
        });
    });
}
