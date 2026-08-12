//! Node.js bindings for `astro2tsx`. Excluded from `cfg(test)` builds, where
//! napi-derive emits no registration and every item here looks dead.
//!
//! Every offset crossing this boundary is in UTF-16 code units, because JS
//! consumers index strings that way.

use napi::bindgen_prelude::Uint32Array;
use napi_derive::napi;

use crate::utf16::Utf16Index;
use crate::{
    ConvertOptions, DEFAULT_SOURCE_NAME, DiagnosticSeverity, ExtractedKind, FrontmatterStatus,
    SourceMapMode, convert_to_tsx as convert_rs,
};

#[napi(object)]
pub struct Range {
    pub start: u32,
    pub end: u32,
}

#[napi(string_enum = "kebab-case")]
pub enum ExtractedTagKind {
    Script,
    Style,
    StyleAttribute,
    EventAttribute,
}

#[napi(object)]
pub struct ExtractedTag {
    /// Range of `content` in the original source.
    pub position: Range,
    pub kind: ExtractedTagKind,
    pub content: String,
    /// Script family (`module`, `json`, …) or style language (`css`, `scss`, …).
    pub lang: Option<String>,
}

#[napi(string_enum = "kebab-case")]
pub enum AstroFrontmatterStatus {
    DoesntExist,
    Open,
    Closed,
}

#[napi(object)]
pub struct AstroDiagnostic {
    pub message: String,
    /// 1 = error, 2 = warning, 3 = information, 4 = hint.
    pub severity: u32,
    pub position: Range,
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
    /// Offsets into `code` where each mapped run starts, ascending. Positions
    /// inside a run resolve as `sourceOffsets[i] + (offset -
    /// generatedOffsets[i])` while within `lengths[i]` — the shape Volar
    /// mappings use. Output outside every run is synthetic.
    pub generated_offsets: Uint32Array,
    pub source_offsets: Uint32Array,
    pub lengths: Uint32Array,
    /// Source Map v3 JSON for `code`, with `sourcesContent` embedded.
    pub map: String,
    /// Range of the frontmatter section within `code`.
    pub frontmatter: Range,
    /// Range of the `<Fragment>` body within `code`.
    pub body: Range,
    pub frontmatter_status: AstroFrontmatterStatus,
    /// Range of the frontmatter in the original source, fences included.
    pub frontmatter_source: Range,
    pub scripts: Vec<ExtractedTag>,
    pub styles: Vec<ExtractedTag>,
    pub diagnostics: Vec<AstroDiagnostic>,
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

    let source_index = Utf16Index::new(&source);
    let generated_index = Utf16Index::new(&result.code);
    let code_len = result.code.len() as u32;
    let source_len = source_index.convert(source.len() as u32);

    let mut generated_offsets = Vec::new();
    let mut source_offsets = Vec::new();
    let mut lengths = Vec::new();
    for (index, mapping) in result.mappings.iter().enumerate() {
        let Some(original) = mapping.original else {
            continue;
        };
        let run_end = result
            .mappings
            .get(index + 1)
            .map(|next| next.generated)
            .unwrap_or(code_len);
        let generated = generated_index.convert(mapping.generated);
        let source = source_index.convert(original);
        // Runs are verbatim copies, so one UTF-16 length serves both sides.
        // The printer maps trailing output one past EOF, so clamp to the source.
        let length = (generated_index.convert(run_end) - generated).min(source_len - source);
        if length == 0 {
            continue;
        }
        generated_offsets.push(generated);
        source_offsets.push(source);
        lengths.push(length);
    }

    ConvertToTsxResult {
        frontmatter: Range {
            start: generated_index.convert(result.frontmatter_range.start),
            end: generated_index.convert(result.frontmatter_range.end),
        },
        body: Range {
            start: generated_index.convert(result.body.start),
            end: generated_index.convert(result.body.end),
        },
        frontmatter_status: match result.frontmatter.status {
            FrontmatterStatus::DoesntExist => AstroFrontmatterStatus::DoesntExist,
            FrontmatterStatus::Open => AstroFrontmatterStatus::Open,
            FrontmatterStatus::Closed => AstroFrontmatterStatus::Closed,
        },
        frontmatter_source: Range {
            start: source_index.convert(result.frontmatter.source.start),
            end: source_index.convert(result.frontmatter.source.end),
        },
        scripts: result
            .scripts
            .iter()
            .map(|tag| extracted_tag_to_napi(tag, &source_index))
            .collect(),
        styles: result
            .styles
            .iter()
            .map(|tag| extracted_tag_to_napi(tag, &source_index))
            .collect(),
        diagnostics: result
            .diagnostics
            .iter()
            .map(|diagnostic| AstroDiagnostic {
                message: diagnostic.message.clone(),
                severity: match diagnostic.severity {
                    DiagnosticSeverity::Error => 1,
                    DiagnosticSeverity::Warning => 2,
                    DiagnosticSeverity::Information => 3,
                    DiagnosticSeverity::Hint => 4,
                },
                position: Range {
                    start: source_index.convert(diagnostic.source.start),
                    end: source_index.convert(diagnostic.source.end),
                },
            })
            .collect(),
        code: result.code,
        map,
        generated_offsets: Uint32Array::new(generated_offsets),
        source_offsets: Uint32Array::new(source_offsets),
        lengths: Uint32Array::new(lengths),
        has_parse_errors: result.has_parse_errors,
    }
}

fn extracted_tag_to_napi(tag: &crate::ExtractedTag, source_index: &Utf16Index) -> ExtractedTag {
    ExtractedTag {
        position: Range {
            start: source_index.convert(tag.source.start),
            end: source_index.convert(tag.source.end),
        },
        kind: match tag.kind {
            ExtractedKind::Script => ExtractedTagKind::Script,
            ExtractedKind::Style => ExtractedTagKind::Style,
            ExtractedKind::StyleAttribute => ExtractedTagKind::StyleAttribute,
            ExtractedKind::EventAttribute => ExtractedTagKind::EventAttribute,
        },
        content: tag.content.clone(),
        lang: tag.lang.clone(),
    }
}
