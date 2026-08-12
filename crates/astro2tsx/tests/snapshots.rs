//! astro2tsx snapshot tests. Run `cargo insta review` to accept changes.
//!
//! Each snapshot captures the whole `ConvertResult`, not just its code.
//!
//! A fixture may open with `// @config key=value` lines, stripped before
//! conversion. The only key is `filename`, which defaults to none.

use std::fmt::Write as _;
use std::fs;

use astro2tsx::{
    ConvertOptions, ConvertResult, DEFAULT_SOURCE_NAME, ExtractedTag, Mapping, convert_to_tsx,
};

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

/// Mappings are per character, so lockstep ones collapse into a run.
struct Run {
    generated_start: u32,
    generated_end: u32,
    original: Option<u32>,
    points: usize,
}

fn runs(mappings: &[Mapping], code_len: u32) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (index, mapping) in mappings.iter().enumerate() {
        let generated_end = mappings
            .get(index + 1)
            .map_or(code_len, |next| next.generated);
        if let Some(open) = runs.last_mut() {
            let previous = &mappings[index - 1];
            let lockstep = match (previous.original, mapping.original) {
                (None, None) => true,
                (Some(before), Some(after)) => {
                    i64::from(mapping.generated) - i64::from(previous.generated)
                        == i64::from(after) - i64::from(before)
                }
                _ => false,
            };
            if lockstep && mapping.generated >= previous.generated {
                open.generated_end = generated_end;
                open.points += 1;
                continue;
            }
        }
        runs.push(Run {
            generated_start: mapping.generated,
            generated_end,
            original: mapping.original,
            points: 1,
        });
    }
    runs
}

fn write_tags(out: &mut String, label: &str, tags: &[ExtractedTag]) {
    if tags.is_empty() {
        let _ = writeln!(out, "{label}: (none)");
        return;
    }
    let _ = writeln!(out, "{label}:");
    for tag in tags {
        let _ = writeln!(
            out,
            "  {:?} lang={:?} generated {}..{} content={:?}",
            tag.kind, tag.lang, tag.range.start, tag.range.end, tag.content
        );
    }
}

fn report(source: &str, source_name: &str, result: &ConvertResult) -> String {
    let mut out = String::new();
    let map = result.source_map(source, source_name);

    out.push_str("--- code, with an inline source map this harness appends ---\n");
    out.push_str(&result.code);
    if !result.code.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&map.to_inline_comment());
    out.push('\n');

    out.push_str("\n--- result ---\n");
    let _ = writeln!(out, "has_parse_errors: {}", result.has_parse_errors);
    let _ = writeln!(
        out,
        "frontmatter: {}..{}",
        result.frontmatter.start, result.frontmatter.end
    );
    let _ = writeln!(out, "body: {}..{}", result.body.start, result.body.end);
    write_tags(&mut out, "scripts", &result.scripts);
    write_tags(&mut out, "styles", &result.styles);

    let runs = runs(&result.mappings, result.code.len() as u32);
    let _ = writeln!(
        out,
        "\n--- mappings: {} points in {} runs ---",
        result.mappings.len(),
        runs.len()
    );
    out.push_str("generated_start..generated_end xpoints -> original; `-` is unmapped\n");
    for run in &runs {
        let original = match run.original {
            Some(offset) => offset.to_string(),
            None => "-".to_string(),
        };
        let _ = writeln!(
            out,
            "{}..{} x{} -> {}",
            run.generated_start, run.generated_end, run.points, original
        );
    }

    out
}

#[test]
fn snapshots() {
    insta::glob!("fixtures/*.astro", |path| {
        let raw = fs::read_to_string(path).unwrap();
        let name = path.file_stem().unwrap().to_str().unwrap();
        let (source, options) = parse_fixture(&raw);
        let source_name = options
            .filename
            .clone()
            .unwrap_or_else(|| DEFAULT_SOURCE_NAME.to_string());
        let output = report(&source, &source_name, &convert_to_tsx(&source, options));

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

/// insta rewrites CRLF to LF when storing a snapshot, hiding this there.
#[test]
fn crlf_fixture_keeps_its_line_endings() {
    let raw = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/crlf_frontmatter.astro"
    ))
    .unwrap();
    let (source, options) = parse_fixture(&raw);
    let result = convert_to_tsx(&source, options);

    assert_eq!(
        source.matches('\n').count(),
        source.matches("\r\n").count(),
        "fixture is no longer CRLF-only, so this test proves nothing"
    );
    for line in source.split("\r\n") {
        if line.is_empty() || line == "---" {
            continue;
        }
        assert!(
            result.code.contains(&format!("{line}\r\n")),
            "{line:?} lost its CRLF:\n{:?}",
            result.code
        );
    }
}
