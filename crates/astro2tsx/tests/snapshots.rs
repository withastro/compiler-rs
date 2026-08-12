//! astro2tsx snapshot tests. Run `cargo insta review` to accept changes.
//!
//! A fixture may open with `// @config key=value` lines, stripped before
//! conversion. The only key is `filename`, which defaults to none.

use std::fs;

use astro2tsx::{ConvertOptions, convert_to_tsx};

/// Parse `// @config` directives from the top of a fixture file.
///
/// Returns `(source_without_directives, options)`.
fn parse_fixture(raw: &str) -> (String, ConvertOptions) {
    let mut options = ConvertOptions::default();

    let mut remaining = raw;
    loop {
        let line = remaining.lines().next().unwrap_or("");
        let Some(config) = line.strip_prefix("// @config ") else {
            break;
        };
        if let Some(value) = config.strip_prefix("filename=") {
            options.filename = Some(value.trim().to_string());
        }
        remaining = remaining[line.len()..].trim_start_matches('\n');
    }

    (remaining.to_string(), options)
}

#[test]
fn snapshots() {
    insta::glob!("fixtures/*.astro", |path| {
        let raw = fs::read_to_string(path).unwrap();
        let name = path.file_stem().unwrap().to_str().unwrap();
        let (source, options) = parse_fixture(&raw);
        let output = convert_to_tsx(&source, options).code;

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
