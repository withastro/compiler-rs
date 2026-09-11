use biome_html_syntax::{AnyHtmlComponentObjectName, AnyHtmlTagName, HtmlRoot};
use biome_rowan::{AstNode, Direction, TextRange};

use crate::printer::{Printer, range_start};
use crate::types::{Diagnostic, DiagnosticSeverity, SourceRange};
use crate::utils::comment_needs_leading_space;

pub(super) fn tag_name_text(name: &AnyHtmlTagName) -> String {
    match name {
        AnyHtmlTagName::HtmlTagName(node) => node
            .value_token()
            .map(|t| t.text_trimmed().to_string())
            .unwrap_or_default(),
        AnyHtmlTagName::HtmlComponentName(node) => node
            .value_token()
            .map(|t| t.text_trimmed().to_string())
            .unwrap_or_default(),
        AnyHtmlTagName::HtmlMemberName(node) => member_name_text(node),
    }
}

fn member_name_text(node: &biome_html_syntax::HtmlMemberName) -> String {
    let object = node
        .object()
        .ok()
        .map(|object| match object {
            AnyHtmlComponentObjectName::HtmlComponentName(c) => c
                .value_token()
                .map(|t| t.text_trimmed().to_string())
                .unwrap_or_default(),
            AnyHtmlComponentObjectName::HtmlTagName(t) => t
                .value_token()
                .map(|t| t.text_trimmed().to_string())
                .unwrap_or_default(),
            AnyHtmlComponentObjectName::HtmlMemberName(inner) => member_name_text(&inner),
        })
        .unwrap_or_default();
    let member = node
        .member()
        .ok()
        .and_then(|m| m.value_token().ok())
        .map(|t| t.text_trimmed().to_string())
        .unwrap_or_default();
    format!("{object}.{member}")
}

/// HTML comments never become nodes; the lexer stores them as token trivia.
pub(super) fn comment_trivia_ranges(root: &HtmlRoot) -> Vec<TextRange> {
    let mut ranges = Vec::new();
    for token in root
        .syntax()
        .descendants_with_tokens(Direction::Next)
        .filter_map(|element| element.into_token())
    {
        for piece in token
            .leading_trivia()
            .pieces()
            .chain(token.trailing_trivia().pieces())
        {
            if piece.is_comments() {
                ranges.push(piece.text_range());
            }
        }
    }
    ranges
}

pub(super) fn emit_jsx_text_range(printer: &mut Printer, from: u32, to: u32) {
    if from >= to {
        return;
    }
    if let Some(text) = printer.source.get(from as usize..to as usize) {
        printer.write_jsx_text_with_mapping(text, from);
    }
}

pub(super) fn emit_source_gap(printer: &mut Printer, from: u32, to: u32) {
    let source = printer.source;
    let to = (to as usize).min(source.len());
    let mut cursor = from as usize;
    if to <= cursor {
        return;
    }

    let first = printer
        .comment_ranges
        .partition_point(|range| u32::from(range.end()) as usize <= cursor);
    for index in first..printer.comment_ranges.len() {
        let range = printer.comment_ranges[index];
        let (start, end) = (range_start(range) as usize, u32::from(range.end()) as usize);
        if start >= to {
            break;
        }
        if start < cursor || end > to {
            continue;
        }
        write_source_gap_text(printer, cursor as u32, start as u32);
        let text = &source[start..end];
        match text
            .strip_prefix("<!--")
            .and_then(|t| t.strip_suffix("-->"))
        {
            Some(body) => emit_html_comment(printer, body, start as u32 + 4),
            // An unterminated comment runs to the end of the file, as in HTML.
            None => {
                let body = text.strip_prefix("<!--").unwrap_or(text);
                let body_start = start as u32 + (text.len() - body.len()) as u32;
                printer.diagnostics.push(Diagnostic {
                    message: "Unterminated comment".to_string(),
                    severity: DiagnosticSeverity::Warning,
                    source: SourceRange::new(start as u32, end as u32),
                });
                emit_html_comment(printer, body, body_start);
            }
        }
        cursor = end;
    }
    write_source_gap_text(printer, cursor as u32, to as u32);
}

fn write_source_gap_text(printer: &mut Printer, from: u32, to: u32) {
    let text = &printer.source[from as usize..to as usize];
    if contains_non_ascii_tag_name(text) {
        printer.write_jsx_text_with_mapping(text, from);
    } else {
        printer.write_with_mapping(text, from);
    }
}

fn emit_html_comment(printer: &mut Printer, body: &str, original_offset: u32) {
    printer.map_nil();
    printer.write("{/**");
    if comment_needs_leading_space(body) {
        printer.write(" ");
    }
    printer.write_comment_body_with_mapping(body, original_offset);
    printer.map_nil();
    printer.write("*/}");
}

pub(super) fn contains_non_ascii_tag_name(text: &str) -> bool {
    text.match_indices('<').any(|(start, _)| {
        let rest = &text[start + 1..];
        let rest = rest.strip_prefix('/').unwrap_or(rest);
        let name = rest
            .split(|ch: char| ch.is_ascii_whitespace() || ch == '/' || ch == '>')
            .next()
            .unwrap_or_default();
        !name.is_empty() && !name.is_ascii()
    })
}

pub(super) fn slice_source(source: &str, range: TextRange) -> &str {
    let start = range_start(range) as usize;
    let end = u32::from(range.end()) as usize;
    source.get(start..end).unwrap_or("")
}
