//! Adapts byte-based compiler ranges to UTF-16 offsets and TypeScript Content Mapper spans.

use napi_derive::napi;

use crate::utf16::Utf16Index;
use crate::{
    ConvertOptions, DiagnosticSeverity, ExtractedKind, ExtractedScriptType, FrontmatterStatus,
    GeneratedRange, SourceRange, convert_to_tsx as convert_rs,
};

const SPAN_MAP_KIND_VERBATIM: u32 = 0;
const SPAN_MAP_KIND_ATOM: u32 = 1;
const SPAN_MAP_FEATURE_DEFINITION: u32 = 1 << 3;
const SPAN_MAP_FEATURE_REFERENCES: u32 = 1 << 6;

#[napi(object)]
pub struct Range {
    pub start: u32,
    pub end: u32,
}

#[napi(string_enum = "kebab-case")]
pub enum ExtractedStyleType {
    Tag,
    StyleAttribute,
}

#[napi(object)]
pub struct ExtractedScript {
    /// Range of `content` in the original source.
    pub position: Range,
    pub content: String,
    #[napi(js_name = "type")]
    pub r#type: ExtractedScriptType,
}

#[napi(object)]
pub struct ExtractedStyle {
    /// Range of `content` in the original source.
    pub position: Range,
    pub content: String,
    #[napi(js_name = "type")]
    pub r#type: ExtractedStyleType,
    /// Normalized `lang` attribute: `css` when absent, `unknown` when dynamic.
    pub lang: String,
}

#[napi(object)]
pub struct AstroDiagnostic {
    pub message: String,
    pub severity: DiagnosticSeverity,
    pub position: Range,
}

#[napi(object)]
#[derive(Default)]
pub struct ConvertToTsxOptions {
    /// Filename used to name the component (e.g. `MyPage.astro` produces
    /// `MyPageAstroComponent`, also exported as `MyPage` for auto-imports
    /// unless frontmatter already exports that name).
    pub filename: Option<String>,
    /// Appends fallback declarations for `Fragment` and `Astro`. Off by default.
    /// A specialized `Astro` declaration is emitted regardless of this option
    /// when frontmatter declares `Props` or exports `getStaticPaths`.
    pub ambient_types: Option<bool>,
}

#[napi(object)]
pub struct ConvertToTsxResult {
    pub code: String,
    /// UTF-16 range of the clean-name re-export in `code`, including its trailing newline.
    /// Absent for dynamic routes, filenames without a matching clean component name,
    /// and names already exported by frontmatter.
    pub generated_component_export: Option<Range>,
    /// TypeScript Content Mapper span mappings in UTF-16 code units.
    #[napi(
        ts_type = "[virtualStart: number, virtualLength: number, originalStart: number, originalLength: number, kind: 0 | 1 | 2, features?: number][]"
    )]
    pub mappings: Vec<Vec<u32>>,
    /// Range of the frontmatter section within `code`.
    pub frontmatter: Range,
    /// Range of the template within `code`, excluding the `<Fragment>` wrappers.
    pub body: Range,
    #[napi(ts_type = "AstroFrontmatterStatus")]
    pub frontmatter_status: FrontmatterStatus,
    /// Range of the frontmatter in the original source, fences included.
    pub frontmatter_source: Range,
    pub scripts: Vec<ExtractedScript>,
    pub styles: Vec<ExtractedStyle>,
    pub diagnostics: Vec<AstroDiagnostic>,
    pub has_parse_errors: bool,
}

/// Convert an Astro source file to TSX for TypeScript editor tooling.
///
/// Recoverable parse errors produce best-effort TSX and set `hasParseErrors`.
/// Diagnostics include ranges in the original source.
#[napi(js_name = "convertToTsx")]
pub fn convert_to_tsx(source: String, options: Option<ConvertToTsxOptions>) -> ConvertToTsxResult {
    let options = options.unwrap_or_default();
    let result = convert_rs(
        &source,
        ConvertOptions {
            filename: options.filename,
            ambient_types: options.ambient_types.unwrap_or(false),
        },
    );
    let source_index = Utf16Index::new(&source);
    let generated_index = Utf16Index::new(&result.code);
    let code_len = result.code.len() as u32;
    let source_len = source_index.convert(source.len() as u32);

    let mut mappings = Vec::new();
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
        // EOF anchors may extend into generated separators, so their runs stop at the source end.
        let length =
            (generated_index.convert(run_end) - generated).min(source_len.saturating_sub(source));
        if length == 0 {
            continue;
        }
        mappings.push(vec![
            generated,
            length,
            source,
            length,
            SPAN_MAP_KIND_VERBATIM,
        ]);
    }

    let GeneratedRange { start, end } = result.component_name_range;
    // TypeScript does not resolve definitions or references through an unmapped virtual export.
    // See https://github.com/microsoft/TypeScript/pull/63936.
    mappings.push(vec![
        generated_index.convert(start),
        generated_index.convert(end) - generated_index.convert(start),
        0,
        0,
        SPAN_MAP_KIND_ATOM,
        SPAN_MAP_FEATURE_DEFINITION | SPAN_MAP_FEATURE_REFERENCES,
    ]);

    ConvertToTsxResult {
        generated_component_export: result
            .generated_component_export
            .map(|range| generated_range_to_napi(range, &generated_index)),
        frontmatter: generated_range_to_napi(result.frontmatter_range, &generated_index),
        body: generated_range_to_napi(result.body, &generated_index),
        frontmatter_status: result.frontmatter.status,
        frontmatter_source: source_range_to_napi(result.frontmatter.source, &source_index),
        scripts: result
            .scripts
            .into_iter()
            .map(|tag| extracted_script_to_napi(tag, &source_index))
            .collect(),
        styles: result
            .styles
            .into_iter()
            .map(|tag| extracted_style_to_napi(tag, &source_index))
            .collect(),
        diagnostics: result
            .diagnostics
            .into_iter()
            .map(|diagnostic| AstroDiagnostic {
                message: diagnostic.message,
                severity: diagnostic.severity,
                position: source_range_to_napi(diagnostic.source, &source_index),
            })
            .collect(),
        code: result.code,
        mappings,
        has_parse_errors: result.has_parse_errors,
    }
}

fn generated_range_to_napi(range: GeneratedRange, index: &Utf16Index) -> Range {
    Range {
        start: index.convert(range.start),
        end: index.convert(range.end),
    }
}

fn source_range_to_napi(range: SourceRange, index: &Utf16Index) -> Range {
    Range {
        start: index.convert(range.start),
        end: index.convert(range.end),
    }
}

fn extracted_script_to_napi(
    tag: crate::ExtractedTag,
    source_index: &Utf16Index,
) -> ExtractedScript {
    ExtractedScript {
        position: source_range_to_napi(tag.source, source_index),
        content: tag.content,
        r#type: tag.script_type.unwrap_or(ExtractedScriptType::Unknown),
    }
}

fn extracted_style_to_napi(tag: crate::ExtractedTag, source_index: &Utf16Index) -> ExtractedStyle {
    ExtractedStyle {
        position: source_range_to_napi(tag.source, source_index),
        content: tag.content,
        r#type: match tag.kind {
            ExtractedKind::StyleAttribute => ExtractedStyleType::StyleAttribute,
            _ => ExtractedStyleType::Tag,
        },
        lang: tag.lang.unwrap_or_else(|| "css".to_string()),
    }
}
