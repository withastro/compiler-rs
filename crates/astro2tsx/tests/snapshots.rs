//! astro2tsx snapshot tests. Run `cargo insta review` to accept changes.
//!
//! The body is only the TSX and its inline source map, so it pastes straight
//! into a viewer; the rest of the result rides in insta's `info` header.
//!
//! A fixture may open with `// @config key=value` lines, stripped before
//! conversion. The only key is `filename`, which defaults to none.

use std::fs;

use astro2tsx::{ConvertOptions, ConvertResult, DEFAULT_SOURCE_NAME, ExtractedTag, convert_to_tsx};
use serde::Serialize;

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

#[derive(Serialize)]
struct Info {
    has_parse_errors: bool,
    frontmatter: Range,
    body: Range,
    scripts: Vec<Tag>,
    styles: Vec<Tag>,
}

#[derive(Serialize)]
struct Range {
    start: u32,
    end: u32,
}

#[derive(Serialize)]
struct Tag {
    kind: String,
    lang: Option<String>,
    generated: Range,
    content: String,
}

fn tags(tags: &[ExtractedTag]) -> Vec<Tag> {
    tags.iter()
        .map(|tag| Tag {
            kind: format!("{:?}", tag.kind),
            lang: tag.lang.clone(),
            generated: Range {
                start: tag.range.start,
                end: tag.range.end,
            },
            content: tag.content.clone(),
        })
        .collect()
}

fn info(result: &ConvertResult) -> Info {
    Info {
        has_parse_errors: result.has_parse_errors,
        frontmatter: Range {
            start: result.frontmatter.start,
            end: result.frontmatter.end,
        },
        body: Range {
            start: result.body.start,
            end: result.body.end,
        },
        scripts: tags(&result.scripts),
        styles: tags(&result.styles),
    }
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
        let result = convert_to_tsx(&source, options);
        let map = result.source_map(&source, &source_name);

        // A leading blank line would shift every generated line by one.
        let mut body = String::new();
        body.push_str(&result.code);
        if !result.code.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&map.to_inline_comment());

        insta::with_settings!({
            snapshot_path => path.parent().unwrap(),
            prepend_module_to_snapshot => false,
            snapshot_suffix => "",
            omit_expression => true,
            info => &info(&result),
        }, {
            insta::assert_snapshot!(name, body);
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
