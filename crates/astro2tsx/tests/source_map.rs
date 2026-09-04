//! Decodes maps independently with `oxc_sourcemap` to avoid self-validating the encoder.

mod common;

use std::fs;

use astro2tsx::{ConvertOptions, DEFAULT_SOURCE_NAME, SourceMapMode, convert_to_tsx};
use common::parse_fixture;
use oxc_sourcemap::SourceMap as DecodedMap;

fn fixtures() -> Vec<(String, String)> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let mut fixtures: Vec<(String, String)> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            if path.extension()? != "astro" {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            Some((name, fs::read_to_string(&path).ok()?))
        })
        .collect();
    fixtures.sort();
    fixtures
}

fn byte_offset(text: &str, line: u32, column: u32) -> Option<usize> {
    let mut matching_offsets = text
        .char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(text.len()))
        .filter(|&byte| line_col(text, byte) == (line, column));
    matching_offsets.next_back()
}

fn line_col(text: &str, byte: usize) -> (u32, u32) {
    let mut line = 0u32;
    let mut col = 0u32;
    let mut previous_was_cr = false;
    for (index, ch) in text.char_indices() {
        if index >= byte {
            break;
        }
        if ch == '\n' && previous_was_cr {
            previous_was_cr = false;
        } else if matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
            line += 1;
            col = 0;
            previous_was_cr = ch == '\r';
        } else {
            col += ch.len_utf16() as u32;
            previous_was_cr = false;
        }
    }
    (line, col)
}

fn resolve_decoded(decoded: &DecodedMap, line: u32, column: u32) -> Option<(u32, u32)> {
    let token = decoded
        .get_tokens()
        .filter(|token| token.get_dst_line() == line && token.get_dst_col() <= column)
        .max_by_key(|token| token.get_dst_col())?;
    token.get_source_id()?;
    Some((
        token.get_src_line(),
        token.get_src_col() + (column - token.get_dst_col()),
    ))
}

#[test]
fn every_fixture_round_trips_through_an_independent_decoder() {
    for (name, raw) in fixtures() {
        let (source, _) = parse_fixture(&raw);
        let result = convert_to_tsx(&source, ConvertOptions::default());
        let json = result
            .source_map(&source, DEFAULT_SOURCE_NAME)
            .to_json_string();

        let decoded = DecodedMap::from_json_string(&json)
            .unwrap_or_else(|error| panic!("{name}: map did not decode: {error}"));

        assert_eq!(
            decoded.get_source(0).map(|s| &**s),
            Some(DEFAULT_SOURCE_NAME),
            "{name}: sources[0] did not survive"
        );
        assert_eq!(
            decoded.get_source_content(0).map(|s| &**s),
            Some(source.as_str()),
            "{name}: sourcesContent did not survive"
        );

        for (index, mapping) in result.mappings.iter().enumerate() {
            let (line, column) = line_col(&result.code, mapping.generated as usize);
            match mapping.original {
                Some(original) => {
                    let resolved = resolve_decoded(&decoded, line, column).unwrap_or_else(|| {
                        panic!("{name}: run {index} at {line}:{column} does not resolve")
                    });
                    assert_eq!(
                        resolved,
                        line_col(&source, original as usize),
                        "{name}: run {index} resolves to the wrong original position"
                    );
                }
                None => {
                    let nil_token = decoded.get_tokens().any(|token| {
                        token.get_dst_line() == line
                            && token.get_dst_col() == column
                            && token.get_source_id().is_none()
                    });
                    assert!(
                        nil_token,
                        "{name}: nil run {index} at {line}:{column} has no explicit end marker"
                    );
                }
            }
        }

        for token in decoded.get_tokens() {
            let generated = byte_offset(&result.code, token.get_dst_line(), token.get_dst_col())
                .unwrap_or_else(|| {
                    panic!(
                        "{name}: token at {}:{} is off the end of the output",
                        token.get_dst_line(),
                        token.get_dst_col()
                    )
                }) as u32;
            let run = result
                .mappings
                .iter()
                .rev()
                .find(|mapping| mapping.generated <= generated)
                .unwrap_or_else(|| panic!("{name}: token at byte {generated} precedes all runs"));
            match run.original {
                Some(original) => {
                    let source_byte =
                        byte_offset(&source, token.get_src_line(), token.get_src_col())
                            .unwrap_or_else(|| {
                                panic!(
                                    "{name}: token source {}:{} is off the end of the input",
                                    token.get_src_line(),
                                    token.get_src_col()
                                )
                            }) as u32;
                    assert_eq!(
                        source_byte,
                        original + (generated - run.generated),
                        "{name}: token at byte {generated} breaks its run's lockstep"
                    );
                }
                None => {
                    assert!(
                        token.get_source_id().is_none(),
                        "{name}: token inside a nil run carries a source"
                    );
                    assert_eq!(
                        generated, run.generated,
                        "{name}: nil runs must not synthesize extra tokens"
                    );
                }
            }
        }
    }
}

#[test]
fn external_mode_omits_the_inline_comment() {
    let source = "---\nlet x = 1;\n---\n<p>Hi</p>\n";
    const COMMENT: &str = "\n//# sourceMappingURL=data:application/json;charset=utf-8;base64,";

    let external = convert_to_tsx(
        source,
        ConvertOptions {
            sourcemap: SourceMapMode::External,
            ..Default::default()
        },
    );
    assert!(!external.code.contains(COMMENT));

    let inline = convert_to_tsx(source, ConvertOptions::default());
    assert!(inline.code.contains(COMMENT));
    assert_eq!(
        inline.code.split_once(COMMENT).unwrap().0,
        external.code,
        "the two modes should agree on everything before the comment"
    );
    assert_eq!(inline.mappings, external.mappings);
}

// The expected column distinguishes UTF-16 from byte and scalar-value columns.
#[test]
fn astral_columns_are_utf16_code_units() {
    let source = "---\nconst \u{3c0} = Math.PI;\n---\n<p>\u{1f984} {\u{3c0}}</p>\n";
    let result = convert_to_tsx(source, ConvertOptions::default());
    let json = result
        .source_map(source, DEFAULT_SOURCE_NAME)
        .to_json_string();
    let decoded = DecodedMap::from_json_string(&json).unwrap();

    let resolve_generated_snippet = |snippet: &str| {
        let generated = result
            .code
            .find(snippet)
            .unwrap_or_else(|| panic!("{snippet:?} not in the output"));
        let (line, column) = line_col(&result.code, generated);
        resolve_decoded(&decoded, line, column)
            .unwrap_or_else(|| panic!("{snippet:?} does not resolve"))
    };

    assert_eq!(resolve_generated_snippet("{\u{3c0}}"), (3, 6));
    assert_eq!(resolve_generated_snippet("\u{3c0} = Math.PI"), (1, 6));
    assert_eq!(resolve_generated_snippet("</p>"), (3, 9));
}

#[test]
fn every_javascript_line_terminator_resets_source_and_generated_columns() {
    for terminator in ["\n", "\r\n", "\r", "\u{2028}", "\u{2029}"] {
        let source = format!("<div>🦄x{terminator}<span>target</span></div>");
        let result = convert_to_tsx(&source, ConvertOptions::default());
        let decoded = DecodedMap::from_json_string(
            &result
                .source_map(&source, DEFAULT_SOURCE_NAME)
                .to_json_string(),
        )
        .unwrap();
        let generated = result.code.find("target").unwrap();
        let (line, column) = line_col(&result.code, generated);

        assert_eq!(
            resolve_decoded(&decoded, line, column),
            Some((1, 6)),
            "wrong mapping after {terminator:?}"
        );
    }
}
