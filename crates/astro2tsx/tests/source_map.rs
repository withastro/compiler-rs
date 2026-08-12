//! Checks the encoded source map by decoding it with `oxc_sourcemap` rather
//! than by re-deriving it from the same code that produced it.

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

/// Byte offset of a zero-based line and UTF-16 column, resolved through
/// `str::encode_utf16` so it shares no code with the encoder.
fn byte_offset(text: &str, line: u32, column: u32) -> Option<usize> {
    let mut line_start = 0usize;
    for (index, line_text) in text.split('\n').enumerate() {
        if index as u32 == line {
            let boundaries = line_text
                .char_indices()
                .map(|(byte, _)| byte)
                .chain(std::iter::once(line_text.len()));
            for byte in boundaries {
                if line_text[..byte].encode_utf16().count() as u32 == column {
                    return Some(line_start + byte);
                }
            }
            return None;
        }
        line_start += line_text.len() + 1;
    }
    None
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

        let tokens: Vec<_> = decoded.get_tokens().collect();
        assert_eq!(
            tokens.len(),
            result.mappings.len(),
            "{name}: decoded {} tokens for {} mappings",
            tokens.len(),
            result.mappings.len()
        );

        for (index, (token, mapping)) in tokens.iter().zip(&result.mappings).enumerate() {
            let generated = byte_offset(&result.code, token.get_dst_line(), token.get_dst_col())
                .unwrap_or_else(|| {
                    panic!(
                        "{name}: mapping {index} decoded to generated {}:{}, which is off the end",
                        token.get_dst_line(),
                        token.get_dst_col()
                    )
                });
            assert_eq!(
                generated as u32, mapping.generated,
                "{name}: mapping {index} generated offset changed across the round trip"
            );

            match mapping.original {
                Some(original) => {
                    assert!(
                        token.get_source_id().is_some(),
                        "{name}: mapping {index} lost its source"
                    );
                    let decoded_original = byte_offset(
                        &source,
                        token.get_src_line(),
                        token.get_src_col(),
                    )
                    .unwrap_or_else(|| {
                        panic!(
                            "{name}: mapping {index} decoded to source {}:{}, which is off the end",
                            token.get_src_line(),
                            token.get_src_col()
                        )
                    });
                    assert_eq!(
                        decoded_original as u32, original,
                        "{name}: mapping {index} original offset changed across the round trip"
                    );
                }
                None => assert!(
                    token.get_source_id().is_none(),
                    "{name}: mapping {index} gained a source it never had"
                ),
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

/// A byte column would be 8 here and a scalar-value column 5, so this pins the
/// units down to UTF-16 rather than merely to "not bytes".
#[test]
fn astral_columns_are_utf16_code_units() {
    let source = "---\nconst \u{3c0} = Math.PI;\n---\n<p>\u{1f984} {\u{3c0}}</p>\n";
    let result = convert_to_tsx(source, ConvertOptions::default());
    let json = result
        .source_map(source, DEFAULT_SOURCE_NAME)
        .to_json_string();
    let decoded = DecodedMap::from_json_string(&json).unwrap();

    let tokens: Vec<_> = decoded.get_tokens().collect();

    // Selecting by raw byte offset keeps the assertion off the decoded columns.
    let column_of = |offset: usize| {
        let index = result
            .mappings
            .iter()
            .position(|mapping| mapping.original == Some(offset as u32))
            .unwrap_or_else(|| panic!("byte {offset} is not mapped"));
        let token = tokens[index];
        (token.get_src_line(), token.get_src_col())
    };

    assert_eq!(column_of(source.find("{\u{3c0}}").unwrap()), (3, 6));
    assert_eq!(column_of(source.find("\u{3c0} = Math.PI").unwrap()), (1, 6));
    assert_eq!(column_of(source.find("</p>").unwrap()), (3, 9));
}
