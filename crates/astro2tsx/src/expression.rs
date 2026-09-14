use biome_js_syntax::{
    AnyJsxAttribute, AnyJsxAttributeValue, AnyJsxElementName, AstroImplicitFragment, JsLanguage,
    JsSyntaxKind, JsxAttribute, JsxAttributeList, JsxElement, JsxFragment, JsxSelfClosingElement,
    JsxString, JsxText,
};
use biome_rowan::{
    AstNode, AstNodeList, SyntaxNode, SyntaxToken, SyntaxTriviaPiece, TextSize, WalkEvent,
};

use crate::printer::Printer;
use crate::types::{ExtractedScriptType, GeneratedRange, SourceRange};
use crate::utils::{
    BodyMode, body_mode, comment_needs_leading_space, decode_html_entities,
    escape_javascript_string, is_html_event_attribute, is_valid_tsx_attribute_name,
    strip_matching_quotes,
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
    let mut stack = vec![(biome_rowan::SyntaxElement::Node(node.clone()), in_children)];
    while let Some((element, in_children)) = stack.pop() {
        let node = match element {
            biome_rowan::SyntaxElement::Token(token) => {
                emit_token(printer, &token, base, in_children);
                continue;
            }
            biome_rowan::SyntaxElement::Node(node) => node,
        };
        if in_children
            && printer.expressions_disabled
            && node.kind() == JsSyntaxKind::JSX_EXPRESSION_CHILD
        {
            let range = node.text_range_with_trivia();
            let start = abs(base, range.start());
            let text = &printer.source[start as usize..abs(base, range.end()) as usize];
            printer.write_jsx_text_with_mapping(text, start);
            continue;
        }
        if emit_special_node(printer, &node, base) {
            continue;
        }
        let child_context = in_children && node.kind() != JsSyntaxKind::JSX_EXPRESSION_CHILD;
        let start = stack.len();
        stack.extend(
            node.children_with_tokens()
                .map(|child| (child, child_context)),
        );
        stack[start..].reverse();
    }
}

fn emit_special_node(printer: &mut Printer, node: &JsNode, base: u32) -> bool {
    match node.kind() {
        JsSyntaxKind::JSX_ELEMENT => {
            if let Some(element) = JsxElement::cast_ref(node) {
                emit_jsx_element(printer, &element, base);
                return true;
            }
        }
        JsSyntaxKind::JSX_SELF_CLOSING_ELEMENT => {
            if let Some(element) = JsxSelfClosingElement::cast_ref(node) {
                emit_self_closing_element(printer, &element, base);
                return true;
            }
        }
        JsSyntaxKind::JSX_FRAGMENT => {
            if let Some(fragment) = JsxFragment::cast_ref(node) {
                emit_jsx_fragment(printer, &fragment, base);
                return true;
            }
        }
        JsSyntaxKind::ASTRO_IMPLICIT_FRAGMENT => {
            if let Some(fragment) = AstroImplicitFragment::cast_ref(node) {
                emit_implicit_fragment(printer, &fragment, base);
                return true;
            }
        }
        JsSyntaxKind::JSX_TEXT => {
            if let Some(text) = JsxText::cast_ref(node) {
                emit_jsx_text(printer, &text, base);
                return true;
            }
        }
        _ => {}
    }
    false
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

fn children_mode(name: Option<&AnyJsxElementName>, attributes: &JsxAttributeList) -> BodyMode {
    // `<Script>` is a component reference, not the HTML element.
    let name = match name {
        Some(AnyJsxElementName::JsxName(intrinsic)) => intrinsic.value_token().ok(),
        _ => None,
    };
    let is_raw = attributes
        .iter()
        .any(|attr| attribute_name_text(&attr).as_deref() == Some("is:raw"));
    body_mode(name.as_ref().map(|token| token.text_trimmed()), is_raw)
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
        BodyMode::Normal | BodyMode::NoExpressions => {
            let disabled = printer.expressions_disabled;
            printer.expressions_disabled |= mode == BodyMode::NoExpressions;
            for child in element.children() {
                emit_node(printer, child.syntax(), base, true);
            }
            printer.expressions_disabled = disabled;
        }
        BodyMode::TextOnly => {
            for child in element.children() {
                let range = child.syntax().text_range_with_trivia();
                let mut cursor = abs(base, range.start());
                let mut walk = child.syntax().preorder();
                while let Some(event) = walk.next() {
                    let WalkEvent::Enter(node) = event else {
                        continue;
                    };
                    if node.kind() == JsSyntaxKind::JSX_EXPRESSION_CHILD {
                        let range = node.text_range_with_trivia();
                        let start = abs(base, range.start());
                        printer.write_jsx_text_with_mapping(
                            &printer.source[cursor as usize..start as usize],
                            cursor,
                        );
                        emit_node(printer, &node, base, true);
                        cursor = abs(base, range.end());
                        walk.skip_subtree();
                    }
                }
                printer.write_jsx_text_with_mapping(
                    &printer.source[cursor as usize..abs(base, range.end()) as usize],
                    cursor,
                );
            }
        }
        BodyMode::Raw => {
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
        BodyMode::Script | BodyMode::Style => {
            if let Some((from, to)) = children_source_span(element) {
                let (from, to) = (abs(base, from), abs(base, to));
                let content = printer.source[from as usize..to as usize].to_string();
                let range = GeneratedRange::new(body_start, printer.position());
                let source = SourceRange::new(from, to);
                if mode == BodyMode::Script {
                    let script_type = classify_script(&opening.attributes());
                    printer.add_script_block(range, source, content, script_type);
                } else {
                    let lang = crate::render::style_lang_for_attr(jsx_attribute_value(
                        &opening.attributes(),
                        "lang",
                    ));
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
    for token in node.descendants_tokens(biome_rowan::Direction::Next) {
        emit_token(printer, &token, base, false);
    }
}

fn attribute_name_text(attr: &AnyJsxAttribute) -> Option<String> {
    if let AnyJsxAttribute::JsxAttribute(attr) = attr {
        return Some(attr.name().ok()?.syntax().text_trimmed().to_string());
    }
    None
}

fn emit_jsx_attributes(printer: &mut Printer, attributes: &JsxAttributeList, base: u32) {
    for attr in attributes {
        match &attr {
            AnyJsxAttribute::JsxAttribute(attribute) => {
                let name = attribute_name_text(&attr).unwrap_or_default();
                if !name.is_empty() && !is_valid_tsx_attribute_name(&name) {
                    printer.map_nil();
                    printer.write(" {...{");
                    emit_invalid_jsx_attribute(printer, attribute, base);
                    printer.map_nil();
                    printer.write("}}");
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

fn emit_invalid_jsx_attribute(printer: &mut Printer, attribute: &JsxAttribute, base: u32) {
    let Ok(name) = attribute.name() else {
        return;
    };
    let name_text = name.syntax().text_trimmed().to_string();
    let name_start = abs(base, name.syntax().text_trimmed_range().start());

    printer.write_js_string_with_mapping(&name_text, name_start);
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
                printer.write(&escape_javascript_string(&decode_html_entities(inner)));
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
        Some(AnyJsxAttributeValue::JsTemplateExpression(template)) => {
            emit_node(printer, template.syntax(), base, false);
        }
        Some(AnyJsxAttributeValue::AnyJsxTag(tag)) => {
            emit_node(printer, tag.syntax(), base, false);
        }
        None => {
            printer.map_nil();
            printer.write("true");
        }
    }
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

/// `None`: absent. `Some(None)`: dynamic or malformed.
/// `Some(Some(value))`: entity-decoded static value, empty for a boolean attribute.
fn jsx_attribute_value(attributes: &JsxAttributeList, name: &str) -> Option<Option<String>> {
    for attr in attributes {
        let AnyJsxAttribute::JsxAttribute(attribute) = attr else {
            continue;
        };
        let Ok(attribute_name) = attribute.name() else {
            continue;
        };
        if !attribute_name
            .syntax()
            .text_trimmed()
            .to_string()
            .eq_ignore_ascii_case(name)
        {
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
            return Some(Some(decode_html_entities(inner).into_owned()));
        }
        return Some(None);
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::test_utils::convert;
    use crate::{ConvertOptions, convert_to_tsx};

    #[test]
    fn raw_element_references_are_not_javascript() {
        for markup in [
            "<math>{R}</math>",
            "<math><mi>{R}</mi></math>",
            "<textarea><Widget /></textarea>",
        ] {
            for source in [markup.to_string(), format!("{{{markup}}}")] {
                let result = convert(&source);
                assert!(!result.code.contains("{R}"), "{}", result.code);
                assert!(!result.code.contains("<Widget"), "{}", result.code);
            }
        }
    }

    #[test]
    fn unparseable_expression_bodies_are_not_silent() {
        let broken = convert_to_tsx("<div>{x ==}</div>", ConvertOptions::default());
        assert!(broken.has_parse_errors, "raw fallback must flag the result");

        let empty = convert_to_tsx("<div>{}</div>", ConvertOptions::default());
        assert!(!empty.has_parse_errors, "an empty expression is fine");
    }

    #[test]
    fn expression_string_literals_keep_their_value() {
        for (input, expected) in [
            ("<div>{\"<br/>\"}</div>", "{\"<br/>\"}"),
            ("<div>{'<b>y</b>'}</div>", "{'<b>y</b>'}"),
            ("<div>{`<b>y</b>`}</div>", "{`<b>y</b>`}"),
        ] {
            let actual = convert_to_tsx(input, ConvertOptions::default()).code;
            assert!(
                actual.contains(expected),
                "string value was rewritten for {input:?}:\n{actual}"
            );
        }
    }

    #[test]
    fn expression_generics_are_not_markup() {
        for (input, expected) in [
            ("<div>{foo<Bar>(x)}</div>", "{foo<Bar>(x)}"),
            ("<div>{a.b<C<D>>(y)}</div>", "{a.b<C<D>>(y)}"),
        ] {
            let actual = convert_to_tsx(input, ConvertOptions::default()).code;
            assert!(
                actual.contains(expected),
                "generics were treated as markup for {input:?}:\n{actual}"
            );
        }
    }

    #[test]
    fn only_adjacent_siblings_are_wrapped_in_a_fragment() {
        let adjacent = convert_to_tsx(
            "<div>{c && <span>a</span> <span>b</span>}</div>",
            ConvertOptions::default(),
        )
        .code;
        assert!(
            adjacent.contains("{c && <Fragment><span>a</span> <span>b</span></Fragment>}"),
            "adjacent siblings were not wrapped:\n{adjacent}"
        );

        for input in [
            "<div>{c && <span>a</span>}</div>",
            "<div>{l.map(i => <span>{i}</span>)}</div>",
        ] {
            let actual = convert_to_tsx(input, ConvertOptions::default()).code;
            assert!(
                !actual.contains("<Fragment><span"),
                "a lone element was wrapped for {input:?}:\n{actual}"
            );
        }
    }

    #[test]
    fn html_comments_inside_expressions_become_jsx_comments() {
        for input in [
            "{list.map(() => <Component><!--Hi--></Component>)}",
            "<div>{x && <span><!--hi--></span>}</div>",
        ] {
            let actual = convert_to_tsx(input, ConvertOptions::default()).code;
            assert!(
                !actual.contains("<!--"),
                "an html comment survived into TSX for {input:?}:\n{actual}"
            );
            assert!(
                actual.contains("{/**"),
                "no jsx comment emitted for {input:?}:\n{actual}"
            );
        }
    }
}
