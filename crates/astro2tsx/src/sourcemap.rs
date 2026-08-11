//! Stores source-position information collected as the printer emits TSX.
//!
//! Offsets are raw and unencoded; VLQ encoding is left to consumers.

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
