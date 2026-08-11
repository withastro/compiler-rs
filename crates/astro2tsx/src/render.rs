//! Walks the Biome HTML CST for an Astro file and emits TSX.

use biome_html_syntax::{
    AnyAstroDirective, AnyAstroFrontmatterElement, AnyHtmlAttribute, AnyHtmlAttributeInitializer,
    AnyHtmlComponentObjectName, AnyHtmlContent, AnyHtmlElement, AnyHtmlTagName,
    AnyHtmlTextExpression, AstroFrontmatterElement, HtmlAttribute, HtmlElement, HtmlRoot,
    HtmlSelfClosingElement, HtmlSingleTextExpression, HtmlSpreadAttribute,
};
use biome_rowan::{AstNode, AstNodeList, TextRange};

use crate::ConvertOptions;
use crate::frontmatter::rewrite_top_level_returns;
use crate::printer::{Printer, range_start};
use crate::props::{PropsAnalysis, analyze as analyze_props};
use crate::sourcemap::GeneratedRange;
use crate::utils::{
    ScriptKind, classify_script_type, encode_double_quote, escape_braces, escape_text,
    is_html_event_attribute, is_valid_tsx_attribute_name, tsx_component_name,
};

const TSX_PREFIX: &str = "/* @jsxImportSource astro */\n\n";

pub(crate) fn render_root(printer: &mut Printer, root: HtmlRoot, options: &ConvertOptions) {
    printer.map_nil();
    printer.write(TSX_PREFIX);

    let frontmatter = root.frontmatter();
    let body = root.html();

    let props_analysis = frontmatter
        .as_ref()
        .and_then(extract_frontmatter_text)
        .map(|s| analyze_props(&s))
        .unwrap_or_default();

    if let Some(frontmatter) = &frontmatter {
        emit_frontmatter(printer, frontmatter);
    }

    // The body is non-empty whenever there's a parsed child OR the source
    // contains a top-level HTML comment that the parser stashed as trivia.
    let body_text_start = body_text_start_offset(&frontmatter);
    let body_text_end = printer.source.len() as u32;
    let has_body_children = body.iter().next().is_some()
        || slice_has_html_comment(&printer.source, body_text_start, body_text_end);
    let body_start;

    if has_body_children {
        if frontmatter.is_some() {
            // Without a `;`, the body's `<Fragment>` parses as a continuation
            // of the frontmatter's last expression (`Astro.props < Fragment >`).
            if needs_trailing_separator(&printer.output) {
                printer.map_nil();
                printer.write("{};");
            }
        }
        printer.map_nil();
        printer.write("<Fragment>\n");
        body_start = printer.position();

        let mut prev_end: Option<u32> = None;
        let body_only_has_comments = body.iter().next().is_none();
        for element in body.iter() {
            let element_range = element.range();
            let element_start = u32::from(element_range.start());
            if let Some(prev) = prev_end {
                emit_source_gap(printer, prev, element_start);
            }
            render_element(printer, element);
            prev_end = Some(u32::from(element_range.end()));
        }

        if body_only_has_comments {
            emit_source_gap(printer, body_text_start, printer.source.len() as u32);
        } else if let Some(prev) = prev_end {
            emit_source_gap(printer, prev, printer.source.len() as u32);
        }
        // One byte past EOF, so trailing whitespace still has a mapping.
        printer.map_to_offset(printer.source.len() as u32);
        printer.write("\n");

        let body_end = printer.position();
        printer.record_body_range(GeneratedRange::new(body_start, body_end));

        printer.map_nil();
        printer.write("</Fragment>\n");
    } else {
        // Stands in for the trailing newline a body would have provided.
        if frontmatter.is_some() {
            printer.map_nil();
            printer.write("\n");
        }
        printer.record_body_range(GeneratedRange::new(printer.position(), printer.position()));
    }

    let component_name = tsx_component_name(options.filename.as_deref());
    emit_default_export(printer, &component_name, &props_analysis);
}

/// Returns the byte offset where body content begins. With a frontmatter
/// present, this is the byte just after the closing `---` fence; without
/// it, this is the start of the source.
fn body_text_start_offset(frontmatter: &Option<AnyAstroFrontmatterElement>) -> u32 {
    if let Some(AnyAstroFrontmatterElement::AstroFrontmatterElement(node)) = frontmatter
        && let Ok(r_fence) = node.r_fence_token()
    {
        return u32::from(r_fence.text_trimmed_range().end());
    }
    0
}

fn extract_frontmatter_text(node: &AnyAstroFrontmatterElement) -> Option<String> {
    if let AnyAstroFrontmatterElement::AstroFrontmatterElement(frontmatter) = node
        && let Ok(content) = frontmatter.content()
        && let Some(token) = content.content_token()
    {
        return Some(token.text_trimmed().to_string());
    }
    None
}

fn emit_default_export(printer: &mut Printer, component: &str, analysis: &PropsAnalysis) {
    // `_props` takes the parameterised `Props<T>`; the Astro global takes the bare ident.
    let (props_param, props_global) = if analysis.has_props {
        if analysis.generics_args.is_empty() {
            ("Props".to_string(), "Props".to_string())
        } else {
            (
                format!("Props{}", analysis.generics_args),
                "Props".to_string(),
            )
        }
    } else if analysis.has_get_static_paths {
        let inferred = "ASTRO__MergeUnion<ASTRO__Get<ASTRO__InferredGetStaticPath, 'props'>>";
        (inferred.to_string(), inferred.to_string())
    } else {
        (
            "Record<string, any>".to_string(),
            "Record<string, any>".to_string(),
        )
    };

    let generics = if analysis.has_props {
        analysis.generics_decl.as_str()
    } else {
        ""
    };

    printer.write(&format!(
        "export default function {component}{generics}(_props: {props_param}): any {{}}\n"
    ));

    if analysis.has_get_static_paths {
        printer.write(
            "type ASTRO__ArrayElement<ArrayType extends readonly unknown[]> = ArrayType extends readonly (infer ElementType)[] ? ElementType : never;\n",
        );
        printer.write(
            "type ASTRO__Flattened<T> = T extends Array<infer U> ? ASTRO__Flattened<U> : T;\n",
        );
        printer.write("type ASTRO__InferredGetStaticPath = ASTRO__Flattened<ASTRO__ArrayElement<Awaited<ReturnType<typeof getStaticPaths>>>>;\n");
        printer.write("type ASTRO__MergeUnion<T, K extends PropertyKey = T extends unknown ? keyof T : never> = T extends unknown ? T & { [P in Exclude<K, keyof T>]?: never } extends infer O ? { [P in keyof O]: O[P] } : never : never;\n");
        printer.write("type ASTRO__Get<T, K> = T extends undefined ? undefined : K extends keyof T ? T[K] : never;\n");
    }

    if analysis.has_props || analysis.has_get_static_paths {
        printer.write(
            "/**\n * Astro global available in all contexts in .astro files\n *\n * [Astro documentation](https://docs.astro.build/reference/api-reference/#astro-global)\n*/\n",
        );
        printer.write(&format!(
            "declare const Astro: Readonly<import('astro').AstroGlobal<{props_global}, typeof {component}"
        ));
        if analysis.has_get_static_paths {
            printer.write(", ASTRO__Get<ASTRO__InferredGetStaticPath, 'params'>");
        }
        printer.write(">>");
    }
}

fn needs_trailing_separator(output: &str) -> bool {
    let trimmed = output.trim_end();
    if trimmed.len() <= TSX_PREFIX.len() + 1 {
        return false;
    }
    !trimmed.ends_with(';')
}

fn emit_frontmatter(printer: &mut Printer, frontmatter: &AnyAstroFrontmatterElement) {
    let frontmatter_start = printer.position();

    match frontmatter {
        AnyAstroFrontmatterElement::AstroFrontmatterElement(node) => {
            emit_frontmatter_element(printer, node);
        }
        AnyAstroFrontmatterElement::AstroBogusFrontmatter(_) => {
            // Recovery node: every byte maps to offset 0, not to its own position.
            printer.map_to_offset(0);
            printer.write(frontmatter.syntax().text_trimmed().to_string().as_str());
        }
    }

    let frontmatter_end = printer.position();
    printer.record_frontmatter_range(GeneratedRange::new(frontmatter_start, frontmatter_end));
}

fn emit_frontmatter_element(printer: &mut Printer, node: &AstroFrontmatterElement) {
    printer.map_to_offset(0);
    if let Ok(content) = node.content()
        && let Some(content_token) = content.content_token()
    {
        let range = content_token.text_range();
        let text = slice_source(&printer.source, range);
        if !text.is_empty() {
            let rewritten = rewrite_top_level_returns(&text);
            printer.write_with_mapping(&rewritten, range_start(range));
        }
    }
    // Separates the frontmatter from the body that follows.
    printer.map_nil();
    printer.write("\n");
}

fn render_element(printer: &mut Printer, element: AnyHtmlElement) {
    match element {
        AnyHtmlElement::AnyHtmlContent(content) => render_content(printer, content),
        AnyHtmlElement::HtmlElement(node) => render_html_element(printer, node),
        AnyHtmlElement::HtmlSelfClosingElement(node) => {
            render_self_closing_element(printer, node);
        }
        AnyHtmlElement::HtmlCdataSection(_)
        | AnyHtmlElement::HtmlProcessingInstruction(_)
        | AnyHtmlElement::HtmlBogusElement(_) => {
            // CDATA, processing instructions, and bogus nodes are passed
            // through verbatim. They occur rarely in `.astro` and produce
            // JSX-illegal output regardless, so the safest action is to
            // preserve the original text and let downstream tooling surface
            // diagnostics.
            let range = element.range();
            let text = slice_source(&printer.source, range);
            printer.write_with_mapping(&text, range_start(range));
        }
    }
}

fn render_content(printer: &mut Printer, content: AnyHtmlContent) {
    match content {
        AnyHtmlContent::HtmlContent(node) => {
            let Ok(token) = node.value_token() else {
                return;
            };
            // Use the trimmed range so trailing whitespace stays in the
            // gap between sibling nodes — the parent's emit_source_gap
            // will emit it once. Otherwise the trivia would double up.
            let range = token.text_trimmed_range();
            let text = token.text_trimmed();
            printer.write_jsx_text_with_mapping(text, range_start(range));
        }
        AnyHtmlContent::HtmlEmbeddedContent(node) => {
            let Ok(token) = node.value_token() else {
                return;
            };
            let original_start = u32::from(token.text_trimmed_range().start());
            let raw = token.text_trimmed().to_string();
            let escaped = escape_text(&raw);
            printer.map_nil();
            printer.write("{`");
            printer.write_with_mapping(&escaped, original_start);
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
            // `{{ … }}` is Vue's syntax — Astro doesn't use it, but we
            // accept it for HTML mode by passing through the raw text.
            let range = node.range();
            let text = slice_source(&printer.source, range);
            printer.write_with_mapping(&text, range_start(range));
        }
        AnyHtmlTextExpression::HtmlBogusTextExpression(node) => {
            let range = node.range();
            let text = slice_source(&printer.source, range);
            printer.write_with_mapping(&text, range_start(range));
        }
        AnyHtmlTextExpression::AnySvelteBlock(_) => {
            // Not relevant for Astro.
        }
    }
}

fn render_single_text_expression(printer: &mut Printer, node: HtmlSingleTextExpression) {
    let Ok(l_curly) = node.l_curly_token() else {
        return;
    };
    let Ok(expression) = node.expression() else {
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
        // The parser stores a `{...}` body as one token, so embedded JSX has to
        // be found by hand; bare JSX in a JS expression parses as a comparison.
        let original_start = u32::from(l_curly_range.end());
        // Whitespace touching `{` is trivia, which the literal token drops.
        let body = slice_source(
            &printer.source,
            TextRange::new(l_curly_range.end(), r_curly_range.start()),
        );
        let raw = body.as_str();
        if raw.is_empty() {
            printer.map_nil();
            printer.write("(void 0)");
        } else if let Some(jsx_range) = scan_jsx_span(raw) {
            let (jsx_start, jsx_end) = jsx_range;
            printer.write_with_mapping(&raw[..jsx_start], original_start);
            printer.map_nil();
            printer.write("<Fragment>");
            printer.write_with_mapping(&raw[jsx_start..jsx_end], original_start + jsx_start as u32);
            printer.map_nil();
            printer.write("</Fragment>");
            printer.write_with_mapping(&raw[jsx_end..], original_start + jsx_end as u32);
        } else {
            printer.write_with_mapping(raw, original_start);
        }
    } else {
        printer.map_nil();
        printer.write("(void 0)");
    }

    printer.map_to_offset(range_start(r_curly_range));
    printer.write("}");
}

/// Locates the first JSX tag in an expression body and returns its byte span in
/// the input slice. JS line and block comments are skipped; string and template
/// literals are not, so a tag inside a quoted value is treated as JSX.
fn scan_jsx_span(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = i.saturating_add(2);
            continue;
        }
        if bytes[i] == b'<' && i + 1 < len && (bytes[i + 1].is_ascii_alphabetic()) {
            return jsx_end_offset(bytes, i).map(|end| (i, end));
        }
        i += 1;
    }
    None
}

/// Given a `<` at `start`, returns the offset just past the matching
/// closing tag (for `<Foo>...</Foo>`) or self-closing `>` (for `<Foo/>`).
fn jsx_end_offset(bytes: &[u8], start: usize) -> Option<usize> {
    let len = bytes.len();
    let mut i = start;
    let mut depth = 0i32;
    while i < len {
        if bytes[i] == b'<' {
            if i + 1 < len && bytes[i + 1] == b'/' {
                while i < len && bytes[i] != b'>' {
                    i += 1;
                }
                if i >= len {
                    return None;
                }
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return Some(i);
                }
            } else {
                let mut self_closing = false;
                while i < len && bytes[i] != b'>' {
                    if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'>' {
                        self_closing = true;
                        i += 1;
                        break;
                    }
                    // Skip over quoted attribute values to avoid `<` /
                    // `>` inside a string confusing the scanner.
                    if bytes[i] == b'"' || bytes[i] == b'\'' {
                        let quote = bytes[i];
                        i += 1;
                        while i < len && bytes[i] != quote {
                            i += 1;
                        }
                    }
                    i += 1;
                }
                if i >= len {
                    return None;
                }
                if self_closing {
                    i += 1; // consume `>`
                    if depth == 0 {
                        return Some(i);
                    }
                } else {
                    depth += 1;
                    i += 1;
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

fn render_html_element(printer: &mut Printer, node: HtmlElement) {
    let Ok(opening) = node.opening_element() else {
        return;
    };
    if opening.name().is_err() {
        // Fragment shorthand: `<>...</>`, which JSX accepts as `Fragment`.
        render_fragment_shorthand(printer, &node);
        return;
    }
    let Ok(name) = opening.name() else {
        return;
    };
    let Ok(open_l_angle) = opening.l_angle_token() else {
        return;
    };
    let open_r_angle = opening.r_angle_token().ok();

    // An open tag with no `>` is round-tripped verbatim rather than shimmed;
    // the result is invalid TSX within that span alone.
    if open_r_angle.is_none() {
        let range = node.syntax().text_trimmed_range();
        let text = slice_source(&printer.source, range);
        printer.write_with_mapping(&text, range_start(range));
        return;
    }

    let tag_name = tag_name_text(&name);
    let attributes = opening.attributes();

    emit_open_tag(
        printer,
        &tag_name,
        u32::from(open_l_angle.text_trimmed_range().start()),
        u32::from(name.range().start()),
        attributes.iter(),
    );

    let open_r_angle = open_r_angle.expect("checked above");
    let r_angle_start = u32::from(open_r_angle.text_trimmed_range().start());
    let attrs_end = open_tag_attrs_end(&name, &attributes);
    emit_intra_tag_space(printer, attrs_end, r_angle_start);
    printer.map_to_offset(r_angle_start);
    printer.write(">");

    let is_script = tag_name.eq_ignore_ascii_case("script");
    let is_style = tag_name.eq_ignore_ascii_case("style");

    let children = node.children();
    let body_start = printer.position();

    let opening_end = u32::from(open_r_angle.text_trimmed_range().end());
    let attrs_for_classify = attributes_to_iter(&node);
    let element_is_raw = attrs_for_classify
        .iter()
        .any(|a| attribute_key(a).as_deref() == Some("is:raw"));
    let script_label = if is_script {
        Some(classify_script_label(&attrs_for_classify))
    } else {
        None
    };
    // `<script is:raw>` stays executable JS, so `is:raw` only wraps non-scripts.
    let wrap_children_as_template = (element_is_raw && !is_script)
        || matches!(script_label, Some("json" | "unknown"))
        || is_style;

    let closing_inner_start = node
        .closing_element()
        .ok()
        .and_then(|c| c.l_angle_token().ok())
        .map(|t| u32::from(t.text_trimmed_range().start()));

    if wrap_children_as_template {
        // Unclosed raw-text content runs to the end of the node the parser built.
        let inner_start = opening_end;
        let inner_end = closing_inner_start.unwrap_or_else(|| u32::from(node.range().end()));
        if inner_end > inner_start {
            let raw = printer.source[inner_start as usize..inner_end as usize].to_string();
            let escaped = escape_text(&raw);
            printer.map_nil();
            printer.write("{`");
            printer.write_with_mapping(&escaped, inner_start);
            printer.map_nil();
            printer.write("`}");
        }
    } else {
        let mut prev_end: Option<u32> = None;
        for child in children.iter() {
            let child_range = child.range();
            let child_start = u32::from(child_range.start());
            let leading_from = prev_end.unwrap_or(opening_end);
            emit_source_gap(printer, leading_from, child_start);
            if is_script {
                render_script_child(printer, child);
            } else {
                render_element(printer, child);
            }
            prev_end = Some(u32::from(child_range.end()));
        }
        if let Some(trailing_to) = closing_inner_start {
            let leading_from = prev_end.unwrap_or(opening_end);
            emit_source_gap(printer, leading_from, trailing_to);
        }
    }

    let body_end = printer.position();

    if is_script || is_style {
        // Capture the inner text range relative to the *original* source for
        // downstream consumers — `tsxRanges.scripts[].position` etc.
        let inner_range = inner_range(&node);
        if let Some((start, end)) = inner_range {
            let content = slice_source(&printer.source, TextRange::new(start.into(), end.into()));
            if is_script {
                printer.add_script_block(
                    GeneratedRange::new(body_start, body_end),
                    content,
                    classify_script_label(&attributes_to_iter(&node)),
                );
            } else {
                printer.add_style_block(
                    GeneratedRange::new(body_start, body_end),
                    content,
                    style_lang_label(&attributes_to_iter(&node)),
                );
            }
        }
    }

    if let Ok(closing) = node.closing_element() {
        let l_angle = closing.l_angle_token().ok();
        let r_angle = closing.r_angle_token().ok();
        let closing_name = closing.name().ok();

        if let Some(l_angle) = l_angle {
            printer.map_to_offset(range_start(l_angle.text_trimmed_range()));
        } else {
            printer.map_nil();
        }
        printer.write("</");

        if let Some(closing_name) = closing_name {
            printer.map_to_offset(range_start(closing_name.range()));
            printer.write(&tag_name_text(&closing_name));
        } else {
            printer.write(&tag_name);
        }

        if let Some(r_angle) = r_angle {
            printer.map_to_offset(range_start(r_angle.text_trimmed_range()));
        } else {
            printer.map_nil();
        }
        printer.write(">");
    }
    // Unclosed tags are unsupported, so no synthetic `</tag>` is emitted.
}

fn attributes_to_iter(node: &HtmlElement) -> Vec<AnyHtmlAttribute> {
    let Ok(opening) = node.opening_element() else {
        return Vec::new();
    };
    opening.attributes().iter().collect()
}

fn classify_script_label(attrs: &[AnyHtmlAttribute]) -> &'static str {
    let type_value = find_attr_value(attrs, "type");
    let is_raw = attrs
        .iter()
        .any(|a| attribute_key(a).as_deref() == Some("is:raw"));
    if is_raw {
        return "raw";
    }
    match classify_script_type(type_value.as_deref()) {
        ScriptKind::Script => {
            if matches!(type_value.as_deref(), Some("module")) {
                "module"
            } else if type_value.is_some() {
                "inline"
            } else {
                "processed-module"
            }
        }
        ScriptKind::Json => "json",
        ScriptKind::Unknown => "unknown",
    }
}

fn style_lang_label(attrs: &[AnyHtmlAttribute]) -> String {
    find_attr_value(attrs, "lang").unwrap_or_else(|| "css".to_string())
}

fn find_attr_value(attrs: &[AnyHtmlAttribute], name: &str) -> Option<String> {
    for attr in attrs {
        let AnyHtmlAttribute::HtmlAttribute(attr_node) = attr else {
            continue;
        };
        let Ok(attr_name) = attr_node.name() else {
            continue;
        };
        let Ok(token) = attr_name.value_token() else {
            continue;
        };
        if token.text_trimmed() != name {
            continue;
        }
        let Some(initializer) = attr_node.initializer() else {
            return Some(String::new());
        };
        let Ok(value) = initializer.value() else {
            continue;
        };
        if let AnyHtmlAttributeInitializer::HtmlString(s) = value
            && let Ok(value_token) = s.value_token()
        {
            let raw = value_token.text_trimmed().to_string();
            return Some(raw.trim_matches(|c| c == '"' || c == '\'').to_string());
        }
    }
    None
}

fn attribute_key(attr: &AnyHtmlAttribute) -> Option<String> {
    match attr {
        AnyHtmlAttribute::HtmlAttribute(attr_node) => {
            let attr_name = attr_node.name().ok()?;
            Some(attr_name.value_token().ok()?.text_trimmed().to_string())
        }
        AnyHtmlAttribute::AnyAstroDirective(directive) => {
            // Reconstruct the `kind:name` form (`is:raw`, `client:load`).
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

fn render_script_child(printer: &mut Printer, child: AnyHtmlElement) {
    if let AnyHtmlElement::AnyHtmlContent(AnyHtmlContent::HtmlEmbeddedContent(node)) = &child
        && let Ok(token) = node.value_token()
    {
        // Wrapped in an arrow body so TSX type-checks the script.
        let original_start = u32::from(token.text_trimmed_range().start());
        let raw = token.text_trimmed().to_string();
        printer.map_nil();
        printer.write("\n{() => {");
        printer.write_with_mapping(&raw, original_start);
        printer.map_nil();
        printer.write("}}\n");
        return;
    }
    render_element(printer, child);
}

fn render_self_closing_element(printer: &mut Printer, node: HtmlSelfClosingElement) {
    let Ok(name) = node.name() else {
        return;
    };
    let Ok(l_angle) = node.l_angle_token() else {
        return;
    };
    let Ok(r_angle) = node.r_angle_token() else {
        return;
    };
    let attributes = node.attributes();
    let tag_name = tag_name_text(&name);

    emit_open_tag(
        printer,
        &tag_name,
        u32::from(l_angle.text_trimmed_range().start()),
        u32::from(name.range().start()),
        attributes.iter(),
    );

    // Emit any whitespace trivia between the last attribute (or tag name)
    // and the `/>` so `<Foo />` and `<Foo/>` are both preserved. We look
    // *before* the slash, since the slash itself sits between the
    // whitespace and the `>`.
    let r_angle_start = u32::from(r_angle.text_trimmed_range().start());
    let pre_slash = node
        .slash_token()
        .map(|s| u32::from(s.text_trimmed_range().start()))
        .unwrap_or(r_angle_start);
    let trivia_start = self_closing_attrs_end(&node);
    if trivia_start < pre_slash {
        let slice_bytes = &printer.source[trivia_start as usize..pre_slash as usize];
        if slice_bytes.chars().any(|c| c.is_whitespace()) {
            printer.map_to_offset(trivia_start);
            printer.write(" ");
        }
    }
    printer.map_to_offset(r_angle_start);
    printer.write("/>");
}

/// Renders an `HtmlElement` whose tag name is missing — JSX's Fragment
/// shorthand `<>...</>`. We emit literal `<>` / `</>` framing and render
/// children inside as normal so embedded expressions still get processed.
fn render_fragment_shorthand(printer: &mut Printer, node: &HtmlElement) {
    let Ok(opening) = node.opening_element() else {
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
    } else {
        printer.map_nil();
    }
    printer.write(">");

    let opening_end = open_r
        .as_ref()
        .map(|t| u32::from(t.text_trimmed_range().end()))
        .unwrap_or_else(|| u32::from(opening.range().end()));

    let closing_inner_start = node
        .closing_element()
        .ok()
        .and_then(|c| c.l_angle_token().ok())
        .map(|t| u32::from(t.text_trimmed_range().start()));

    let mut prev_end: Option<u32> = None;
    for child in node.children() {
        let child_range = child.range();
        let leading_from = prev_end.unwrap_or(opening_end);
        emit_source_gap(printer, leading_from, u32::from(child_range.start()));
        render_element(printer, child);
        prev_end = Some(u32::from(child_range.end()));
    }
    if let Some(trailing_to) = closing_inner_start {
        emit_source_gap(printer, prev_end.unwrap_or(opening_end), trailing_to);
    }

    if let Ok(closing) = node.closing_element() {
        let l_angle = closing.l_angle_token().ok();
        let r_angle = closing.r_angle_token().ok();
        if let Some(l_angle) = l_angle {
            printer.map_to_offset(range_start(l_angle.text_trimmed_range()));
        } else {
            printer.map_nil();
        }
        printer.write("</");
        if let Some(r_angle) = r_angle {
            printer.map_to_offset(range_start(r_angle.text_trimmed_range()));
        } else {
            printer.map_nil();
        }
        printer.write(">");
    }
}

/// Position immediately after the last attribute (or the tag name when
/// there are no attributes). Used to detect whether there's whitespace
/// between the attributes and the closing `>`/`/>` of the open tag.
fn open_tag_attrs_end(
    name: &AnyHtmlTagName,
    attributes: &biome_html_syntax::HtmlAttributeList,
) -> u32 {
    let attrs: Vec<_> = attributes.iter().collect();
    if let Some(last) = attrs.last() {
        return u32::from(last.range().end());
    }
    u32::from(name.range().end())
}

/// Emits a single space when the source has whitespace between two positions
/// in a tag header. JSX collapses runs of spaces, so `<Foo   >` becomes `<Foo >`.
fn emit_intra_tag_space(printer: &mut Printer, from: u32, to: u32) {
    if from >= to {
        return;
    }
    let from_usize = from as usize;
    let to_usize = (to as usize).min(printer.source.len());
    if from_usize >= to_usize {
        return;
    }
    let slice = &printer.source[from_usize..to_usize];
    if slice.chars().any(|c| c.is_whitespace()) {
        printer.map_to_offset(from);
        printer.write(" ");
    }
}

fn self_closing_attrs_end(node: &HtmlSelfClosingElement) -> u32 {
    let attrs: Vec<_> = node.attributes().iter().collect();
    if let Some(last) = attrs.last() {
        return u32::from(last.range().end());
    }
    if let Ok(name) = node.name() {
        return u32::from(name.range().end());
    }
    0
}

fn emit_open_tag<I>(
    printer: &mut Printer,
    tag_name: &str,
    angle_start: u32,
    name_start: u32,
    attributes: I,
) where
    I: IntoIterator<Item = AnyHtmlAttribute>,
{
    printer.map_to_offset(angle_start);
    printer.write("<");
    printer.map_to_offset(name_start);
    printer.write(tag_name);

    let attrs: Vec<AnyHtmlAttribute> = attributes.into_iter().collect();
    let mut invalid: Vec<InvalidAttr> = Vec::new();

    for attr in &attrs {
        if matches!(attr, AnyHtmlAttribute::HtmlBogusAttribute(_)) {
            // In Astro mode the parser misclassifies `:foo=""` / `@foo=""` as Vue
            // directives, so their source text has to be recovered by hand.
            if let Some(rec) = recover_bogus_attribute(&printer.source, attr) {
                invalid.push(rec);
            }
            continue;
        }
        if let Some(name) = attribute_key(attr)
            && !is_valid_tsx_attribute_name(&name)
        {
            invalid.push(InvalidAttr::Node(attr.clone()));
            continue;
        }
        emit_attribute(printer, attr);
    }

    if !invalid.is_empty() {
        printer.map_nil();
        printer.write(" {...{");
        let mut wrote_entry = false;
        for attr in invalid.iter() {
            match attr {
                InvalidAttr::Node(node) => {
                    wrote_entry |= emit_invalid_attribute(printer, node, wrote_entry);
                }
                InvalidAttr::Recovered { key, value } => {
                    if wrote_entry {
                        printer.map_nil();
                        printer.write(",");
                    }
                    printer.map_nil();
                    printer.write(&format!("{key}:{value}"));
                    wrote_entry = true;
                }
            }
        }
        printer.map_nil();
        printer.write("}}");
    }
}

enum InvalidAttr {
    Node(AnyHtmlAttribute),
    Recovered { key: String, value: String },
}

/// Reconstructs `:class="hey"` (or any other bogus-parsed attribute)
/// from its raw source text. Returns `None` for shapes we can't safely
/// route into the spread block; the attribute is then dropped.
fn recover_bogus_attribute(source: &str, attr: &AnyHtmlAttribute) -> Option<InvalidAttr> {
    let range = attr.range();
    let start = u32::from(range.start()) as usize;
    let end = u32::from(range.end()) as usize;
    if start >= end || end > source.len() {
        return None;
    }
    let text = source[start..end].trim_end();
    let (key, value) = match text.find('=') {
        Some(idx) => {
            let raw_key = text[..idx].trim();
            let raw_value = text[idx + 1..].trim();
            let key = format!("\"{}\"", raw_key.replace('"', "\\\""));
            let value = if raw_value.starts_with('"') || raw_value.starts_with('\'') {
                raw_value.to_string()
            } else if raw_value.starts_with('{') && raw_value.ends_with('}') {
                // Expression form: `{foo}` becomes `(foo)` so the spread
                // object literal stays a valid expression.
                format!("({})", &raw_value[1..raw_value.len() - 1])
            } else {
                format!("\"{}\"", raw_value.replace('"', "\\\""))
            };
            (key, value)
        }
        None => {
            let key = format!("\"{}\"", text.trim().replace('"', "\\\""));
            (key, "true".to_string())
        }
    };
    Some(InvalidAttr::Recovered { key, value })
}

fn emit_attribute(printer: &mut Printer, attr: &AnyHtmlAttribute) {
    match attr {
        AnyHtmlAttribute::HtmlAttribute(attr_node) => emit_html_attribute(printer, attr_node),
        AnyHtmlAttribute::HtmlSpreadAttribute(spread) => emit_spread_attribute(printer, spread),
        AnyHtmlAttribute::HtmlAttributeSingleTextExpression(node) => {
            // Astro shorthand attribute: `<Foo {bar} />`.
            if let Ok(expression) = node.expression()
                && let Ok(token) = expression.html_literal_token()
            {
                let trimmed = token.text_trimmed().to_string();
                let original_start = u32::from(token.text_trimmed_range().start());
                printer.map_nil();
                printer.write(" ");
                printer.write_with_mapping(&trimmed, original_start);
                printer.map_nil();
                printer.write("=");
                printer.map_nil();
                printer.write("{");
                printer.write_with_mapping(&trimmed, original_start);
                printer.map_nil();
                printer.write("}");
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
    let key_start = u32::from(name_token.text_trimmed_range().start());

    printer.map_nil();
    printer.write(" ");
    printer.map_to_offset(key_start);
    printer.write(&key_text);

    let Some(initializer) = attr_node.initializer() else {
        return;
    };
    let Ok(value) = initializer.value() else {
        return;
    };
    let Ok(eq_token) = initializer.eq_token() else {
        return;
    };
    let eq_start = u32::from(eq_token.text_trimmed_range().start());

    match value {
        AnyHtmlAttributeInitializer::HtmlString(s) => {
            let Ok(value_token) = s.value_token() else {
                return;
            };
            let raw = value_token.text_trimmed().to_string();
            // Template-literal attribute (`attr=\`tag\``) — the lexer
            // emitted these as `HtmlString` so the parser path is shared
            // with quoted strings. Detect the leading backtick and emit
            // as `{\`...\`}` so JSX accepts it.
            if raw.starts_with('`') && raw.ends_with('`') && raw.len() >= 2 {
                let inner = &raw[1..raw.len() - 1];
                let value_start = u32::from(value_token.text_trimmed_range().start()) + 1;
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
            // The token includes the surrounding quotes. Strip them so we
            // can re-emit a TSX-safe value.
            let inner = if (raw.starts_with('"') && raw.ends_with('"'))
                || (raw.starts_with('\'') && raw.ends_with('\''))
            {
                &raw[1..raw.len() - 1]
            } else {
                raw.as_str()
            };
            let value_start = u32::from(value_token.text_trimmed_range().start()) + 1;
            let inner_end = value_start + inner.len() as u32;
            let encoded = encode_double_quote(inner);

            printer.map_to_offset(eq_start);
            printer.write("=");
            printer.map_to_offset(value_start - 1);
            printer.write("\"");
            let generated_start = printer.position();
            printer.write_with_mapping(&encoded, value_start);
            let generated_end = printer.position();
            printer.map_to_offset(inner_end);
            printer.write("\"");

            if is_html_event_attribute(&key_text) {
                printer.add_event_attribute(
                    GeneratedRange::new(generated_start, generated_end),
                    inner.to_string(),
                );
            }
            if key_text == "style" {
                printer.add_style_attribute(
                    GeneratedRange::new(generated_start, generated_end),
                    inner.to_string(),
                );
            }
        }
        AnyHtmlAttributeInitializer::HtmlAttributeSingleTextExpression(expr) => {
            // `attr={expr}` — passes through verbatim with surrounding
            // braces so JSX accepts it.
            let Ok(text) = expr.expression() else {
                return;
            };
            let Ok(literal) = text.html_literal_token() else {
                return;
            };
            let value = literal.text_trimmed().to_string();
            let value_start = u32::from(literal.text_trimmed_range().start());

            printer.map_to_offset(eq_start);
            printer.write("=");
            printer.map_nil();
            printer.write("{");
            printer.write_with_mapping(&value, value_start);
            printer.map_nil();
            printer.write("}");
        }
        AnyHtmlAttributeInitializer::SvelteTemplateAttributeValue(_)
        | AnyHtmlAttributeInitializer::VueVForValue(_) => {
            // Not applicable in Astro mode.
        }
    }
}

fn emit_spread_attribute(printer: &mut Printer, spread: &HtmlSpreadAttribute) {
    let Ok(argument) = spread.argument() else {
        return;
    };
    let Ok(literal) = argument.html_literal_token() else {
        return;
    };
    let value = literal.text_trimmed().to_string();
    let value_start = u32::from(literal.text_trimmed_range().start());

    printer.map_nil();
    printer.write(" {");
    printer.map_nil();
    printer.write("...");
    printer.write_with_mapping(&value, value_start);
    printer.map_nil();
    printer.write("}");
}

fn emit_astro_directive(printer: &mut Printer, directive: &AnyAstroDirective) {
    // Astro directives are valid TSX attributes because JSX supports `:` namespaces.
    let range = directive.range();
    let text = slice_source(&printer.source, range);
    printer.map_nil();
    printer.write(" ");
    printer.write_with_mapping(&text, range_start(range));
}

/// Returns whether an entry was written; a skipped entry must not be separated.
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
    let key_start = u32::from(name_token.text_trimmed_range().start());

    if needs_separator {
        printer.map_nil();
        printer.write(",");
    }
    printer.map_to_offset(key_start);
    printer.write(&format!("\"{key_text}\""));

    match attr_node.initializer() {
        None => {
            printer.map_nil();
            printer.write(":");
            printer.map_to_offset(key_start);
            printer.write("true");
        }
        Some(initializer) => match initializer.value() {
            Ok(AnyHtmlAttributeInitializer::HtmlString(s)) => {
                let Ok(value_token) = s.value_token() else {
                    printer.map_nil();
                    printer.write(":true");
                    return true;
                };
                let raw = value_token.text_trimmed().to_string();
                let inner = if (raw.starts_with('"') && raw.ends_with('"'))
                    || (raw.starts_with('\'') && raw.ends_with('\''))
                {
                    &raw[1..raw.len() - 1]
                } else {
                    raw.as_str()
                };
                printer.map_nil();
                printer.write(":");
                printer.map_nil();
                printer.write(&format!("\"{}\"", encode_double_quote(inner)));
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
                let value_start = u32::from(literal.text_trimmed_range().start());
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

fn tag_name_text(name: &AnyHtmlTagName) -> String {
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

fn inner_range(node: &HtmlElement) -> Option<(u32, u32)> {
    let opening = node.opening_element().ok()?;
    let start = u32::from(opening.r_angle_token().ok()?.text_trimmed_range().end());
    let end = match node
        .closing_element()
        .ok()
        .and_then(|c| c.l_angle_token().ok())
    {
        Some(l_angle) => u32::from(l_angle.text_trimmed_range().start()),
        None => u32::from(node.range().end()),
    };
    if start <= end {
        Some((start, end))
    } else {
        None
    }
}

/// Copies `source[from..to]` into the output with per-byte mappings, turning
/// embedded `<!-- ... -->` into `{/** ... */}` so the gap stays valid TSX.
fn emit_source_gap(printer: &mut Printer, from: u32, to: u32) {
    if to <= from {
        return;
    }
    let from_usize = from as usize;
    let to_usize = (to as usize).min(printer.source.len());
    if from_usize >= to_usize {
        return;
    }
    let slice = printer.source[from_usize..to_usize].to_string();
    emit_with_comment_translation(printer, &slice, from);
}

fn emit_with_comment_translation(printer: &mut Printer, slice: &str, base_offset: u32) {
    let bytes = slice.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if i + 4 <= bytes.len() && &bytes[i..i + 4] == b"<!--" {
            let body_start = i + 4;
            let mut end = body_start;
            while end + 3 <= bytes.len() && &bytes[end..end + 3] != b"-->" {
                end += 1;
            }
            if end + 3 > bytes.len() {
                // Emitted as plain text so no content is dropped.
                let chunk = &slice[i..];
                printer.write_with_mapping(chunk, base_offset + i as u32);
                return;
            }
            let body = &slice[body_start..end];
            let original_offset = base_offset + body_start as u32;
            emit_html_comment(printer, body, original_offset);
            i = end + 3;
        } else {
            // Emit a maximal non-comment run, mapping byte-by-byte.
            let chunk_start = i;
            while i < bytes.len() && !(i + 4 <= bytes.len() && &bytes[i..i + 4] == b"<!--") {
                i += 1;
            }
            let chunk = &slice[chunk_start..i];
            if !chunk.is_empty() {
                printer.write_with_mapping(chunk, base_offset + chunk_start as u32);
            }
        }
    }
}

fn emit_html_comment(printer: &mut Printer, body: &str, original_offset: u32) {
    printer.map_nil();
    printer.write("{/**");
    // Without a space, `{/**/}` would close the comment immediately.
    let needs_leading_space = body.chars().next().is_none_or(|c| !c.is_whitespace());
    if needs_leading_space {
        printer.write(" ");
    }
    let escaped = escape_braces(body);
    printer.write_with_mapping(&escaped, original_offset);
    printer.map_nil();
    printer.write("*/}");
}

/// Returns true when the source between `from` and `to` contains an HTML
/// comment that the parser stored as token trivia.
fn slice_has_html_comment(source: &str, from: u32, to: u32) -> bool {
    let from_usize = from as usize;
    let to_usize = (to as usize).min(source.len());
    if from_usize >= to_usize {
        return false;
    }
    source[from_usize..to_usize].contains("<!--")
}

fn slice_source(source: &str, range: TextRange) -> String {
    let start = u32::from(range.start()) as usize;
    let end = u32::from(range.end()) as usize;
    if start >= source.len() || end > source.len() {
        return String::new();
    }
    source[start..end].to_string()
}
