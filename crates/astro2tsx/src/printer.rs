//! Mutable output buffer: generated TSX bytes plus source-position mappings.

use biome_rowan::TextRange;

use crate::sourcemap::{
    Diagnostic, ExtractedKind, ExtractedScriptType, ExtractedTag, FrontmatterInfo, GeneratedRange,
    Mapping, SourceRange,
};
use crate::utils::{comment_body_escape, template_text_escape};

pub(crate) struct Printer<'a> {
    pub(crate) source: &'a str,
    /// Ranges of `<!-- ... -->` trivia, ascending, straight from the lexer.
    pub(crate) comment_ranges: Vec<TextRange>,
    pub(crate) output: String,
    pub(crate) mappings: Vec<Mapping>,
    pub(crate) frontmatter_range: GeneratedRange,
    pub(crate) body_range: GeneratedRange,
    pub(crate) scripts: Vec<ExtractedTag>,
    pub(crate) styles: Vec<ExtractedTag>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) frontmatter_info: FrontmatterInfo,
    /// An expression body failed to parse and was emitted verbatim.
    pub(crate) has_expression_errors: bool,
}

impl<'a> Printer<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        Self {
            source,
            comment_ranges: Vec::new(),
            output: String::new(),
            mappings: Vec::new(),
            frontmatter_range: GeneratedRange::default(),
            body_range: GeneratedRange::default(),
            scripts: Vec::new(),
            styles: Vec::new(),
            diagnostics: Vec::new(),
            frontmatter_info: FrontmatterInfo::default(),
            has_expression_errors: false,
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
        self.push_mapping(Mapping::original_at(generated, original));
    }

    pub(crate) fn map_nil(&mut self) {
        let generated = self.position();
        self.push_mapping(Mapping::nil(generated));
    }

    /// A mapping opens a run: generated and original advance in lockstep until
    /// the next mapping. Pairs that merely continue the current run are dropped.
    fn push_mapping(&mut self, mapping: Mapping) {
        if let Some(last) = self.mappings.last_mut() {
            if last.generated == mapping.generated {
                *last = mapping;
                return;
            }
            let continues = match (last.original, mapping.original) {
                (None, None) => true,
                (Some(last_original), Some(original)) => {
                    original.wrapping_sub(last_original) == mapping.generated - last.generated
                }
                _ => false,
            };
            if continues {
                return;
            }
        }
        self.mappings.push(mapping);
    }

    /// Emits `text` as one lockstep run starting at `original_start`.
    pub(crate) fn write_with_mapping(&mut self, text: &str, original_start: u32) {
        if text.is_empty() {
            return;
        }
        self.map_to_offset(original_start);
        self.output.push_str(text);
    }

    /// JSX text cannot contain raw `<`, `>`, or `}`; they emit as `{\`>\`}`.
    pub(crate) fn write_jsx_text_with_mapping(&mut self, text: &str, original_start: u32) {
        let mut original = original_start;
        for ch in text.chars() {
            if ch == '<' || ch == '>' || ch == '}' {
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

    /// Template-body escaping; escapes differ from their source byte, so they stay nil-mapped.
    pub(crate) fn write_template_text_with_mapping(&mut self, text: &str, original_start: u32) {
        let mut original = original_start;
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            match template_text_escape(ch, chars.peek().copied()) {
                Some(escaped) => self.write_nil_mapped(escaped),
                None => {
                    self.map_to_offset(original);
                    self.output.push(ch);
                }
            }
            original += ch.len_utf8() as u32;
        }
    }

    /// JSX-comment-body escaping, with the same mapping guarantee as above.
    pub(crate) fn write_comment_body_with_mapping(&mut self, body: &str, original_start: u32) {
        let mut original = original_start;
        let mut previous = None;
        for ch in body.chars() {
            match comment_body_escape(previous, ch) {
                Some(escaped) => self.write_nil_mapped(escaped),
                None => {
                    self.map_to_offset(original);
                    self.output.push(ch);
                }
            }
            previous = Some(ch);
            original += ch.len_utf8() as u32;
        }
    }

    /// Escaping for a value emitted inside synthetic double quotes.
    pub(crate) fn write_attribute_value_with_mapping(&mut self, text: &str, original_start: u32) {
        let mut original = original_start;
        for ch in text.chars() {
            if ch == '"' {
                self.write_nil_mapped("&quot;");
            } else {
                self.map_to_offset(original);
                self.output.push(ch);
            }
            original += ch.len_utf8() as u32;
        }
    }

    fn write_nil_mapped(&mut self, text: &str) {
        self.map_nil();
        self.output.push_str(text);
    }

    pub(crate) fn add_script_block(
        &mut self,
        range: GeneratedRange,
        source: SourceRange,
        content: String,
        script_type: ExtractedScriptType,
    ) {
        self.scripts.push(ExtractedTag {
            range,
            source,
            kind: ExtractedKind::Script,
            content,
            lang: None,
            script_type: Some(script_type),
        });
    }

    pub(crate) fn add_style_block(
        &mut self,
        range: GeneratedRange,
        source: SourceRange,
        content: String,
        lang: String,
    ) {
        self.styles.push(ExtractedTag {
            range,
            source,
            kind: ExtractedKind::Style,
            content,
            lang: Some(lang),
            script_type: None,
        });
    }

    pub(crate) fn add_event_attribute(
        &mut self,
        range: GeneratedRange,
        source: SourceRange,
        content: String,
    ) {
        self.scripts.push(ExtractedTag {
            range,
            source,
            kind: ExtractedKind::EventAttribute,
            content,
            lang: None,
            script_type: Some(ExtractedScriptType::EventAttribute),
        });
    }

    pub(crate) fn add_style_attribute(
        &mut self,
        range: GeneratedRange,
        source: SourceRange,
        content: String,
    ) {
        self.styles.push(ExtractedTag {
            range,
            source,
            kind: ExtractedKind::StyleAttribute,
            content,
            lang: Some("css".to_string()),
            script_type: None,
        });
    }
}

pub(crate) fn range_start(range: TextRange) -> u32 {
    u32::from(range.start())
}
