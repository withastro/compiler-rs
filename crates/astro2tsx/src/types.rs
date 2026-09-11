#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    /// Offset into the generated TSX, in bytes.
    pub generated: u32,
    /// Offset into the original `.astro` source in bytes, or `None` for emitted
    /// text that has no corresponding source.
    pub original: Option<u32>,
}

impl Mapping {
    pub(crate) fn original_at(generated: u32, original: u32) -> Self {
        Self {
            generated,
            original: Some(original),
        }
    }

    pub(crate) fn nil(generated: u32) -> Self {
        Self {
            generated,
            original: None,
        }
    }
}

/// Byte range inside the generated TSX.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GeneratedRange {
    pub start: u32,
    pub end: u32,
}

impl GeneratedRange {
    pub(crate) fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}

/// Byte range inside the original `.astro` source. Distinct from
/// [`GeneratedRange`] so the two coordinate spaces cannot be mixed up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceRange {
    pub start: u32,
    pub end: u32,
}

impl SourceRange {
    pub(crate) fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}

#[napi_derive::napi]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error = 1,
    Warning = 2,
    Information = 3,
    Hint = 4,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub severity: DiagnosticSeverity,
    pub source: SourceRange,
}

#[napi_derive::napi(string_enum = "kebab-case", js_name = "AstroFrontmatterStatus")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FrontmatterStatus {
    #[default]
    DoesntExist,
    /// An opening fence with no closing one.
    Open,
    Closed,
}

/// `source` spans the opening fence through the end of the closing fence,
/// so it doubles as the offset where body content starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrontmatterInfo {
    pub status: FrontmatterStatus,
    pub source: SourceRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedTag {
    /// Range of `content` within the generated TSX, for every kind.
    pub range: GeneratedRange,
    /// Range of `content` within the original source.
    pub source: SourceRange,
    pub kind: ExtractedKind,
    pub content: String,
    /// Style language (`css`, `scss`, …); `None` for scripts.
    pub lang: Option<String>,
    /// How a script's contents should be treated; `None` for styles.
    pub script_type: Option<ExtractedScriptType>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractedKind {
    Script,
    Style,
    StyleAttribute,
    EventAttribute,
}

/// A bare `<script>` is processed by Astro; anything else is inlined as
/// written. `Unknown` covers a `type` whose value cannot be known statically.
#[napi_derive::napi(string_enum = "kebab-case")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractedScriptType {
    ProcessedModule,
    Module,
    Inline,
    EventAttribute,
    Json,
    Raw,
    Unknown,
}
