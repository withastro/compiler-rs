//! Node.js bindings for `astro2tsx`. Excluded from `cfg(test)` builds, where
//! napi-derive emits no registration and every item here looks dead.

use napi::bindgen_prelude::Uint32Array;
use napi_derive::napi;

use crate::{
    ConvertOptions, DEFAULT_SOURCE_NAME, ExtractedKind, SourceMapMode, convert_to_tsx as convert_rs,
};

/// Sentinel in the odd slots of `mappings`: that output has no original position.
#[napi]
pub const NO_ORIGINAL: u32 = u32::MAX;

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
    /// `"external"` omits the inline `//# sourceMappingURL=`; anything else appends it.
    pub sourcemap: Option<String>,
}

#[napi(object)]
pub struct ConvertToTsxResult {
    pub code: String,
    /// Flat `(generated, original)` byte-offset pairs, ascending by generated
    /// offset. Each pair opens a run: both offsets advance in lockstep until
    /// the next pair. An original of `0xFFFF_FFFF` opens an unmapped run.
    pub mappings: Uint32Array,
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
            sourcemap: match opts.sourcemap.as_deref() {
                Some("external") => SourceMapMode::External,
                _ => SourceMapMode::Inline,
            },
        },
    );
    let map = result.source_map(&source, &source_name).to_json_string();

    let mut mappings = Vec::with_capacity(result.mappings.len() * 2);
    for mapping in &result.mappings {
        mappings.push(mapping.generated);
        mappings.push(mapping.original.unwrap_or(NO_ORIGINAL));
    }

    ConvertToTsxResult {
        code: result.code,
        map,
        mappings: Uint32Array::new(mappings),
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
