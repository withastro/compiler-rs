use biome_js_syntax::{JsSyntaxKind, JsSyntaxToken, unescape_js_string};
use biome_rowan::TextRange;

use crate::types::{
    Diagnostic, ExtractedKind, ExtractedScriptType, ExtractedTag, FrontmatterInfo, GeneratedRange,
    Mapping, SourceRange,
};
use crate::utils::{comment_body_escape, escape_javascript_string, template_text_escape};

pub(crate) struct Printer<'a> {
    pub(crate) source: &'a str,
    /// Lexer comment ranges sorted for partition-point lookups.
    pub(crate) comment_ranges: Vec<TextRange>,
    pub(crate) output: String,
    pub(crate) mappings: Vec<Mapping>,
    pub(crate) frontmatter_range: GeneratedRange,
    /// Generated completion slot used when the source has no frontmatter.
    pub(crate) frontmatter_insertion_range: Option<GeneratedRange>,
    pub(crate) body_range: GeneratedRange,
    pub(crate) scripts: Vec<ExtractedTag>,
    pub(crate) styles: Vec<ExtractedTag>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) frontmatter_info: FrontmatterInfo,
    pub(crate) has_embedded_parse_errors: bool,
    pub(crate) expressions_disabled: bool,
    /// Public component name that the default-export function would take, watched so that
    /// any frontmatter or template use of it can be detected during emission.
    pub(crate) watched_name: Option<String>,
    /// Whether an identifier matching `watched_name` was emitted.
    pub(crate) watched_name_seen: bool,
    /// Generated range of the render scope's outline label and the fence it names.
    pub(crate) scope_label: Option<(GeneratedRange, SourceRange)>,
    /// Zero-length `(generated, original)` anchors that give each detached chunk boundary one edit.
    pub(crate) edit_anchors: Vec<(u32, u32)>,
    /// Generated ranges whose mapped text opts out of editor features.
    pub(crate) featureless: Vec<GeneratedRange>,
}

impl<'a> Printer<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        Self {
            source,
            comment_ranges: Vec::new(),
            output: String::new(),
            mappings: Vec::new(),
            frontmatter_range: GeneratedRange::default(),
            frontmatter_insertion_range: None,
            body_range: GeneratedRange::default(),
            scripts: Vec::new(),
            styles: Vec::new(),
            diagnostics: Vec::new(),
            frontmatter_info: FrontmatterInfo::default(),
            has_embedded_parse_errors: false,
            expressions_disabled: false,
            watched_name: None,
            watched_name_seen: false,
            scope_label: None,
            edit_anchors: Vec::new(),
            featureless: Vec::new(),
        }
    }

    pub(crate) fn watch_name(&mut self, name: &str) {
        self.watched_name = Some(name.to_string());
    }

    /// Records an emitted identifier, flagging it when it matches the watched name.
    /// JavaScript identifiers must be unescaped by the caller.
    pub(crate) fn note_identifier(&mut self, identifier: &str) {
        if self.watched_name.as_deref() == Some(identifier) {
            self.watched_name_seen = true;
        }
    }

    /// Records an emitted JavaScript token if it references a value by name. Covers
    /// plain identifiers and JSX component names (`<Tree />`), which Biome lexes as
    /// `JSX_IDENT` inside a reference identifier.
    pub(crate) fn note_js_token(&mut self, token: &JsSyntaxToken) {
        if self.watched_name.is_none() {
            return;
        }
        let is_reference = match token.kind() {
            JsSyntaxKind::IDENT => true,
            JsSyntaxKind::JSX_IDENT => token
                .parent()
                .is_some_and(|parent| parent.kind() == JsSyntaxKind::JSX_REFERENCE_IDENTIFIER),
            _ => false,
        };
        if is_reference {
            self.note_identifier(&unescape_js_string(token.token_text_trimmed()));
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

    fn push_mapping(&mut self, mapping: Mapping) {
        if let Some(last) = self.mappings.last_mut() {
            if last.generated == mapping.generated {
                *last = mapping;
                return;
            }
            let continues = match (last.original, mapping.original) {
                (None, None) => true,
                (Some(last_original), Some(original)) => {
                    // Source offsets can decrease: shorthand attributes map the same span twice.
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

    pub(crate) fn write_with_mapping(&mut self, text: &str, original_start: u32) {
        if text.is_empty() {
            return;
        }
        self.map_to_offset(original_start);
        self.output.push_str(text);
    }

    pub(crate) fn write_jsx_text_with_mapping(&mut self, text: &str, original_start: u32) {
        let mut original = original_start;
        for ch in text.chars() {
            if ch == '<' || ch == '>' || ch == '{' || ch == '}' {
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

    pub(crate) fn write_js_string_with_mapping(&mut self, text: &str, start: u32) {
        self.write_nil_mapped("\"");
        let mut buffer = [0; 4];
        for (offset, ch) in text.char_indices() {
            if matches!(ch, '"' | '\\' | '\u{2028}' | '\u{2029}') || ch.is_ascii_control() {
                let encoded = escape_javascript_string(ch.encode_utf8(&mut buffer));
                self.write_nil_mapped(&encoded[1..encoded.len() - 1]);
            } else {
                self.write_with_mapping(ch.encode_utf8(&mut buffer), start + offset as u32);
            }
        }
        self.write_nil_mapped("\"");
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
