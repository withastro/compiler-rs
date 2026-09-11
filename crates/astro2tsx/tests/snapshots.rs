use std::fs;

use astro2tsx::{ConvertOptions, ConvertResult, ExtractedTag, convert_to_tsx};
use biome_js_parser::{JsParserOptions, parse};
use biome_languages::JsFileSource;
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

fn assert_mapped_runs_are_verbatim(source: &str, result: &ConvertResult, label: &str) {
    let code_len = result.code.len() as u32;
    let mut previous_generated = 0;
    for (index, mapping) in result.mappings.iter().enumerate() {
        assert!(
            mapping.generated >= previous_generated,
            "{label}: run {index} goes backwards"
        );
        previous_generated = mapping.generated;
        let Some(original) = mapping.original else {
            continue;
        };
        let run_end = result
            .mappings
            .get(index + 1)
            .map(|next| next.generated)
            .unwrap_or(code_len);
        let length = (run_end - mapping.generated).min(source.len() as u32 - original);
        let generated_slice =
            &result.code[mapping.generated as usize..(mapping.generated + length) as usize];
        let source_slice = &source[original as usize..(original + length) as usize];
        assert_eq!(
            generated_slice, source_slice,
            "{label}: run {index} is not verbatim"
        );
    }
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

#[test]
fn every_fixture_keeps_mapped_runs_verbatim() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let mut checked = 0;
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "astro") {
            continue;
        }
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let raw = fs::read_to_string(&path).unwrap();
        let (source, options) = parse_fixture(&raw);
        let result = convert_to_tsx(&source, options);
        assert_mapped_runs_are_verbatim(&source, &result, &name);
        if !result.has_parse_errors {
            let parsed = parse(
                &result.code,
                JsFileSource::tsx(),
                JsParserOptions::default(),
            );
            assert!(
                parsed.diagnostics().is_empty(),
                "generated invalid TSX for {name:?}:\n{:?}\n{}",
                parsed.diagnostics(),
                result.code
            );
        }
        checked += 1;
    }
    assert!(
        checked > 50,
        "expected to check most fixtures, got {checked}"
    );
}

#[test]
fn stripping_the_doctype_keeps_mapped_runs_verbatim() {
    let source = "---\nconst é = 1;\n---\n\n<!doctype html>\n<html lang=en data-x=\"𝒳\"><body>{é}</body></html>\n";
    for ambient_types in [false, true] {
        let result = convert_to_tsx(
            source,
            ConvertOptions {
                ambient_types,
                ..Default::default()
            },
        );
        assert!(!result.code.contains("<!"), "{}", result.code);
        assert_mapped_runs_are_verbatim(source, &result, "doctype");
    }
}
