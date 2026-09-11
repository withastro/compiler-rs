use biome_html_syntax::{
    AnyHtmlAttribute, AnyHtmlContent, AnyHtmlElement, AnyHtmlTagName, AnyHtmlTextExpression,
    AstroFragment, HtmlElement, HtmlSelfClosingElement, HtmlSingleTextExpression,
};
use biome_js_parser::{JsOffsetParse, JsParserOptions, parse_js_with_offset};
use biome_languages::JsFileSource;
use biome_languages::javascript::JsEmbeddingKind;
use biome_rowan::{AstNode, AstNodeList, TextRange, TextSize, WalkEvent};

use crate::expression::emit_expression_tree;
use crate::printer::{Printer, range_start};
use crate::types::{Diagnostic, DiagnosticSeverity, GeneratedRange, SourceRange};
use crate::utils::{BodyMode, body_mode};

use super::attribute::{attribute_key, emit_intra_tag_space, emit_open_tag};
use super::extracted::{classify_script, inner_range, style_lang_label};
use super::text::{
    contains_non_ascii_tag_name, emit_jsx_text_range, emit_source_gap, slice_source, tag_name_text,
};

pub(super) fn render_element(printer: &mut Printer, element: AnyHtmlElement) {
    match element {
        AnyHtmlElement::AnyHtmlContent(content) => render_content(printer, content),
        AnyHtmlElement::HtmlElement(node) => render_html_element(printer, node),
        AnyHtmlElement::AstroFragment(node) => render_astro_fragment(printer, &node),
        AnyHtmlElement::HtmlSelfClosingElement(node) => {
            render_self_closing_element(printer, node);
        }
        AnyHtmlElement::HtmlCdataSection(_)
        | AnyHtmlElement::HtmlProcessingInstruction(_)
        | AnyHtmlElement::HtmlBogusElement(_) => {
            // Incomplete TSX-compatible syntax stays verbatim so editor completion can consume it.
            let range = element.range();
            let text = slice_source(printer.source, range);
            let start = range_start(range);
            // A stray doctype recovers as bogus text, but TSX has no doctype syntax.
            if text
                .get(..9)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("<!doctype"))
            {
                let rest_at = text.find('>').map(|i| i + 1).unwrap_or(text.len());
                printer.write_with_mapping(&text[rest_at..], start + rest_at as u32);
                return;
            }
            if contains_non_ascii_tag_name(text) {
                printer.write_jsx_text_with_mapping(text, start);
            } else {
                printer.write_with_mapping(text, start);
            }
        }
    }
}

fn render_content(printer: &mut Printer, content: AnyHtmlContent) {
    match content {
        AnyHtmlContent::HtmlContent(node) => {
            let Ok(token) = node.value_token() else {
                return;
            };
            // Trimmed range: the parent's gap emission owns surrounding trivia, else it doubles.
            let range = token.text_trimmed_range();
            let text = token.text_trimmed();
            printer.write_jsx_text_with_mapping(text, range_start(range));
        }
        AnyHtmlContent::HtmlEmbeddedContent(node) => {
            let Ok(token) = node.value_token() else {
                return;
            };
            let original_start = range_start(token.text_trimmed_range());
            let raw = token.text_trimmed().to_string();
            printer.map_nil();
            printer.write("{`");
            printer.write_template_text_with_mapping(&raw, original_start);
            printer.map_nil();
            printer.write("`}");
        }
        AnyHtmlContent::AnyHtmlTextExpression(expression) => {
            render_text_expression(printer, expression);
        }
    }
}

fn render_text_expression(printer: &mut Printer, expression: AnyHtmlTextExpression) {
    match expression {
        AnyHtmlTextExpression::HtmlSingleTextExpression(node) => {
            render_single_text_expression(printer, node);
        }
        AnyHtmlTextExpression::HtmlDoubleTextExpression(node) => {
            let range = node.range();
            let text = slice_source(printer.source, range);
            printer.write_with_mapping(text, range_start(range));
        }
        AnyHtmlTextExpression::HtmlBogusTextExpression(node) => {
            let range = node.range();
            let text = slice_source(printer.source, range);
            printer.write_with_mapping(text, range_start(range));
        }
        AnyHtmlTextExpression::AnySvelteBlock(_) => {}
    }
}

fn render_single_text_expression(printer: &mut Printer, node: HtmlSingleTextExpression) {
    let Ok(l_curly) = node.l_curly_token() else {
        return;
    };
    let Some(expression) = node.expression() else {
        return;
    };
    let Ok(r_curly) = node.r_curly_token() else {
        return;
    };

    let l_curly_range = l_curly.text_trimmed_range();
    let r_curly_range = r_curly.text_trimmed_range();

    printer.map_to_offset(range_start(l_curly_range));
    printer.write("{");

    if expression.html_literal_token().is_ok() {
        let original_start = u32::from(l_curly_range.end());
        // Whitespace touching `{` is trivia, which the literal token drops.
        let raw = slice_source(
            printer.source,
            TextRange::new(l_curly_range.end(), r_curly_range.start()),
        );
        if raw.is_empty() {
            printer.map_nil();
            printer.write("(void 0)");
        } else {
            emit_expression_body(printer, raw, original_start, false);
        }
    } else {
        printer.map_nil();
        printer.write("(void 0)");
    }

    printer.map_to_offset(range_start(r_curly_range));
    printer.write("}");
}

fn parse_expression_body(text: &str, base_offset: u32) -> JsOffsetParse {
    parse_js_with_offset(
        text,
        TextSize::from(base_offset),
        JsFileSource::tsx().with_embedding_kind(JsEmbeddingKind::Astro {
            frontmatter: false,
            is_class_attribute: false,
        }),
        JsParserOptions::default(),
    )
}

pub(super) fn emit_expression_body(
    printer: &mut Printer,
    raw: &str,
    original_start: u32,
    require_value: bool,
) {
    let parse = parse_expression_body(raw, original_start);
    let syntax = parse.syntax().into_inner();
    if parse.diagnostics().is_empty() {
        if require_value
            && syntax
                .first_token()
                .is_some_and(|token| token.kind() == biome_js_syntax::JsSyntaxKind::EOF)
        {
            printer.map_nil();
            printer.write("(void 0)");
        }
        emit_expression_tree(printer, &syntax, original_start);
    } else {
        printer.has_embedded_parse_errors = true;
        for diagnostic in parse.diagnostics() {
            printer.diagnostics.push(Diagnostic {
                message: diagnostic.message.to_string(),
                severity: DiagnosticSeverity::Error,
                source: diagnostic_source_range(diagnostic, original_start, raw.len() as u32),
            });
        }
        printer.write_with_mapping(raw, original_start);
    }
    drop(parse);
    crate::syntax::drop_syntax_tree(syntax);
}

fn diagnostic_source_range(
    diagnostic: &biome_parser::diagnostic::ParseDiagnostic,
    start: u32,
    len: u32,
) -> SourceRange {
    match biome_diagnostics::Diagnostic::location(diagnostic).span {
        Some(span) => SourceRange::new(
            start + u32::from(span.start()),
            start + u32::from(span.end()),
        ),
        None => SourceRange::new(start, start + len),
    }
}

fn render_html_element(printer: &mut Printer, node: HtmlElement) {
    let Ok(opening) = node.opening_element() else {
        return;
    };
    let Ok(name) = opening.name() else {
        return;
    };
    let Ok(open_l_angle) = opening.l_angle_token() else {
        return;
    };
    let open_r_angle = opening.r_angle_token().ok();

    if open_r_angle.is_none() {
        let range = node.syntax().text_trimmed_range();
        let text = slice_source(printer.source, range);
        printer.write_with_mapping(text, range_start(range));
        return;
    }

    let tag_name = tag_name_text(&name);
    let attributes: Vec<AnyHtmlAttribute> = opening.attributes().iter().collect();

    emit_open_tag(
        printer,
        &tag_name,
        range_start(open_l_angle.text_trimmed_range()),
        range_start(name.range()),
        &attributes,
    );

    let open_r_angle = open_r_angle.expect("checked above");
    let r_angle_start = range_start(open_r_angle.text_trimmed_range());
    let attrs_end = attributes
        .last()
        .map(|attr| u32::from(attr.range().end()))
        .unwrap_or_else(|| u32::from(name.range().end()));
    emit_intra_tag_space(printer, attrs_end, r_angle_start);
    printer.map_to_offset(r_angle_start);
    printer.write(">");

    // `<Script>` is a component, not the HTML element, so match the node kind.
    let is_html_tag = matches!(name, AnyHtmlTagName::HtmlTagName(_));
    let element_is_raw = attributes
        .iter()
        .any(|a| attribute_key(a).as_deref() == Some("is:raw"));
    let mode = body_mode(is_html_tag.then_some(tag_name.as_str()), element_is_raw);
    let is_script = mode == BodyMode::Script;
    let is_style = mode == BodyMode::Style;

    let children = node.children();
    let body_start = printer.position();

    let opening_end = u32::from(open_r_angle.text_trimmed_range().end());
    let inline_raw_body = mode == BodyMode::Raw;

    let closing_inner_start = node
        .closing_element()
        .ok()
        .and_then(|c| c.l_angle_token().ok())
        .map(|t| range_start(t.text_trimmed_range()));

    if is_script || is_style {
        // Keep the tag for TSX analysis while reporting its body separately.
    } else if inline_raw_body {
        let inner_start = opening_end;
        let inner_end = closing_inner_start.unwrap_or_else(|| u32::from(node.range().end()));
        if inner_end > inner_start {
            let raw = &printer.source[inner_start as usize..inner_end as usize];
            printer.map_nil();
            printer.write("{`");
            printer.write_template_text_with_mapping(raw, inner_start);
            printer.map_nil();
            printer.write("`}");
        }
    } else if mode == BodyMode::TextOnly {
        let mut prev_end = opening_end;
        let mut walk = children.syntax().preorder();
        while let Some(event) = walk.next() {
            let WalkEvent::Enter(node) = event else {
                continue;
            };
            if let Some(expression) = AnyHtmlTextExpression::cast(node) {
                let range = expression.range();
                emit_jsx_text_range(printer, prev_end, range_start(range));
                render_text_expression(printer, expression);
                prev_end = u32::from(range.end());
                walk.skip_subtree();
            }
        }
        emit_jsx_text_range(
            printer,
            prev_end,
            closing_inner_start.unwrap_or_else(|| u32::from(node.range().end())),
        );
    } else {
        let mut prev_end: Option<u32> = None;
        for child in children.iter() {
            let child_range = child.range();
            let child_start = range_start(child_range);
            let leading_from = prev_end.unwrap_or(opening_end);
            emit_source_gap(printer, leading_from, child_start);
            render_element(printer, child);
            prev_end = Some(u32::from(child_range.end()));
        }
        if let Some(trailing_to) = closing_inner_start {
            let leading_from = prev_end.unwrap_or(opening_end);
            emit_source_gap(printer, leading_from, trailing_to);
        }
    }

    let body_end = printer.position();

    if is_script || is_style {
        let inner_range = inner_range(&node);
        if let Some((start, end)) = inner_range {
            let content =
                slice_source(printer.source, TextRange::new(start.into(), end.into())).to_string();
            if is_script {
                printer.add_script_block(
                    GeneratedRange::new(body_start, body_end),
                    SourceRange::new(start, end),
                    content,
                    classify_script(&attributes),
                );
            } else {
                printer.add_style_block(
                    GeneratedRange::new(body_start, body_end),
                    SourceRange::new(start, end),
                    content,
                    style_lang_label(&attributes),
                );
            }
        }
    }

    if let Ok(closing) = node.closing_element() {
        if let Ok(l_angle) = closing.l_angle_token() {
            printer.map_to_offset(range_start(l_angle.text_trimmed_range()));
            printer.write("</");
        }
        if let Ok(closing_name) = closing.name() {
            printer.map_to_offset(range_start(closing_name.range()));
            printer.write(&tag_name_text(&closing_name));
        }
        if let Ok(r_angle) = closing.r_angle_token() {
            printer.map_to_offset(range_start(r_angle.text_trimmed_range()));
            printer.write(">");
        }
    }
}

fn render_self_closing_element(printer: &mut Printer, node: HtmlSelfClosingElement) {
    let Ok(name) = node.name() else {
        return;
    };
    let Ok(l_angle) = node.l_angle_token() else {
        return;
    };
    let Ok(r_angle) = node.r_angle_token() else {
        let range = node.syntax().text_trimmed_range();
        let text = slice_source(printer.source, range);
        printer.write_with_mapping(text, range_start(range));
        return;
    };
    let attributes: Vec<AnyHtmlAttribute> = node.attributes().iter().collect();
    let tag_name = tag_name_text(&name);

    emit_open_tag(
        printer,
        &tag_name,
        range_start(l_angle.text_trimmed_range()),
        range_start(name.range()),
        &attributes,
    );

    let r_angle_start = range_start(r_angle.text_trimmed_range());
    let pre_slash = node
        .slash_token()
        .map(|s| range_start(s.text_trimmed_range()))
        .unwrap_or(r_angle_start);
    let attrs_end = attributes
        .last()
        .map(|attr| u32::from(attr.range().end()))
        .unwrap_or_else(|| u32::from(name.range().end()));
    emit_intra_tag_space(printer, attrs_end, pre_slash);
    match node.slash_token() {
        Some(slash) => printer.map_to_offset(range_start(slash.text_trimmed_range())),
        None => printer.map_nil(),
    }
    printer.write("/");
    printer.map_to_offset(r_angle_start);
    printer.write(">");
}

fn render_astro_fragment(printer: &mut Printer, node: &AstroFragment) {
    let Ok(opening) = node.opening_fragment() else {
        return;
    };
    let Ok(open_l) = opening.l_angle_token() else {
        return;
    };
    let open_r = opening.r_angle_token().ok();

    printer.map_to_offset(range_start(open_l.text_trimmed_range()));
    printer.write("<");
    if let Some(open_r) = open_r.as_ref() {
        printer.map_to_offset(range_start(open_r.text_trimmed_range()));
        printer.write(">");
    }

    let opening_end = open_r
        .as_ref()
        .map(|t| u32::from(t.text_trimmed_range().end()))
        .unwrap_or_else(|| u32::from(opening.range().end()));

    let closing_inner_start = node
        .closing_fragment()
        .ok()
        .and_then(|c| c.l_angle_token().ok())
        .map(|t| range_start(t.text_trimmed_range()));

    let mut prev_end: Option<u32> = None;
    for child in node.children() {
        let child_range = child.range();
        let leading_from = prev_end.unwrap_or(opening_end);
        emit_source_gap(printer, leading_from, range_start(child_range));
        render_element(printer, child);
        prev_end = Some(u32::from(child_range.end()));
    }
    if let Some(trailing_to) = closing_inner_start {
        emit_source_gap(printer, prev_end.unwrap_or(opening_end), trailing_to);
    }

    if let Ok(closing) = node.closing_fragment() {
        if let Ok(l_angle) = closing.l_angle_token() {
            printer.map_to_offset(range_start(l_angle.text_trimmed_range()));
            printer.write("</");
        }
        if let Ok(r_angle) = closing.r_angle_token() {
            printer.map_to_offset(range_start(r_angle.text_trimmed_range()));
            printer.write(">");
        }
    }
}

#[cfg(test)]
mod tests {
    use biome_js_parser::{JsParserOptions, parse};
    use biome_languages::JsFileSource;

    use crate::test_utils::assert_mapped_runs_are_verbatim;
    use crate::{ConvertOptions, convert_to_tsx};

    #[test]
    fn body_range_is_recorded() {
        let result = convert_to_tsx("<h1>Hi</h1>", ConvertOptions::default());
        assert!(result.body.end > result.body.start);
        let body_slice = &result.code[result.body.start as usize..result.body.end as usize];
        assert!(body_slice.contains("<h1>"));
        assert!(body_slice.contains("</h1>"));
    }

    #[test]
    fn comments_before_the_first_element_survive() {
        for input in [
            "<!-- leading -->\n<div>x</div>",
            "---\nconst a = 1;\n---\n<!-- between -->\n<div>x</div>",
        ] {
            let actual = convert_to_tsx(input, ConvertOptions::default()).code;
            assert!(
                actual.contains("{/** leading */}") || actual.contains("{/** between */}"),
                "a leading comment was dropped for {input:?}:\n{actual}"
            );
            assert!(!actual.contains("<!--"), "untranslated comment:\n{actual}");
        }
    }

    #[test]
    fn doctype_never_reaches_the_output() {
        for input in [
            "<!doctype html>\n<html><body>x</body></html>",
            "<!DOCTYPE html>\n<html><body>x</body></html>",
            "---\nconst a = 1;\n---\n\n<!doctype html>\n<html lang=\"en\"><body>{a}</body></html>\n",
            "---\nconst a = 1;\n---\n<!doctype html>",
            "<div>hi</div>\n<!DOCTYPE html>\n<p>after</p>",
            "<!DOCTYPE html>\n<!DOCTYPE html>\n<p/>",
            "<main>\n<!doctype html>\n<p>x</p>\n</main>",
        ] {
            let actual = convert_to_tsx(input, ConvertOptions::default()).code;
            assert!(
                !actual.contains("<!"),
                "the doctype survived for {input:?}:\n{actual}"
            );
        }

        let actual = convert_to_tsx(
            "<!doctype html>\n<html><body>x</body></html>",
            ConvertOptions::default(),
        )
        .code;
        assert!(
            actual.contains("<html>"),
            "the html element was lost:\n{actual}"
        );

        let sibling = convert_to_tsx(
            "<div>hi</div>\n<!DOCTYPE html>\n<p>after</p>",
            ConvertOptions::default(),
        )
        .code;
        assert!(
            sibling.contains("<p>after</p>"),
            "content after a stray doctype was lost:\n{sibling}"
        );
    }

    #[test]
    fn bare_less_than_in_text_is_escaped() {
        for input in ["<p>a < b</p>", "<div>5 < 10 is true</div>"] {
            let result = convert_to_tsx(input, ConvertOptions::default());
            assert!(
                result.code.contains("{`<`}"),
                "bare < survived for {input:?}:\n{}",
                result.code
            );
            assert!(!result.has_parse_errors, "{input:?} should parse cleanly");
            assert_mapped_runs_are_verbatim(input, &result, "bare <");
        }
    }

    #[test]
    fn astro_text_that_is_not_tsx_text_is_escaped() {
        for input in [
            r#"<math><annotation>f\colon X \to \mathbb{R}^{2x}</annotation></math>"#,
            "<日本>hi</日本>",
        ] {
            let result = convert_to_tsx(input, ConvertOptions::default());
            assert!(
                !result.has_parse_errors,
                "{input:?}: {:?}",
                result.diagnostics
            );
            let parsed = parse(
                &result.code,
                JsFileSource::tsx(),
                JsParserOptions::default(),
            );
            assert!(
                parsed.diagnostics().is_empty(),
                "generated invalid TSX for {input:?}:\n{:?}\n{}",
                parsed.diagnostics(),
                result.code
            );
            assert_mapped_runs_are_verbatim(input, &result, "Astro-only text");
        }

        for element in [
            "iframe",
            "noembed",
            "noframes",
            "plaintext",
            "textarea",
            "title",
            "xmp",
        ] {
            let input = format!("<{element}>{{value}} with <b>tags</b></{element}>");
            let result = convert_to_tsx(&input, ConvertOptions::default());
            assert!(
                !result.has_parse_errors,
                "{input:?}: {:?}",
                result.diagnostics
            );
            let parsed = parse(
                &result.code,
                JsFileSource::tsx(),
                JsParserOptions::default(),
            );
            assert!(
                parsed.diagnostics().is_empty(),
                "generated invalid TSX for {input:?}:\n{:?}\n{}",
                parsed.diagnostics(),
                result.code
            );
            let expected = if matches!(element, "title" | "textarea") {
                "{value} with {`<`}b{`>`}tags{`<`}/b{`>`}"
            } else {
                "{`{value} with <b>tags</b>`}"
            };
            assert!(
                result.code.contains(expected),
                "tag-looking content should be text for {element}:\n{}",
                result.code
            );
            assert_mapped_runs_are_verbatim(&input, &result, "text-only HTML children");
        }

        for element in ["pre", "listing"] {
            let input = format!("<{element}>{{value}} with <b>tags</b></{element}>");
            let result = convert_to_tsx(&input, ConvertOptions::default());
            assert!(result.code.contains("{value} with <b>tags</b>"));
            assert_mapped_runs_are_verbatim(&input, &result, "structured raw-space children");
        }
    }

    #[test]
    fn text_only_elements_preserve_expressions_inside_literal_tags() {
        for tag in ["title", "textarea"] {
            for body in [
                "<b>{value}</b>",
                "<b><i>{value}</i></b>",
                "\n <b>\n {value} \n </b> \n",
                "<Widget>{value}</Widget>",
                "<b>{value}</b>{other}",
                "<b>{ok && <span>{value}</span>}</b>",
            ] {
                let markup = format!("<{tag}>{body}</{tag}>");
                for source in [markup.clone(), format!("{{{markup}}}")] {
                    let result = crate::test_utils::convert(&source);
                    assert!(result.code.contains("{value}"), "{}", result.code);
                    assert!(!result.code.contains("<b>"), "{}", result.code);
                    assert!(!result.code.contains("<Widget>"), "{}", result.code);
                    if body.contains("other") {
                        assert!(result.code.contains("{other}"), "{}", result.code);
                    }
                    if body.contains("span") {
                        assert!(
                            result.code.contains("<span>{value}</span>"),
                            "{}",
                            result.code
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn unclosed_text_only_elements_keep_literal_content() {
        for tag in ["title", "textarea"] {
            let source = format!("<{tag}>before <b>after</b>");
            let result = convert_to_tsx(&source, ConvertOptions::default());
            assert!(
                result.code.contains("before {`<`}b{`>`}after{`<`}/b{`>`}"),
                "{}",
                result.code
            );
            assert_mapped_runs_are_verbatim(&source, &result, "unclosed text-only element");
        }
    }

    #[test]
    fn raw_template_escapes_are_present_but_unmapped() {
        let source = "<div is:raw>a`b ${x}</div>";
        let result = convert_to_tsx(source, ConvertOptions::default());
        assert!(result.code.contains("{`a\\`b \\${x}`}"), "{}", result.code);
        assert_mapped_runs_are_verbatim(source, &result, "raw escapes");
    }
}
