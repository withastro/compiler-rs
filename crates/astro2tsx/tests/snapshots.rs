mod common;

use std::fs;

use astro2tsx::{ConvertResult, ExtractedTag, convert_to_tsx};
use common::parse_fixture;
use serde::Serialize;

#[derive(Serialize)]
struct Info {
    has_parse_errors: bool,
    frontmatter_status: String,
    frontmatter_source: Range,
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
    source: Range,
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
            source: Range {
                start: tag.source.start,
                end: tag.source.end,
            },
            content: tag.content.clone(),
        })
        .collect()
}

fn info(result: &ConvertResult) -> Info {
    Info {
        has_parse_errors: result.has_parse_errors,
        frontmatter_status: format!("{:?}", result.frontmatter.status),
        frontmatter_source: Range {
            start: result.frontmatter.source.start,
            end: result.frontmatter.source.end,
        },
        frontmatter: Range {
            start: result.frontmatter_range.start,
            end: result.frontmatter_range.end,
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
        let result = convert_to_tsx(&source, options);

        insta::with_settings!({
            snapshot_path => path.parent().unwrap(),
            prepend_module_to_snapshot => false,
            snapshot_suffix => "",
            omit_expression => true,
            info => &info(&result),
        }, {
            insta::assert_snapshot!(name, result.code);
        });
    });
}

// Insta normalizes CRLF in snapshots, so this test reads the fixture directly.
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
