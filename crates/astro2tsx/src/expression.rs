use biome_js_syntax::{
    AnyJsxAttribute, AnyJsxAttributeValue, AnyJsxElementName, AstroImplicitFragment, JsLanguage,
    JsSyntaxKind, JsxAttribute, JsxAttributeList, JsxElement, JsxFragment, JsxSelfClosingElement,
    JsxString, JsxText,
};
use biome_rowan::{AstNode, AstNodeList, SyntaxNode, SyntaxToken, SyntaxTriviaPiece, TextSize};

use crate::printer::Printer;
use crate::sourcemap::{ExtractedScriptType, GeneratedRange, SourceRange};
use crate::utils::{
    comment_needs_leading_space, decode_html_entities, escape_javascript_string,
    is_html_event_attribute, is_valid_tsx_attribute_name, strip_matching_quotes,
};

type JsNode = SyntaxNode<JsLanguage>;
type JsToken = SyntaxToken<JsLanguage>;
type JsTrivia = SyntaxTriviaPiece<JsLanguage>;

/// `root` ranges are body-relative; `base` shifts them to document offsets.
pub(crate) fn emit_expression_tree(printer: &mut Printer, root: &JsNode, base: u32) {
    emit_node(printer, root, base, false);
}

fn abs(base: u32, offset: TextSize) -> u32 {
    base + u32::from(offset)
}

fn emit_node(printer: &mut Printer, node: &JsNode, base: u32, in_children: bool) {
    match node.kind() {
        JsSyntaxKind::JSX_ELEMENT => {
            if let Some(element) = JsxElement::cast_ref(node) {
                emit_jsx_element(printer, &element, base);
                return;
            }
        }
        JsSyntaxKind::JSX_SELF_CLOSING_ELEMENT => {
            if let Some(element) = JsxSelfClosingElement::cast_ref(node) {
                emit_self_closing_element(printer, &element, base);
                return;
            }
        }
        JsSyntaxKind::JSX_FRAGMENT => {
            if let Some(fragment) = JsxFragment::cast_ref(node) {
                emit_jsx_fragment(printer, &fragment, base);
                return;
            }
        }
        JsSyntaxKind::ASTRO_IMPLICIT_FRAGMENT => {
            if let Some(fragment) = AstroImplicitFragment::cast_ref(node) {
                emit_implicit_fragment(printer, &fragment, base);
                return;
            }
        }
        JsSyntaxKind::JSX_TEXT => {
            if let Some(text) = JsxText::cast_ref(node) {
                emit_jsx_text(printer, &text, base);
                return;
            }
        }
        _ => {}
    }
    let child_context = in_children && node.kind() != JsSyntaxKind::JSX_EXPRESSION_CHILD;
    for child in node.children_with_tokens() {
        match child {
            biome_rowan::SyntaxElement::Node(child) => {
                emit_node(printer, &child, base, child_context)
            }
            biome_rowan::SyntaxElement::Token(token) => {
                emit_token(printer, &token, base, child_context)
            }
        }
    }
}

fn emit_token(printer: &mut Printer, token: &JsToken, base: u32, in_children: bool) {
    for piece in token.leading_trivia().pieces() {
        emit_trivia(printer, &piece, base, in_children);
    }
    printer.write_with_mapping(
        token.text_trimmed(),
        abs(base, token.text_trimmed_range().start()),
    );
    for piece in token.trailing_trivia().pieces() {
        emit_trivia(printer, &piece, base, in_children);
    }
}

fn emit_trivia(printer: &mut Printer, piece: &JsTrivia, base: u32, in_children: bool) {
    let text = piece.text();
    if piece.is_comments()
        && let Some(body) = text
            .strip_prefix("<!--")
            .and_then(|rest| rest.strip_suffix("-->"))
    {
        printer.map_nil();
        printer.write(if in_children { "{/**" } else { "/**" });
        if comment_needs_leading_space(body) {
            printer.write(" ");
        }
        printer.write_comment_body_with_mapping(body, abs(base, piece.text_range().start()) + 4);
        printer.map_nil();
        printer.write(if in_children { "*/}" } else { "*/" });
        return;
    }
    printer.write_with_mapping(text, abs(base, piece.text_range().start()));
}

fn emit_jsx_text(printer: &mut Printer, text: &JsxText, base: u32) {
    let Ok(token) = text.value_token() else {
        return;
    };
    for piece in token.leading_trivia().pieces() {
        emit_trivia(printer, &piece, base, true);
    }
    printer.write_jsx_text_with_mapping(
        token.text_trimmed(),
        abs(base, token.text_trimmed_range().start()),
    );
    for piece in token.trailing_trivia().pieces() {
        emit_trivia(printer, &piece, base, true);
    }
}

fn emit_jsx_fragment(printer: &mut Printer, fragment: &JsxFragment, base: u32) {
    if let Ok(opening) = fragment.opening_fragment() {
        emit_node(printer, opening.syntax(), base, true);
    }
    for child in fragment.children() {
        emit_node(printer, child.syntax(), base, true);
    }
    if let Ok(closing) = fragment.closing_fragment() {
        emit_node(printer, closing.syntax(), base, false);
    }
}

fn emit_implicit_fragment(printer: &mut Printer, fragment: &AstroImplicitFragment, base: u32) {
    printer.map_nil();
    printer.write("<Fragment>");
    for child in fragment.children() {
        emit_node(printer, child.syntax(), base, true);
    }
    printer.map_nil();
    printer.write("</Fragment>");
}

enum ChildrenMode {
    Normal,
    ExcludedScript,
    ExcludedStyle,
    RawTemplate,
}

fn children_mode(name: Option<&AnyJsxElementName>, attributes: &JsxAttributeList) -> ChildrenMode {
    // `<Script>` is a component reference, not the HTML element.
    if let Some(AnyJsxElementName::JsxName(intrinsic)) = name
        && let Ok(token) = intrinsic.value_token()
    {
        if token.text_trimmed().eq_ignore_ascii_case("script") {
            return ChildrenMode::ExcludedScript;
        }
        if token.text_trimmed().eq_ignore_ascii_case("style") {
            return ChildrenMode::ExcludedStyle;
        }
    }
    let is_raw = attributes
        .iter()
        .any(|attr| attribute_name_text(&attr).as_deref() == Some("is:raw"));
    if is_raw {
        ChildrenMode::RawTemplate
    } else {
        ChildrenMode::Normal
    }
}

fn emit_jsx_element(printer: &mut Printer, element: &JsxElement, base: u32) {
    let Ok(opening) = element.opening_element() else {
        emit_verbatim(printer, element.syntax(), base);
        return;
    };
    let mode = children_mode(opening.name().ok().as_ref(), &opening.attributes());

    if let Ok(l_angle) = opening.l_angle_token() {
        emit_token(printer, &l_angle, base, false);
    }
    if let Ok(name) = opening.name() {
        emit_verbatim(printer, name.syntax(), base);
    }
    emit_jsx_attributes(printer, &opening.attributes(), base);
    if let Ok(r_angle) = opening.r_angle_token() {
        for piece in r_angle.leading_trivia().pieces() {
            emit_trivia(printer, &piece, base, false);
        }
        printer.write_with_mapping(
            r_angle.text_trimmed(),
            abs(base, r_angle.text_trimmed_range().start()),
        );
        for piece in r_angle.trailing_trivia().pieces() {
            emit_trivia(printer, &piece, base, true);
        }
    }

    let body_start = printer.position();
    match mode {
        ChildrenMode::Normal => {
            for child in element.children() {
                emit_node(printer, child.syntax(), base, true);
            }
        }
        ChildrenMode::RawTemplate => {
            if let Some((from, to)) = children_source_span(element) {
                let raw = &printer.source[abs(base, from) as usize..abs(base, to) as usize];
                if !raw.is_empty() {
                    printer.map_nil();
                    printer.write("{`");
                    printer.write_template_text_with_mapping(raw, abs(base, from));
                    printer.map_nil();
                    printer.write("`}");
                }
            }
        }
        ChildrenMode::ExcludedScript | ChildrenMode::ExcludedStyle => {
            if let Some((from, to)) = children_source_span(element) {
                let (from, to) = (abs(base, from), abs(base, to));
                let content = printer.source[from as usize..to as usize].to_string();
                let range = GeneratedRange::new(body_start, printer.position());
                let source = SourceRange::new(from, to);
                if matches!(mode, ChildrenMode::ExcludedScript) {
                    let script_type = classify_script(&opening.attributes());
                    printer.add_script_block(range, source, content, script_type);
                } else {
                    let lang = jsx_attribute_value(&opening.attributes(), "lang")
                        .flatten()
                        .filter(|lang| !lang.is_empty())
                        .unwrap_or_else(|| "css".to_string());
                    printer.add_style_block(range, source, content, lang);
                }
            }
        }
    }

    if let Ok(closing) = element.closing_element() {
        let mut first = true;
        for child in closing.syntax().children_with_tokens() {
            match child {
                biome_rowan::SyntaxElement::Token(token) => {
                    // The `<` of `</tag>` carries children-position leading trivia.
                    emit_token(printer, &token, base, first);
                    first = false;
                }
                biome_rowan::SyntaxElement::Node(node) => {
                    emit_verbatim(printer, &node, base);
                    first = false;
                }
            }
        }
    }
}

fn children_source_span(element: &JsxElement) -> Option<(TextSize, TextSize)> {
    let start = element
        .opening_element()
        .ok()?
        .r_angle_token()
        .ok()?
        .text_trimmed_range()
        .end();
    let end = element
        .closing_element()
        .ok()?
        .l_angle_token()
        .ok()?
        .text_trimmed_range()
        .start();
    (start <= end).then_some((start, end))
}

fn emit_self_closing_element(printer: &mut Printer, element: &JsxSelfClosingElement, base: u32) {
    if let Ok(l_angle) = element.l_angle_token() {
        emit_token(printer, &l_angle, base, false);
    }
    if let Ok(name) = element.name() {
        emit_verbatim(printer, name.syntax(), base);
    }
    emit_jsx_attributes(printer, &element.attributes(), base);
    match element.slash_token() {
        Some(slash) => {
            emit_token(printer, &slash, base, false);
            if let Ok(r_angle) = element.r_angle_token() {
                emit_token(printer, &r_angle, base, false);
            }
        }
        // A void element parses without `/`, but TSX requires the slash.
        None => {
            if let Ok(r_angle) = element.r_angle_token() {
                for piece in r_angle.leading_trivia().pieces() {
                    emit_trivia(printer, &piece, base, false);
                }
                printer.map_nil();
                printer.write("/");
                printer.write_with_mapping(
                    r_angle.text_trimmed(),
                    abs(base, r_angle.text_trimmed_range().start()),
                );
                for piece in r_angle.trailing_trivia().pieces() {
                    emit_trivia(printer, &piece, base, false);
                }
            }
        }
    }
}

fn emit_verbatim(printer: &mut Printer, node: &JsNode, base: u32) {
    for child in node.children_with_tokens() {
        match child {
            biome_rowan::SyntaxElement::Node(child) => emit_verbatim(printer, &child, base),
            biome_rowan::SyntaxElement::Token(token) => emit_token(printer, &token, base, false),
        }
    }
}

fn attribute_name_text(attr: &AnyJsxAttribute) -> Option<String> {
    if let AnyJsxAttribute::JsxAttribute(attr) = attr {
        return Some(attr.name().ok()?.syntax().text_trimmed().to_string());
    }
    None
}

fn emit_jsx_attributes(printer: &mut Printer, attributes: &JsxAttributeList, base: u32) {
    let mut invalid: Vec<JsxAttribute> = Vec::new();

    for attr in attributes {
        match &attr {
            AnyJsxAttribute::JsxAttribute(attribute) => {
                let name = attribute_name_text(&attr).unwrap_or_default();
                if !name.is_empty() && !is_valid_tsx_attribute_name(&name) {
                    invalid.push(attribute.clone());
                    continue;
                }
                emit_plain_jsx_attribute(printer, attribute, base);
            }
            AnyJsxAttribute::JsxShorthandAttribute(shorthand) => {
                if let Ok(name) = shorthand.name()
                    && let Ok(token) = name.value_token()
                {
                    let text = token.text_trimmed().to_string();
                    let start = abs(base, token.text_trimmed_range().start());
                    if let Ok(l_curly) = shorthand.l_curly_token() {
                        for piece in l_curly.leading_trivia().pieces() {
                            emit_trivia(printer, &piece, base, false);
                        }
                    }
                    printer.write_with_mapping(&text, start);
                    printer.map_nil();
                    printer.write("={");
                    printer.write_with_mapping(&text, start);
                    printer.map_nil();
                    printer.write("}");
                }
            }
            AnyJsxAttribute::JsxSpreadAttribute(spread) => {
                emit_node(printer, spread.syntax(), base, false);
            }
            _ => {}
        }
    }

    if !invalid.is_empty() {
        printer.map_nil();
        printer.write(" {...{");
        let mut wrote_entry = false;
        for attribute in &invalid {
            wrote_entry |= emit_invalid_jsx_attribute(printer, attribute, base, wrote_entry);
        }
        printer.map_nil();
        printer.write("}}");
    }
}

fn emit_plain_jsx_attribute(printer: &mut Printer, attribute: &JsxAttribute, base: u32) {
    let name_text = attribute
        .name()
        .ok()
        .map(|name| name.syntax().text_trimmed().to_string());
    if let Ok(name) = attribute.name() {
        emit_verbatim(printer, name.syntax(), base);
    }
    let Some(initializer) = attribute.initializer() else {
        return;
    };
    if let Ok(eq) = initializer.eq_token() {
        emit_token(printer, &eq, base, false);
    }
    let Ok(value) = initializer.value() else {
        return;
    };
    match &value {
        AnyJsxAttributeValue::JsxString(string) => {
            let emitted = emit_jsx_string(printer, string, base);
            let Some((range, source, content)) = emitted else {
                return;
            };
            let Some(lower_name) = name_text.map(|name| name.to_ascii_lowercase()) else {
                return;
            };
            if is_html_event_attribute(&lower_name) {
                printer.add_event_attribute(range, source, content.clone());
            }
            if lower_name == "style" {
                printer.add_style_attribute(range, source, content);
            }
        }
        AnyJsxAttributeValue::JsxExpressionAttributeValue(expression) => {
            emit_node(printer, expression.syntax(), base, false);
        }
        AnyJsxAttributeValue::AnyJsxTag(tag) => {
            emit_node(printer, tag.syntax(), base, false);
        }
        // Astro's `attr=`t${x}`` needs braces to be a TSX attribute value.
        AnyJsxAttributeValue::JsTemplateExpression(template) => {
            printer.map_nil();
            printer.write("{");
            emit_node(printer, template.syntax(), base, false);
            printer.map_nil();
            printer.write("}");
        }
    }
}

fn emit_jsx_string(
    printer: &mut Printer,
    string: &JsxString,
    base: u32,
) -> Option<(GeneratedRange, SourceRange, String)> {
    let token = string.value_token().ok()?;
    let text = token.text_trimmed();
    let start = abs(base, token.text_trimmed_range().start());

    for piece in token.leading_trivia().pieces() {
        emit_trivia(printer, &piece, base, false);
    }
    let extracted = if let Some(inner) = strip_matching_quotes(text) {
        let generated_start = printer.position() + 1;
        printer.write_with_mapping(text, start);
        (
            GeneratedRange::new(generated_start, printer.position() - 1),
            SourceRange::new(start + 1, start + 1 + inner.len() as u32),
            inner.to_string(),
        )
    } else {
        printer.map_nil();
        printer.write("\"");
        let generated_start = printer.position();
        printer.write_attribute_value_with_mapping(text, start);
        let generated_end = printer.position();
        printer.map_nil();
        printer.write("\"");
        (
            GeneratedRange::new(generated_start, generated_end),
            SourceRange::new(start, start + text.len() as u32),
            text.to_string(),
        )
    };
    for piece in token.trailing_trivia().pieces() {
        emit_trivia(printer, &piece, base, false);
    }
    Some(extracted)
}

fn emit_invalid_jsx_attribute(
    printer: &mut Printer,
    attribute: &JsxAttribute,
    base: u32,
    needs_separator: bool,
) -> bool {
    let Ok(name) = attribute.name() else {
        return false;
    };
    let name_text = name.syntax().text_trimmed().to_string();
    let name_start = abs(base, name.syntax().text_trimmed_range().start());

    if needs_separator {
        printer.map_nil();
        printer.write(",");
    }
    printer.map_nil();
    printer.write("\"");
    printer.write_with_mapping(&name_text, name_start);
    printer.map_nil();
    printer.write("\"");
    printer.map_nil();
    printer.write(":");

    let value = attribute
        .initializer()
        .and_then(|initializer| initializer.value().ok());
    match value {
        Some(AnyJsxAttributeValue::JsxString(string)) => {
            if let Ok(token) = string.value_token() {
                let text = token.text_trimmed();
                let inner = strip_matching_quotes(text).unwrap_or(text);
                printer.map_nil();
                printer.write(&format!(
                    "\"{}\"",
                    escape_javascript_string(&decode_html_entities(inner))
                ));
            } else {
                printer.map_nil();
                printer.write("true");
            }
        }
        Some(AnyJsxAttributeValue::JsxExpressionAttributeValue(expression)) => {
            printer.map_nil();
            printer.write("(");
            if let Ok(inner) = expression.expression() {
                emit_node(printer, inner.syntax(), base, false);
            } else {
                printer.write("void 0");
            }
            printer.map_nil();
            printer.write(")");
        }
        _ => {
            printer.map_nil();
            printer.write("true");
        }
    }
    true
}

fn classify_script(attributes: &JsxAttributeList) -> ExtractedScriptType {
    if attributes.iter().next().is_none() {
        return ExtractedScriptType::ProcessedModule;
    }
    if attributes
        .iter()
        .any(|attr| attribute_name_text(&attr).as_deref() == Some("is:raw"))
    {
        return ExtractedScriptType::Raw;
    }
    crate::render::script_type_for_attr(jsx_attribute_value(attributes, "type"))
}

/// `None`: absent. `Some(None)`: present but not statically knowable.
/// `Some(Some(value))`: string value with any matching quotes stripped.
fn jsx_attribute_value(attributes: &JsxAttributeList, name: &str) -> Option<Option<String>> {
    for attr in attributes {
        let AnyJsxAttribute::JsxAttribute(attribute) = attr else {
            continue;
        };
        let Ok(attribute_name) = attribute.name() else {
            continue;
        };
        if attribute_name.syntax().text_trimmed() != name {
            continue;
        }
        let Some(initializer) = attribute.initializer() else {
            return Some(Some(String::new()));
        };
        if let Ok(AnyJsxAttributeValue::JsxString(string)) = initializer.value()
            && let Ok(token) = string.value_token()
        {
            let text = token.text_trimmed().to_string();
            let inner = strip_matching_quotes(&text).unwrap_or(&text);
            return Some(Some(inner.to_string()));
        }
        return Some(None);
    }
    None
}
