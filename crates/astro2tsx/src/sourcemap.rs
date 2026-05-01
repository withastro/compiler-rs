//! Stores source-position information collected as the printer emits TSX.
//!
//! The upstream Go printer feeds entries directly into a VLQ-encoded
//! sourcemap chunk via `sourcemap.ChunkBuilder`. We collect raw `(generated,
//! original)` byte offsets here and leave VLQ encoding to consumers, which
//! keeps the dependency footprint of this crate minimal.

/// A single entry in the source-to-TSX byte mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    /// Offset into the generated TSX (in bytes).
    pub generated: u32,
    /// Offset into the original `.astro` source (in bytes), or `None` for
    /// emitted text that has no corresponding source — analogous to the
    /// upstream `addNilSourceMapping` calls.
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

/// Byte range inside the generated TSX. Produced for each "section" the
/// upstream compiler exposes (frontmatter, body, scripts, styles).
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

/// Mirrors `TSXExtractedTag` from the upstream printer: a chunk of script
/// or style content with the original `.astro` byte range it covers and a
/// classifier describing how the content should be interpreted downstream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedTag {
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
