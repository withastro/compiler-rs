//! Node.js bindings for `astro2tsx`. Excluded from `cfg(test)` builds, where
//! napi-derive emits no registration and every item here looks dead.

use napi_derive::napi;

use crate::{ConvertOptions, DEFAULT_SOURCE_NAME, ExtractedKind, convert_to_tsx as convert_rs};

/// Per-byte source-position mapping between the emitted TSX and the source.
#[napi(object)]
pub struct Mapping {
    /// Byte offset into the generated TSX.
    pub generated: u32,
    /// Byte offset into the original `.astro` source. `None` for synthetic
    /// content (e.g. the `<Fragment>` wrapping or the prefix comment).
    pub original: Option<u32>,
}

/// Byte range inside the generated TSX.
#[napi(object)]
pub struct GeneratedRange {
    pub start: u32,
    pub end: u32,
}

#[napi(string_enum)]
pub enum ExtractedTagKind {
    Script,
    Style,
    StyleAttribute,
    EventAttribute,
}

#[napi(object)]
pub struct ExtractedTag {
    pub range: GeneratedRange,
    pub kind: ExtractedTagKind,
    pub content: String,
    pub lang: Option<String>,
}

#[napi(object)]
#[derive(Default)]
pub struct ConvertToTsxOptions {
    /// Filename used to derive the default-exported component identifier
    /// (e.g. `MyPage.astro` produces `MyPage__AstroComponent_`). Optional.
    pub filename: Option<String>,
}

#[napi(object)]
pub struct ConvertToTsxResult {
    pub code: String,
    pub mappings: Vec<Mapping>,
    /// Source Map v3 JSON for `code`, with `sourcesContent` embedded.
    pub map: String,
    pub frontmatter: GeneratedRange,
    pub body: GeneratedRange,
    pub scripts: Vec<ExtractedTag>,
    pub styles: Vec<ExtractedTag>,
    pub has_parse_errors: bool,
}

/// Convert an Astro source file to TSX for tsserver intellisense.
///
/// The conversion is error-tolerant: malformed input produces a
/// best-effort TSX output rather than throwing, and `hasParseErrors` is
/// set to `true` when the parser surfaced one or more diagnostics.
#[napi(js_name = "convertToTsx")]
pub fn convert_to_tsx(source: String, options: Option<ConvertToTsxOptions>) -> ConvertToTsxResult {
    let opts = options.unwrap_or_default();
    let source_name = opts
        .filename
        .clone()
        .unwrap_or_else(|| DEFAULT_SOURCE_NAME.to_string());
    let result = convert_rs(
        &source,
        ConvertOptions {
            filename: opts.filename,
        },
    );
    let map = result.source_map(&source, &source_name).to_json();

    ConvertToTsxResult {
        code: result.code,
        map,
        mappings: result
            .mappings
            .into_iter()
            .map(|m| Mapping {
                generated: m.generated,
                original: m.original,
            })
            .collect(),
        frontmatter: GeneratedRange {
            start: result.frontmatter.start,
            end: result.frontmatter.end,
        },
        body: GeneratedRange {
            start: result.body.start,
            end: result.body.end,
        },
        scripts: result
            .scripts
            .into_iter()
            .map(extracted_tag_to_napi)
            .collect(),
        styles: result
            .styles
            .into_iter()
            .map(extracted_tag_to_napi)
            .collect(),
        has_parse_errors: result.has_parse_errors,
    }
}

fn extracted_tag_to_napi(tag: crate::ExtractedTag) -> ExtractedTag {
    ExtractedTag {
        range: GeneratedRange {
            start: tag.range.start,
            end: tag.range.end,
        },
        kind: match tag.kind {
            ExtractedKind::Script => ExtractedTagKind::Script,
            ExtractedKind::Style => ExtractedTagKind::Style,
            ExtractedKind::StyleAttribute => ExtractedTagKind::StyleAttribute,
            ExtractedKind::EventAttribute => ExtractedTagKind::EventAttribute,
        },
        content: tag.content,
        lang: tag.lang,
    }
}
