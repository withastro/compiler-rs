pub use oxc_sourcemap::SourceMap;
use oxc_sourcemap::SourceMapBuilder;

/// Used for `sources[0]` when the caller supplies no filename.
pub const DEFAULT_SOURCE_NAME: &str = "input.astro";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceMapMode {
    /// Append a `//# sourceMappingURL=` data URI comment to `code`.
    #[default]
    Inline,
    /// Leave `code` alone; the map is only available separately.
    External,
}

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

pub fn to_inline_comment(map: &SourceMap) -> String {
    format!("//# sourceMappingURL={}", map.to_data_url())
}

struct LineIndex<'a> {
    text: &'a str,
    line_starts: Vec<usize>,
    last_offset: usize,
    last_line: u32,
    last_column: u32,
}

impl<'a> LineIndex<'a> {
    fn new(text: &'a str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(index, _)| index + 1));
        Self {
            text,
            line_starts,
            last_offset: 0,
            last_line: 0,
            last_column: 0,
        }
    }

    /// Zero-based line and column; consumers index columns in UTF-16 units.
    fn locate(&mut self, offset: u32) -> (u32, u32) {
        let mut offset = (offset as usize).min(self.text.len());
        // A mid-character byte offset must clamp, not panic, mid-conversion.
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = (self.line_starts.partition_point(|&start| start <= offset) - 1) as u32;
        let (scan_from, mut column) = if line == self.last_line && offset >= self.last_offset {
            (self.last_offset, self.last_column)
        } else {
            (self.line_starts[line as usize], 0)
        };
        for ch in self.text[scan_from..offset].chars() {
            column += ch.len_utf16() as u32;
        }
        self.last_offset = offset;
        self.last_line = line;
        self.last_column = column;
        (line, column)
    }
}

/// Mappings must be sorted; mapped runs need a segment per line because columns reset.
pub(crate) fn encode(
    source: &str,
    generated: &str,
    mappings: &[Mapping],
    source_name: &str,
) -> SourceMap {
    let mut builder = SourceMapBuilder::default();
    let source_id = builder.add_source_and_content(source_name, source);
    let mut source_lines = LineIndex::new(source);
    let mut generated_lines = LineIndex::new(generated);
    let line_starts = generated_lines.line_starts.clone();

    for (index, mapping) in mappings.iter().enumerate() {
        let run_end = mappings
            .get(index + 1)
            .map(|next| next.generated)
            .unwrap_or(generated.len() as u32);

        let (dst_line, dst_col) = generated_lines.locate(mapping.generated);
        match mapping.original {
            Some(original) => {
                let (src_line, src_col) = source_lines.locate(original);
                builder.add_token(dst_line, dst_col, src_line, src_col, Some(source_id), None);

                let first_line_start =
                    line_starts.partition_point(|&start| start <= mapping.generated as usize);
                for &line_start in &line_starts[first_line_start..] {
                    if line_start as u32 >= run_end {
                        break;
                    }
                    let advanced = original + (line_start as u32 - mapping.generated);
                    let (dst_line, dst_col) = generated_lines.locate(line_start as u32);
                    let (src_line, src_col) = source_lines.locate(advanced);
                    builder.add_token(dst_line, dst_col, src_line, src_col, Some(source_id), None);
                }
            }
            None => builder.add_token(dst_line, dst_col, 0, 0, None, None),
        }
    }

    builder.into_sourcemap()
}

#[cfg(test)]
mod tests {
    use super::LineIndex;

    #[test]
    fn columns_count_utf16_code_units() {
        let mut index = LineIndex::new("<p>🦄 {π}</p>");
        assert_eq!(index.locate(0), (0, 0));
        assert_eq!(index.locate(3), (0, 3));
        assert_eq!(index.locate(7), (0, 5));
        assert_eq!(index.locate(8), (0, 6));
    }

    #[test]
    fn lines_are_zero_based_and_columns_restart() {
        let mut index = LineIndex::new("ab\ncd\n\nef");
        assert_eq!(index.locate(0), (0, 0));
        assert_eq!(index.locate(3), (1, 0));
        assert_eq!(index.locate(4), (1, 1));
        assert_eq!(index.locate(6), (2, 0));
        assert_eq!(index.locate(7), (3, 0));
    }

    #[test]
    fn non_monotonic_queries_rescan_correctly() {
        let mut index = LineIndex::new("aπb\ncd");
        assert_eq!(index.locate(4), (0, 3));
        assert_eq!(index.locate(1), (0, 1));
        assert_eq!(index.locate(5), (1, 0));
        assert_eq!(index.locate(6), (1, 1));
        assert_eq!(index.locate(4), (0, 3));
    }
}
