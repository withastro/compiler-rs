//! Internal mutable buffer the renderer writes into. Tracks both the
//! generated TSX bytes and the running list of source-position mappings.

use biome_rowan::TextRange;

use crate::sourcemap::{ExtractedKind, ExtractedTag, GeneratedRange, Mapping};

pub(crate) struct Printer {
    pub(crate) source: String,
    pub(crate) output: String,
    pub(crate) mappings: Vec<Mapping>,
    pub(crate) frontmatter_range: GeneratedRange,
    pub(crate) body_range: GeneratedRange,
    pub(crate) scripts: Vec<ExtractedTag>,
    pub(crate) styles: Vec<ExtractedTag>,
}

impl Printer {
    pub(crate) fn new(source: &str) -> Self {
        Self {
            source: source.to_string(),
            output: String::new(),
            mappings: Vec::new(),
            frontmatter_range: GeneratedRange::default(),
            body_range: GeneratedRange::default(),
            scripts: Vec::new(),
            styles: Vec::new(),
        }
    }

    pub(crate) fn position(&self) -> u32 {
        self.output.len() as u32
    }

    pub(crate) fn write(&mut self, text: &str) {
        self.output.push_str(text);
    }

    pub(crate) fn map_to_offset(&mut self, original: u32) {
        let generated = self.position();
        self.mappings
            .push(Mapping::original_at(generated, original));
    }

    pub(crate) fn map_nil(&mut self) {
        let generated = self.position();
        self.mappings.push(Mapping::nil(generated));
    }

    /// Emits `text` while attaching a per-character mapping back to the
    /// source range starting at `original_start`.
    pub(crate) fn write_with_mapping(&mut self, text: &str, original_start: u32) {
        let mut original = original_start;
        for ch in text.chars() {
            self.map_to_offset(original);
            self.output.push(ch);
            original += ch.len_utf8() as u32;
        }
    }

    /// JSX cannot contain raw `>` or `}`, so emit them inside `{\`...\`}`
    /// when they show up in text content.
    pub(crate) fn write_jsx_text_with_mapping(&mut self, text: &str, original_start: u32) {
        let mut original = original_start;
        for ch in text.chars() {
            if ch == '>' || ch == '}' {
                self.map_nil();
                self.output.push_str("{`");
                self.map_to_offset(original);
                self.output.push(ch);
                self.map_nil();
                self.output.push_str("`}");
            } else {
                self.map_to_offset(original);
                self.output.push(ch);
            }
            original += ch.len_utf8() as u32;
        }
    }

    pub(crate) fn record_frontmatter_range(&mut self, range: GeneratedRange) {
        self.frontmatter_range = range;
    }

    pub(crate) fn record_body_range(&mut self, range: GeneratedRange) {
        self.body_range = range;
    }

    pub(crate) fn add_script_block(
        &mut self,
        range: GeneratedRange,
        content: String,
        kind_label: &str,
    ) {
        self.scripts.push(ExtractedTag {
            range,
            kind: ExtractedKind::Script,
            content,
            lang: Some(kind_label.to_string()),
        });
    }

    pub(crate) fn add_style_block(&mut self, range: GeneratedRange, content: String, lang: String) {
        self.styles.push(ExtractedTag {
            range,
            kind: ExtractedKind::Style,
            content,
            lang: Some(lang),
        });
    }

    pub(crate) fn add_event_attribute(&mut self, range: GeneratedRange, content: String) {
        self.scripts.push(ExtractedTag {
            range,
            kind: ExtractedKind::EventAttribute,
            content,
            lang: Some("event-attribute".to_string()),
        });
    }

    pub(crate) fn add_style_attribute(&mut self, range: GeneratedRange, content: String) {
        self.styles.push(ExtractedTag {
            range,
            kind: ExtractedKind::StyleAttribute,
            content,
            lang: Some("css".to_string()),
        });
    }
}

pub(crate) fn range_start(range: TextRange) -> u32 {
    u32::from(range.start())
}
