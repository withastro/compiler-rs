//! Source-position information collected as the printer emits TSX, and its
//! encoding as a standard Source Map v3 document.

use serde::Serialize;

/// Used for `sources[0]` when the caller supplies no filename.
pub const DEFAULT_SOURCE_NAME: &str = "input.astro";

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedTag {
    /// Range of `content` within the generated TSX, for every kind.
    pub range: GeneratedRange,
    pub kind: ExtractedKind,
    pub content: String,
    pub lang: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractedKind {
    Script,
    Style,
    StyleAttribute,
    EventAttribute,
}

/// A Source Map v3 document. `sourcesContent` is always populated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SourceMap {
    pub version: u8,
    pub sources: Vec<String>,
    #[serde(rename = "sourcesContent")]
    pub sources_content: Vec<Option<String>>,
    pub names: Vec<String>,
    pub mappings: String,
}

impl SourceMap {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a source map contains only serialisable types")
    }
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn push_vlq(out: &mut String, value: i64) {
    let mut remaining = if value < 0 {
        ((value.unsigned_abs()) << 1) | 1
    } else {
        (value as u64) << 1
    };
    loop {
        let mut digit = (remaining & 0b1_1111) as usize;
        remaining >>= 5;
        if remaining > 0 {
            digit |= 0b10_0000;
        }
        out.push(BASE64[digit] as char);
        if remaining == 0 {
            return;
        }
    }
}

struct LineIndex<'a> {
    text: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    fn new(text: &'a str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(index, _)| index + 1));
        Self { text, line_starts }
    }

    /// Zero-based line and column; consumers index columns in UTF-16 units.
    fn locate(&self, offset: u32) -> (u32, u32) {
        let offset = (offset as usize).min(self.text.len());
        let line = self.line_starts.partition_point(|&start| start <= offset) - 1;
        let line_start = self.line_starts[line];
        let mut column = 0usize;
        for (index, ch) in self.text[line_start..].char_indices() {
            if line_start + index >= offset {
                break;
            }
            column += ch.len_utf16();
        }
        (line as u32, column as u32)
    }
}

pub(crate) fn encode(
    source: &str,
    generated: &str,
    mappings: &[Mapping],
    source_name: &str,
) -> SourceMap {
    let source_lines = LineIndex::new(source);
    let generated_lines = LineIndex::new(generated);

    let mut encoded = String::new();
    let mut line = 0u32;
    let mut column = 0i64;
    let mut source_line = 0i64;
    let mut source_column = 0i64;
    let mut first_on_line = true;

    for mapping in mappings {
        let (mapping_line, mapping_column) = generated_lines.locate(mapping.generated);
        while line < mapping_line {
            encoded.push(';');
            line += 1;
            column = 0;
            first_on_line = true;
        }
        if !first_on_line {
            encoded.push(',');
        }
        first_on_line = false;

        push_vlq(&mut encoded, i64::from(mapping_column) - column);
        column = i64::from(mapping_column);

        let Some(original) = mapping.original else {
            continue;
        };
        let (original_line, original_column) = source_lines.locate(original);
        // There is only ever one source, so its index never moves.
        push_vlq(&mut encoded, 0);
        push_vlq(&mut encoded, i64::from(original_line) - source_line);
        source_line = i64::from(original_line);
        push_vlq(&mut encoded, i64::from(original_column) - source_column);
        source_column = i64::from(original_column);
    }

    SourceMap {
        version: 3,
        sources: vec![source_name.to_string()],
        sources_content: vec![Some(source.to_string())],
        names: Vec::new(),
        mappings: encoded,
    }
}

#[cfg(test)]
mod tests {
    use super::{LineIndex, push_vlq};

    fn vlq(value: i64) -> String {
        let mut out = String::new();
        push_vlq(&mut out, value);
        out
    }

    #[test]
    fn vlq_matches_known_values() {
        assert_eq!(vlq(0), "A");
        assert_eq!(vlq(1), "C");
        assert_eq!(vlq(-1), "D");
        assert_eq!(vlq(15), "e");
        assert_eq!(vlq(16), "gB");
        assert_eq!(vlq(-16), "hB");
        assert_eq!(vlq(1000), "w+B");
    }

    #[test]
    fn columns_count_utf16_code_units() {
        let index = LineIndex::new("<p>🦄 {π}</p>");
        assert_eq!(index.locate(0), (0, 0));
        assert_eq!(index.locate(3), (0, 3));
        // The emoji is four bytes but two UTF-16 code units.
        assert_eq!(index.locate(7), (0, 5));
        assert_eq!(index.locate(8), (0, 6));
    }

    #[test]
    fn lines_are_zero_based_and_columns_restart() {
        let index = LineIndex::new("ab\ncd\n\nef");
        assert_eq!(index.locate(0), (0, 0));
        assert_eq!(index.locate(3), (1, 0));
        assert_eq!(index.locate(4), (1, 1));
        assert_eq!(index.locate(6), (2, 0));
        assert_eq!(index.locate(7), (3, 0));
    }
}
