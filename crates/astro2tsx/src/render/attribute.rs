use biome_html_syntax::{
    AnyAstroDirective, AnyHtmlAttribute, AnyHtmlAttributeInitializer, HtmlAttribute,
    HtmlAttributeInitializerClause, HtmlSpreadAttribute,
};
use biome_rowan::{AstNode, TextRange, TextSize};

use crate::printer::{Printer, range_start};
use crate::types::{GeneratedRange, SourceRange};
use crate::utils::{
    decode_html_entities, escape_javascript_string, is_html_event_attribute,
    is_valid_tsx_attribute_name, strip_matching_quotes,
};

use super::element::emit_expression_body;
use super::text::slice_source;

/// Tag-header gaps hold whitespace or JS comments — both valid TSX, so they round-trip whole.
pub(super) fn emit_intra_tag_space(printer: &mut Printer, from: u32, to: u32) {
    if from < to {
        let text = &printer.source[from as usize..to as usize];
        printer.write_with_mapping(text, from);
    }
}

pub(super) fn emit_open_tag(
    printer: &mut Printer,
    tag_name: &str,
    angle_start: u32,
    name_start: u32,
    attrs: &[AnyHtmlAttribute],
) {
    printer.map_to_offset(angle_start);
    printer.write("<");
    printer.map_to_offset(name_start);
    printer.write(tag_name);

    let mut invalid: Vec<&AnyHtmlAttribute> = Vec::new();

    let mut index = 0;
    while let Some(attr) = attrs.get(index) {
        if let Some(range) = recovered_v_for_range(printer.source, attrs, index) {
            printer.suppressed_html_diagnostics.push(SourceRange::new(
                range_start(attr.range()),
                u32::from(attr.range().end()),
            ));
            emit_vue_as_html_attribute(printer, range, None);
            index += 1;
            while attrs
                .get(index)
                .is_some_and(|attr| attr.range().start() < range.end())
            {
                index += 1;
            }
            continue;
        }
        if let Some(name) = attribute_key(attr)
            && !is_valid_tsx_attribute_name(&name)
        {
            invalid.push(attr);
            index += 1;
            continue;
        }
        emit_attribute(printer, attr);
        index += 1;
    }

    if !invalid.is_empty() {
        printer.map_nil();
        printer.write(" {...{");
        let mut wrote_entry = false;
        for attr in invalid {
            wrote_entry |= emit_invalid_attribute(printer, attr, wrote_entry);
        }
        printer.map_nil();
        printer.write("}}");
    }
}

fn recovered_v_for_range(
    source: &str,
    attrs: &[AnyHtmlAttribute],
    index: usize,
) -> Option<TextRange> {
    let current = attrs.get(index)?;
    let AnyHtmlAttribute::HtmlBogusAttribute(_) = current else {
        return None;
    };
    if slice_source(source, current.range()).trim() != "v-for=" {
        return None;
    }
    let next = attrs.get(index + 1)?;
    if current.range().end() != next.range().start()
        || !matches!(
            next,
            AnyHtmlAttribute::HtmlAttribute(_)
                | AnyHtmlAttribute::HtmlAttributeSingleTextExpression(_)
        )
    {
        return None;
    }
    let end = match next {
        AnyHtmlAttribute::HtmlAttributeSingleTextExpression(_) => next.range().end(),
        AnyHtmlAttribute::HtmlAttribute(_) => {
            let value_start = u32::from(current.range().end()) as usize;
            let value_len = reconstructed_v_for_value_len(&source[value_start..])?;
            TextSize::from((value_start + value_len) as u32)
        }
        _ => return None,
    };
    Some(TextRange::new(current.range().start(), end))
}

pub(super) fn reconstructed_v_for_source_range(text: &str, start: u32) -> Option<SourceRange> {
    let (attribute_start, _) = text.match_indices("v-for=").find(|(index, _)| {
        text[..*index]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
    })?;
    let value_start = attribute_start + "v-for=".len();
    let attribute_end = value_start + reconstructed_v_for_value_len(&text[value_start..])?;
    let trailing = text[attribute_end..].trim_start();
    let end = if matches!(trailing, ">" | "/>") {
        text.len()
    } else {
        attribute_end
    };
    Some(SourceRange::new(
        start + attribute_start as u32,
        start + end as u32,
    ))
}

fn reconstructed_v_for_value_len(value: &str) -> Option<usize> {
    match value.chars().next()? {
        delimiter @ ('"' | '\'') => {
            Some(delimiter.len_utf8() + value[delimiter.len_utf8()..].find(delimiter)? + 1)
        }
        '{' => None,
        _ => Some(
            value
                .find(|ch: char| ch.is_whitespace() || matches!(ch, '/' | '>'))
                .unwrap_or(value.len()),
        ),
    }
}

fn emit_attribute(printer: &mut Printer, attr: &AnyHtmlAttribute) {
    match attr {
        AnyHtmlAttribute::HtmlAttribute(attr_node) => emit_html_attribute(printer, attr_node),
        AnyHtmlAttribute::HtmlSpreadAttribute(spread) => emit_spread_attribute(printer, spread),
        AnyHtmlAttribute::HtmlAttributeSingleTextExpression(node) => {
            if let Ok(expression) = node.expression()
                && let Ok(token) = expression.html_literal_token()
            {
                let trimmed = token.text_trimmed().to_string();
                let original_start = range_start(token.text_trimmed_range());
                printer.map_nil();
                printer.write(" ");
                if trimmed.starts_with("/*") && trimmed.ends_with("*/") {
                    printer.write_with_mapping(&trimmed, original_start);
                } else {
                    printer.write_with_mapping(&trimmed, original_start);
                    printer.map_nil();
                    printer.write("={");
                    printer.write_with_mapping(&trimmed, original_start);
                    printer.map_nil();
                    printer.write("}");
                }
            }
        }
        AnyHtmlAttribute::AnyAstroDirective(directive) => emit_astro_directive(printer, directive),
        AnyHtmlAttribute::AnyVueDirective(directive) if directive.as_vue_directive().is_some() => {
            emit_vue_as_html_attribute(
                printer,
                directive.range(),
                directive
                    .as_vue_directive()
                    .and_then(|directive| directive.initializer()),
            );
        }
        AnyHtmlAttribute::HtmlBogusAttribute(attribute)
            if slice_source(printer.source, attribute.range())
                .trim_start()
                .starts_with("v-") =>
        {
            emit_vue_as_html_attribute(printer, attribute.range(), None);
        }
        AnyHtmlAttribute::AnyAngularBinding(_)
        | AnyHtmlAttribute::AngularStructuralDirective(_)
        | AnyHtmlAttribute::AngularTemplateRefVariable(_)
        | AnyHtmlAttribute::AnySvelteDirective(_)
        | AnyHtmlAttribute::AnyVueDirective(_)
        | AnyHtmlAttribute::HtmlAttributeDoubleTextExpression(_)
        | AnyHtmlAttribute::HtmlBogusAttribute(_)
        | AnyHtmlAttribute::SvelteAttachAttribute(_) => {
            // Foreign or recovery kinds; emitting them would produce invalid TSX.
        }
    }
}

fn emit_html_attribute(printer: &mut Printer, attr_node: &HtmlAttribute) {
    let Ok(name) = attr_node.name() else {
        return;
    };
    let Ok(name_token) = name.value_token() else {
        return;
    };
    let key_text = name_token.text_trimmed().to_string();
    let key_start = range_start(name_token.text_trimmed_range());

    printer.map_nil();
    printer.write(" ");
    printer.map_to_offset(key_start);
    printer.write(&key_text);

    emit_attribute_initializer(printer, &key_text, attr_node.initializer());
}

fn emit_vue_as_html_attribute(
    printer: &mut Printer,
    range: TextRange,
    initializer: Option<HtmlAttributeInitializerClause>,
) {
    let start = range_start(range);
    let raw = slice_source(printer.source, range).trim();

    printer
        .suppressed_html_diagnostics
        .push(SourceRange::new(start, u32::from(range.end())));
    printer.map_nil();
    printer.write(" ");

    if let Some(initializer) = initializer {
        emit_attribute_with_initializer(printer, range, start, initializer);
        return;
    }

    let Some((key, value)) = raw.split_once('=') else {
        printer.write_with_mapping(raw, start);
        return;
    };
    let key = key.trim_end();
    let value = value.trim_start();
    let eq_offset = start + raw.find('=').unwrap_or(key.len()) as u32;
    let value_offset = start + raw.len() as u32 - value.len() as u32;
    printer.write_with_mapping(key, start);
    printer.map_to_offset(eq_offset);
    printer.write("=");
    if strip_matching_quotes(value).is_some() {
        printer.write_with_mapping(value, value_offset);
    } else if let Some(inner) = value
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
    {
        emit_attribute_expression(printer, inner, value_offset + 1);
    } else {
        printer.map_nil();
        printer.write("\"");
        printer.write_attribute_value_with_mapping(value, value_offset);
        printer.map_nil();
        printer.write("\"");
    }
}

fn emit_attribute_with_initializer(
    printer: &mut Printer,
    range: TextRange,
    start: u32,
    initializer: HtmlAttributeInitializerClause,
) {
    let Ok(eq_token) = initializer.eq_token() else {
        return;
    };
    let key = slice_source(
        printer.source,
        TextRange::new(range.start(), eq_token.text_trimmed_range().start()),
    )
    .trim_end();
    printer.write_with_mapping(key, start);
    emit_attribute_initializer(printer, key, Some(initializer));
}

fn emit_attribute_initializer(
    printer: &mut Printer,
    key_text: &str,
    initializer: Option<HtmlAttributeInitializerClause>,
) {
    let Some(initializer) = initializer else {
        return;
    };
    let Ok(value) = initializer.value() else {
        return;
    };
    let Ok(eq_token) = initializer.eq_token() else {
        return;
    };
    let eq_start = range_start(eq_token.text_trimmed_range());

    match value {
        AnyHtmlAttributeInitializer::HtmlString(s) => {
            let Ok(value_token) = s.value_token() else {
                return;
            };
            let raw = value_token.text_trimmed().to_string();
            // The lexer files template-literal values under `HtmlString` too.
            if raw.starts_with('`') && raw.ends_with('`') && raw.len() >= 2 {
                let inner = &raw[1..raw.len() - 1];
                let value_start = range_start(value_token.text_trimmed_range()) + 1;
                printer.map_to_offset(eq_start);
                printer.write("=");
                printer.map_nil();
                printer.write("{");
                printer.map_to_offset(value_start - 1);
                printer.write("`");
                printer.write_with_mapping(inner, value_start);
                printer.map_to_offset(value_start + inner.len() as u32);
                printer.write("`");
                printer.map_nil();
                printer.write("}");
                return;
            }
            let token_start = range_start(value_token.text_trimmed_range());
            // Astro allows unquoted values, so the token may carry no quotes to strip.
            let quoted = strip_matching_quotes(&raw).is_some();
            let (inner, value_start) = if quoted {
                (&raw[1..raw.len() - 1], token_start + 1)
            } else {
                (raw.as_str(), token_start)
            };
            let inner_end = value_start + inner.len() as u32;

            printer.map_to_offset(eq_start);
            printer.write("=");
            let (generated_start, generated_end);
            if quoted {
                let token_generated_start = printer.position();
                printer.write_with_mapping(&raw, token_start);
                generated_start = token_generated_start + 1;
                generated_end = printer.position() - 1;
            } else {
                printer.map_nil();
                printer.write("\"");
                generated_start = printer.position();
                printer.write_attribute_value_with_mapping(inner, value_start);
                generated_end = printer.position();
                printer.map_nil();
                printer.write("\"");
            }

            let lower_key = key_text.to_ascii_lowercase();
            if is_html_event_attribute(&lower_key) {
                printer.add_event_attribute(
                    GeneratedRange::new(generated_start, generated_end),
                    SourceRange::new(value_start, inner_end),
                    inner.to_string(),
                );
            }
            if lower_key == "style" {
                printer.add_style_attribute(
                    GeneratedRange::new(generated_start, generated_end),
                    SourceRange::new(value_start, inner_end),
                    inner.to_string(),
                );
            }
        }
        AnyHtmlAttributeInitializer::HtmlAttributeSingleTextExpression(expr) => {
            let literal = expr
                .expression()
                .ok()
                .and_then(|e| e.html_literal_token().ok());
            printer.map_to_offset(eq_start);
            printer.write("=");
            match literal {
                Some(literal) if !literal.text_trimmed().trim().is_empty() => {
                    let value = literal.text_trimmed().to_string();
                    emit_attribute_expression(
                        printer,
                        &value,
                        range_start(literal.text_trimmed_range()),
                    );
                }
                _ => emit_attribute_expression(printer, "", eq_start + 1),
            }
        }
        AnyHtmlAttributeInitializer::SvelteTemplateAttributeValue(_)
        | AnyHtmlAttributeInitializer::VueVForValue(_) => {}
    }
}

fn emit_attribute_expression(printer: &mut Printer, value: &str, value_start: u32) {
    printer.map_nil();
    printer.write("{");
    if value.trim().is_empty() {
        printer.write("(void 0)");
    } else {
        emit_expression_body(printer, value, value_start);
    }
    printer.map_nil();
    printer.write("}");
}

fn emit_spread_attribute(printer: &mut Printer, spread: &HtmlSpreadAttribute) {
    let Ok(argument) = spread.argument() else {
        return;
    };
    let Ok(literal) = argument.html_literal_token() else {
        return;
    };
    let value = literal.text_trimmed().to_string();
    let value_start = range_start(literal.text_trimmed_range());

    printer.map_nil();
    printer.write(" {");
    printer.map_nil();
    printer.write("...");
    printer.write_with_mapping(&value, value_start);
    printer.map_nil();
    printer.write("}");
}

fn emit_astro_directive(printer: &mut Printer, directive: &AnyAstroDirective) {
    let range = directive.range();
    let start = range_start(range);
    let value = match directive {
        AnyAstroDirective::AstroIsDirective(directive) => directive.value(),
        AnyAstroDirective::AstroClientDirective(directive) => directive.value(),
        AnyAstroDirective::AstroClassDirective(directive) => directive.value(),
        AnyAstroDirective::AstroDefineDirective(directive) => directive.value(),
        AnyAstroDirective::AstroServerDirective(directive) => directive.value(),
        AnyAstroDirective::AstroSetDirective(directive) => directive.value(),
    };
    let initializer = value.ok().and_then(|value| value.initializer());

    printer.map_nil();
    printer.write(" ");
    if let Some(initializer) = initializer {
        emit_attribute_with_initializer(printer, range, start, initializer);
    } else {
        printer.write_with_mapping(slice_source(printer.source, range), start);
    }
}

/// A skipped entry must not advance comma insertion.
fn emit_invalid_attribute(
    printer: &mut Printer,
    attr: &AnyHtmlAttribute,
    needs_separator: bool,
) -> bool {
    let AnyHtmlAttribute::HtmlAttribute(attr_node) = attr else {
        return false;
    };
    let Ok(name) = attr_node.name() else {
        return false;
    };
    let Ok(name_token) = name.value_token() else {
        return false;
    };
    let key_text = name_token.text_trimmed().to_string();
    let key_start = range_start(name_token.text_trimmed_range());

    if needs_separator {
        printer.map_nil();
        printer.write(",");
    }
    printer.map_nil();
    printer.write("\"");
    printer.write_with_mapping(&key_text, key_start);
    printer.map_nil();
    printer.write("\"");

    match attr_node.initializer() {
        None => {
            printer.map_nil();
            printer.write(":true");
        }
        Some(initializer) => match initializer.value() {
            Ok(AnyHtmlAttributeInitializer::HtmlString(s)) => {
                let Ok(value_token) = s.value_token() else {
                    printer.map_nil();
                    printer.write(":true");
                    return true;
                };
                let raw = value_token.text_trimmed().to_string();
                let inner = strip_matching_quotes(&raw).unwrap_or(raw.as_str());
                printer.map_nil();
                printer.write(":");
                printer.map_nil();
                printer.write(&format!(
                    "\"{}\"",
                    escape_javascript_string(&decode_html_entities(inner))
                ));
            }
            Ok(AnyHtmlAttributeInitializer::HtmlAttributeSingleTextExpression(expr)) => {
                let Ok(text) = expr.expression() else {
                    printer.map_nil();
                    printer.write(":true");
                    return true;
                };
                let Ok(literal) = text.html_literal_token() else {
                    printer.map_nil();
                    printer.write(":true");
                    return true;
                };
                let value = literal.text_trimmed().to_string();
                let value_start = range_start(literal.text_trimmed_range());
                printer.map_nil();
                printer.write(":(");
                printer.write_with_mapping(&value, value_start);
                printer.map_nil();
                printer.write(")");
            }
            _ => {
                printer.map_nil();
                printer.write(":true");
            }
        },
    }
    true
}

pub(super) fn attribute_key(attr: &AnyHtmlAttribute) -> Option<String> {
    match attr {
        AnyHtmlAttribute::HtmlAttribute(attr_node) => {
            let attr_name = attr_node.name().ok()?;
            Some(attr_name.value_token().ok()?.text_trimmed().to_string())
        }
        AnyHtmlAttribute::AnyAstroDirective(directive) => {
            let prefix = match directive {
                AnyAstroDirective::AstroIsDirective(_) => "is",
                AnyAstroDirective::AstroClientDirective(_) => "client",
                AnyAstroDirective::AstroClassDirective(_) => "class",
                AnyAstroDirective::AstroDefineDirective(_) => "define",
                AnyAstroDirective::AstroServerDirective(_) => "server",
                AnyAstroDirective::AstroSetDirective(_) => "set",
            };
            let name = directive_value_name(directive)?;
            Some(format!("{prefix}:{name}"))
        }
        _ => None,
    }
}

fn directive_value_name(directive: &AnyAstroDirective) -> Option<String> {
    let value = match directive {
        AnyAstroDirective::AstroIsDirective(d) => d.value().ok()?,
        AnyAstroDirective::AstroClientDirective(d) => d.value().ok()?,
        AnyAstroDirective::AstroClassDirective(d) => d.value().ok()?,
        AnyAstroDirective::AstroDefineDirective(d) => d.value().ok()?,
        AnyAstroDirective::AstroServerDirective(d) => d.value().ok()?,
        AnyAstroDirective::AstroSetDirective(d) => d.value().ok()?,
    };
    let name = value.name().ok()?;
    Some(name.value_token().ok()?.text_trimmed().to_string())
}

#[cfg(test)]
mod tests {
    use biome_js_parser::{JsParserOptions, parse};
    use biome_languages::JsFileSource;

    use crate::test_utils::assert_mapped_runs_are_verbatim;
    use crate::{ConvertOptions, SourceRange, convert_to_tsx};

    #[test]
    fn valueless_expression_attribute_keeps_a_value() {
        let actual = convert_to_tsx("<div @click={} />", ConvertOptions::default()).code;
        assert!(actual.contains("{...{\"@click\":true}}"), "{actual}");
    }

    #[test]
    fn spread_object_entries_are_comma_separated_without_a_leading_comma() {
        let actual = convert_to_tsx("<div @click={} @other={} />", ConvertOptions::default()).code;
        assert!(
            actual.contains("{...{\"@click\":true,\"@other\":true}}"),
            "{actual}"
        );

        for input in [
            "<div client:load.foo @z={} />",
            "<div @z={} client:load.foo />",
        ] {
            let actual = convert_to_tsx(input, ConvertOptions::default()).code;
            assert!(
                actual.contains("{...{\"@z\":true}}"),
                "stray separator for {input:?}:\n{actual}"
            );
        }
    }

    #[test]
    fn astro_attribute_syntax_that_tsx_rejects_is_normalized() {
        for input in [
            r#"<Component set:html=`${content}` />"#,
            r#"<article set:text=`content` />"#,
            r#"<div class=`item-${id}` />"#,
            r#"<h1 {/* comment */} value="1">Hello</h1>"#,
            r#"<Component foo{value} />"#,
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
            assert_mapped_runs_are_verbatim(input, &result, "Astro attributes");
        }
    }

    #[test]
    fn single_quoted_attributes_round_trip_with_their_quotes() {
        let source = "<div data-x='a\"b' title='plain'></div>";
        let result = convert_to_tsx(source, ConvertOptions::default());
        assert!(result.code.contains("data-x='a\"b'"), "{}", result.code);
        assert_mapped_runs_are_verbatim(source, &result, "single quotes");
    }

    #[test]
    fn multiline_tag_headers_keep_their_whitespace() {
        let source = "<Comp\n  foo={bar}\n/>";
        let result = convert_to_tsx(source, ConvertOptions::default());
        assert!(result.code.contains("foo={bar}\n/>"), "{}", result.code);
        assert_mapped_runs_are_verbatim(source, &result, "multiline tag");
    }

    #[test]
    fn astro_hyphenated_attributes_are_not_vue_syntax() {
        for input in [
            "<div v-if />",
            "<div v-if=visible />",
            "<Component v-if={visible} />",
        ] {
            let result = convert_to_tsx(input, ConvertOptions::default());
            assert!(
                !result.has_parse_errors,
                "{input:?}: {:?}",
                result.diagnostics
            );
            assert!(result.diagnostics.is_empty(), "{input:?}");
            assert!(result.code.contains("v-if"), "{input:?}: {}", result.code);
        }
    }

    #[test]
    fn astro_hyphenated_attribute_expressions_are_validated() {
        let result = convert_to_tsx("<Component v-if={visible ==} />", ConvertOptions::default());
        assert!(result.has_parse_errors, "{}", result.code);
        assert!(!result.diagnostics.is_empty(), "{}", result.code);
    }

    #[test]
    fn v_for_attributes_preserve_their_complete_value() {
        for (input, expected) in [
            ("<div v-for=\"item in items\" />", "v-for=\"item in items\""),
            ("<div v-for=items />", "v-for=\"items\""),
            ("<div v-for={items} />", "v-for={items}"),
            ("<div v-for />", "v-for"),
            ("<Component v-for={items} />", "v-for={items}"),
        ] {
            let result = convert_to_tsx(input, ConvertOptions::default());
            assert!(
                !result.has_parse_errors,
                "{input:?}: {:?}",
                result.diagnostics
            );
            assert!(result.diagnostics.is_empty(), "{input:?}");
            assert!(result.code.contains(expected), "{input:?}: {}", result.code);
            assert!(!result.code.contains("v-for=\"\""), "{}", result.code);
            assert_mapped_runs_are_verbatim(input, &result, "v-for attribute");
        }

        let malformed = convert_to_tsx("<Component v-for={items ==} />", ConvertOptions::default());
        assert!(malformed.has_parse_errors, "{}", malformed.code);
        assert!(!malformed.diagnostics.is_empty(), "{}", malformed.code);
        assert!(
            malformed.code.contains("v-for={items ==}"),
            "{}",
            malformed.code
        );
    }

    #[test]
    fn reconstructed_v_for_suppresses_only_its_recovery_diagnostics() {
        for input in [
            "<div v-for=\"a&amp;b\" />",
            "<div v-for=a&amp;b />",
            "<Component v-for=\"a&amp;b\" />",
            "<Component v-for=a&amp;b />",
        ] {
            let result = convert_to_tsx(input, ConvertOptions::default());
            assert!(
                !result.has_parse_errors,
                "{input:?}: {:?}",
                result.diagnostics
            );
            assert!(
                result.diagnostics.is_empty(),
                "{input:?}: {:?}",
                result.diagnostics
            );
            assert!(result.code.contains("v-for=\"a&amp;b\""), "{}", result.code);
        }

        let malformed = "<div v-for=\"a&amp;b\" /";
        let result = convert_to_tsx(malformed, ConvertOptions::default());
        let v_for_end = malformed.find(" /").unwrap() as u32;
        assert!(result.has_parse_errors, "{}", result.code);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.source.start >= v_for_end),
            "{:?}",
            result.diagnostics
        );
        assert!(!result.diagnostics.is_empty());
    }

    #[test]
    fn reconstructed_v_for_expression_diagnostics_use_document_offsets() {
        let input = "<main><Component v-for={items ==} /></main>";
        let result = convert_to_tsx(input, ConvertOptions::default());
        let expected = input.find('}').unwrap() as u32;
        let diagnostic = result.diagnostics.last().unwrap();

        assert_eq!(
            diagnostic.source,
            SourceRange {
                start: expected,
                end: expected,
            }
        );
        assert_eq!(
            &input[diagnostic.source.start as usize..diagnostic.source.end as usize],
            ""
        );
    }

    #[test]
    fn reconstructed_v_for_uses_the_complete_expression_boundary() {
        for (input, expected) in [
            (
                r#"<Component v-for={{a: 1}} data-after="yes" />"#,
                "v-for={{a: 1}}",
            ),
            (
                r#"<Component v-for={items.map(x => ({x}))} data-after="yes" />"#,
                "v-for={items.map(x => ({x}))}",
            ),
            (
                r#"<Component v-for={fn("}")} data-after="yes" />"#,
                r#"v-for={fn("}")}"#,
            ),
            (
                r#"<Component v-for={`item-${items.map(x => ({x}))}`} data-after="yes" />"#,
                "v-for={`item-${items.map(x => ({x}))}`}",
            ),
            (
                r#"<Component v-for={items.map(/* } */ x => ({x}))} data-after="yes" />"#,
                "v-for={items.map(/* } */ x => ({x}))}",
            ),
            (
                r#"<Component v-for={items.filter(x => /}/.test(x))} data-after="yes" />"#,
                "v-for={items.filter(x => /}/.test(x))}",
            ),
        ] {
            let result = convert_to_tsx(input, ConvertOptions::default());
            assert!(
                !result.has_parse_errors,
                "{input:?}: {:?}",
                result.diagnostics
            );
            assert!(
                result.diagnostics.is_empty(),
                "{input:?}: {:?}",
                result.diagnostics
            );
            assert!(result.code.contains(expected), "{input:?}: {}", result.code);
            assert!(
                result.code.contains("data-after=\"yes\""),
                "{}",
                result.code
            );
            assert_mapped_runs_are_verbatim(input, &result, "complex v-for expression");
        }

        let malformed = r#"<Component v-for={items.map(x => ({x})} data-after="yes" />"#;
        let result = convert_to_tsx(malformed, ConvertOptions::default());
        assert!(result.has_parse_errors, "{}", result.code);
        assert!(!result.diagnostics.is_empty(), "{}", result.code);
        assert!(
            result.code.contains("v-for={items.map(x => ({x})}"),
            "{}",
            result.code
        );
        assert!(
            result.code.contains("data-after=\"yes\""),
            "{}",
            result.code
        );
        assert_mapped_runs_are_verbatim(malformed, &result, "malformed nested v-for expression");
    }

    #[test]
    fn invalid_attribute_string_values_are_javascript_escaped() {
        for input in [
            "<Component @event='quote\" slash\\ line\n\u{2028}\u{2029}\t' />",
            "{ok && <Component @event='quote\" slash\\ line\n\u{2028}\u{2029}\t' />}",
        ] {
            let actual = convert_to_tsx(input, ConvertOptions::default()).code;
            assert!(
                actual
                    .contains("{...{\"@event\":\"quote\\\" slash\\\\ line\\n\\u2028\\u2029\\t\"}}"),
                "{actual}"
            );
        }
    }

    #[test]
    fn invalid_attribute_string_values_are_html_decoded() {
        let encoded = "&NotEqualTilde; &#128; &#0; &#xD800; &#x110000; &copy &amp; &quot; &copycat &amp= &bogus;";
        let decoded = "≂̸ € � � � © & \\\" &copycat &amp= &bogus;";
        for input in [
            format!("<Component @event='{encoded}' />"),
            format!("{{ok && <Component @event='{encoded}' />}}"),
        ] {
            let result = convert_to_tsx(&input, ConvertOptions::default());
            assert!(
                result
                    .code
                    .contains(&format!("{{...{{\"@event\":\"{decoded}\"}}}}")),
                "{}",
                result.code
            );
            assert_mapped_runs_are_verbatim(&input, &result, "decoded attribute entities");
        }
    }
}
