use biome_html_syntax::{
    AnyAstroDirective, AnyHtmlAttribute, AnyHtmlAttributeInitializer, HtmlAttribute,
    HtmlAttributeInitializerClause, HtmlSpreadAttribute,
};
use biome_rowan::{AstNode, TextRange};

use crate::printer::{Printer, range_start};
use crate::types::{GeneratedRange, SourceRange};
use crate::utils::{
    decode_html_entities, escape_javascript_string, is_html_event_attribute,
    is_valid_tsx_attribute_name, strip_matching_quotes,
};

use super::element::emit_expression_body;
use super::text::slice_source;

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

    for attr in attrs {
        emit_attribute(printer, attr);
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
        AnyHtmlAttribute::AnyAngularBinding(_)
        | AnyHtmlAttribute::AngularStructuralDirective(_)
        | AnyHtmlAttribute::AngularTemplateRefVariable(_)
        | AnyHtmlAttribute::AnySvelteDirective(_)
        | AnyHtmlAttribute::AnyVueDirective(_)
        | AnyHtmlAttribute::HtmlAttributeDoubleTextExpression(_)
        | AnyHtmlAttribute::HtmlBogusAttribute(_)
        | AnyHtmlAttribute::SvelteAttachAttribute(_) => {
            // Foreign or recovery kinds; emitting them would produce invalid TSX.
            // TODO: Check if some of them are actually relevant
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

    emit_named_attribute(printer, &key_text, key_start, attr_node.initializer());
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
    emit_named_attribute(printer, key, start, Some(initializer));
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
            if raw.starts_with('`') && raw.ends_with('`') && raw.len() >= 2 {
                printer.map_to_offset(eq_start);
                printer.write("=");
                emit_attribute_expression(
                    printer,
                    &raw,
                    range_start(value_token.text_trimmed_range()),
                );
                return;
            }
            let token_start = range_start(value_token.text_trimmed_range());
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
        emit_expression_body(printer, value, value_start, true);
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
    emit_expression_body(printer, &value, value_start, true);
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

    if let Some(initializer) = initializer {
        emit_attribute_with_initializer(printer, range, start, initializer);
    } else {
        emit_named_attribute(printer, slice_source(printer.source, range), start, None);
    }
}

fn emit_named_attribute(
    printer: &mut Printer,
    key: &str,
    start: u32,
    initializer: Option<HtmlAttributeInitializerClause>,
) {
    printer.map_nil();
    printer.write(" ");
    if is_valid_tsx_attribute_name(key) {
        printer.write_with_mapping(key, start);
        emit_attribute_initializer(printer, key, initializer);
        return;
    }
    printer.write("{...{");
    printer.write_js_string_with_mapping(key, start);
    printer.write(":");
    match initializer.and_then(|initializer| initializer.value().ok()) {
        Some(AnyHtmlAttributeInitializer::HtmlString(string)) => {
            if let Ok(token) = string.value_token() {
                let raw = token.text_trimmed();
                if raw.starts_with('`') {
                    emit_expression_body(
                        printer,
                        raw,
                        range_start(token.text_trimmed_range()),
                        true,
                    );
                } else {
                    printer.write(&escape_javascript_string(&decode_html_entities(
                        strip_matching_quotes(raw).unwrap_or(raw),
                    )));
                }
            } else {
                printer.write("void 0");
            }
        }
        Some(AnyHtmlAttributeInitializer::HtmlAttributeSingleTextExpression(expression)) => {
            printer.write("(");
            let literal = expression
                .expression()
                .ok()
                .and_then(|text| text.html_literal_token().ok());
            if let Some(literal) = literal.filter(|token| !token.text_trimmed().trim().is_empty()) {
                emit_expression_body(
                    printer,
                    literal.text_trimmed(),
                    range_start(literal.text_trimmed_range()),
                    true,
                );
            } else {
                printer.write("void 0");
            }
            printer.map_nil();
            printer.write(")");
        }
        _ => printer.write("true"),
    }
    printer.map_nil();
    printer.write("}}");
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

    use crate::test_utils::{assert_mapped_runs_are_verbatim, convert};
    use crate::{ConvertOptions, SourceRange, convert_to_tsx};

    #[test]
    fn attributes_preserve_expressions_in_both_contexts() {
        for name in [
            "value",
            "@x",
            "a:b:c",
            "a:",
            "a:1",
            "a:-b",
            "client:load.foo",
        ] {
            for value in ["`${foo}`", "{ok && <br>}", "{<style>.x{color:red}</style>}"] {
                let markup = format!("<C {name}={value} />");
                for source in [markup.clone(), format!("{{{markup}}}")] {
                    let result = convert(&source);
                    if value.contains("foo") {
                        assert!(result.code.contains("`${foo}`"), "{}", result.code);
                    }
                    if value.contains("style") {
                        assert_eq!(result.styles.len(), 1, "{}", result.code);
                    }
                }
            }
        }
        for source in [
            "<C {...{ x: <br> }} />",
            "<C x=`${<br>}` />",
            "<C {...{ x: <style>.x{color:red}</style> }} />",
        ] {
            let result = convert(source);
            if source.contains("style") {
                assert_eq!(result.styles.len(), 1);
            }
        }
        for source in ["<C {...a ==} />", "<C @x={a ==} />", "<C x=`${a ==}` />"] {
            assert!(
                convert_to_tsx(source, ConvertOptions::default()).has_parse_errors,
                "{source}"
            );
        }
    }

    #[test]
    fn transformed_attribute_keys_are_escaped_and_ordered() {
        for markup in [
            r#"<C @foo\unicode="x" />"#,
            r#"<C @foo\bar="x" />"#,
            r#"<C @foo="bad" {...attrs} @bar="ok" />"#,
        ] {
            for source in [markup.to_string(), format!("{{{markup}}}")] {
                let result = convert(&source);
                if markup.contains("attrs") {
                    assert!(
                        result.code.find("@foo").unwrap() < result.code.find("...attrs").unwrap()
                    );
                    assert!(
                        result.code.find("...attrs").unwrap() < result.code.find("@bar").unwrap()
                    );
                } else {
                    assert!(result.code.contains("\\\\"));
                }
            }
        }
    }

    #[test]
    #[ignore = "requires Biome Astro JSX attribute recovery"]
    fn embedded_opening_tag_comments_and_empty_attributes_are_valid() {
        for source in [
            "{<C {/* comment */} />}",
            "{<C foo={} />}",
            "{<C :foo=\"bad\" {...attrs} :bar=\"ok\" />}",
        ] {
            convert(source);
        }
    }

    #[test]
    fn comment_only_attribute_values_are_undefined() {
        for name in ["foo", "@foo", "set:html"] {
            for value in ["/* comment */", "// comment\n", "/* first */ /* second */"] {
                let source = format!("<C {name}={{{value}}} />");
                let result = convert(&source);
                assert!(result.code.contains("void 0"), "{}", result.code);
                assert!(result.code.contains(value), "{}", result.code);
            }
        }
        for value in [
            "/* before */ value /* after */",
            "/* before */ /x/.test(value)",
        ] {
            let result = convert(&format!("<C foo={{{value}}} />"));
            assert!(!result.code.contains("void 0"), "{}", result.code);
        }
    }

    #[test]
    fn valueless_expression_attribute_keeps_a_value() {
        let actual = convert_to_tsx("<div @click={} />", ConvertOptions::default()).code;
        assert!(actual.contains("{...{\"@click\":(void 0)}}"), "{actual}");
    }

    #[test]
    fn transformed_attributes_preserve_each_entry() {
        let actual = convert_to_tsx("<div @click={} @other={} />", ConvertOptions::default()).code;
        assert!(
            actual.contains("{...{\"@click\":(void 0)}} {...{\"@other\":(void 0)}}"),
            "{actual}"
        );

        for input in [
            "<div client:load.foo @z={} />",
            "<div @z={} client:load.foo />",
        ] {
            let actual = convert_to_tsx(input, ConvertOptions::default()).code;
            assert!(
                actual.contains("{...{\"@z\":(void 0)}}"),
                "missing value for {input:?}:\n{actual}"
            );
            assert!(
                actual.contains("{...{\"client:load.foo\":true}}"),
                "{actual}"
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
    #[ignore = "requires Biome to parse v-* as ordinary Astro attributes"]
    fn vue_named_attributes_preserve_expressions_in_both_contexts() {
        for value in ["`${foo}`", "{ok && <br>}", "{<style>.x{color:red}</style>}"] {
            let markup = format!("<C v-on:click.stop={value} />");
            for source in [markup.clone(), format!("{{{markup}}}")] {
                let result = convert(&source);
                if value.contains("foo") {
                    assert!(result.code.contains("`${foo}`"), "{}", result.code);
                }
                if value.contains("style") {
                    assert_eq!(result.styles.len(), 1, "{}", result.code);
                }
            }
        }
    }

    #[test]
    #[ignore = "requires Biome to parse v-* as ordinary Astro attributes"]
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
    #[ignore = "requires Biome to parse v-* as ordinary Astro attributes"]
    fn astro_hyphenated_attribute_expressions_are_validated() {
        let result = convert_to_tsx("<Component v-if={visible ==} />", ConvertOptions::default());
        assert!(result.has_parse_errors, "{}", result.code);
        assert!(!result.diagnostics.is_empty(), "{}", result.code);
    }

    #[test]
    #[ignore = "requires Biome to parse v-* as ordinary Astro attributes"]
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
    #[ignore = "requires Biome to parse v-* as ordinary Astro attributes"]
    fn v_for_values_allow_entities_without_masking_malformed_tags() {
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
    #[ignore = "requires Biome to parse v-* as ordinary Astro attributes"]
    fn v_for_expression_diagnostics_use_document_offsets() {
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
    #[ignore = "requires Biome to parse v-* as ordinary Astro attributes"]
    fn v_for_uses_the_complete_expression_boundary() {
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
