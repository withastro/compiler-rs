//! Walks the Biome HTML CST for an Astro file and emits TSX.

use biome_html_syntax::{
    AnyAstroDirective, AnyAstroFrontmatterElement, AnyHtmlAttribute, AnyHtmlAttributeInitializer,
    AnyHtmlComponentObjectName, AnyHtmlContent, AnyHtmlElement, AnyHtmlTagName,
    AnyHtmlTextExpression, HtmlAttribute, HtmlElement, HtmlRoot, HtmlSelfClosingElement,
    HtmlSingleTextExpression, HtmlSpreadAttribute,
};
use biome_js_parser::{JsOffsetParse, JsParserOptions, parse, parse_js_with_offset};
use biome_languages::JsFileSource;
use biome_languages::javascript::JsEmbeddingKind;
use biome_rowan::{AstNode, AstNodeList, Direction, TextRange, TextSize};

use crate::ConvertOptions;
use crate::expression::emit_expression_tree;
use crate::frontmatter::rewrite_top_level_returns;
use crate::printer::{Printer, range_start};
use crate::props::{PropsAnalysis, analyze as analyze_props};
use crate::sourcemap::{
    Diagnostic, DiagnosticSeverity, FrontmatterInfo, FrontmatterStatus, GeneratedRange, SourceRange,
};
use crate::utils::{
    ScriptKind, classify_script_type, comment_needs_leading_space, encode_double_quote,
    is_html_event_attribute, is_valid_tsx_attribute_name, tsx_component_name,
};

const TSX_PREFIX: &str = "/* @jsxImportSource astro */\n\n";

pub(crate) fn render_root(printer: &mut Printer, root: HtmlRoot, options: &ConvertOptions) {
    printer.comment_ranges = comment_trivia_ranges(&root);
    printer.map_nil();
    printer.write(TSX_PREFIX);
    // Where frontmatter would be inserted, so editors can anchor an edit there.
    printer.frontmatter_range = GeneratedRange::new(printer.position(), printer.position());

    let frontmatter = root.frontmatter();
    let body = root.html();

    let content = frontmatter.as_ref().and_then(frontmatter_content);
    let mut props_analysis = PropsAnalysis::default();
    let mut rewritten: Option<(String, u32)> = None;
    if let Some((text, start)) = &content
        && !text.is_empty()
    {
        let parse = parse(text, JsFileSource::astro(), JsParserOptions::default());
        let js_root = parse.tree();
        props_analysis = analyze_props(&js_root);
        rewritten = Some((rewrite_top_level_returns(text, &js_root), *start));
    }

    if let Some(node) = &frontmatter {
        emit_frontmatter(printer, node, rewritten.as_ref(), frontmatter_anchor(node));
    }

    printer.frontmatter_info = frontmatter_info(&frontmatter, printer.source.len() as u32);
    let body_text_start = body_text_start_offset(&frontmatter);
    // A childless body still needs its `<Fragment>` when comment trivia remains.
    let has_body_children = body.iter().next().is_some()
        || printer
            .comment_ranges
            .iter()
            .any(|range| u32::from(range.start()) >= body_text_start);
    let body_start;

    if has_body_children {
        // Unterminated frontmatter would swallow the body as `x < Fragment > ...`.
        let frontmatter_needs_terminator = match &frontmatter {
            Some(AnyAstroFrontmatterElement::AstroFrontmatterElement(_)) => content
                .as_ref()
                .is_some_and(|(text, _)| !text.trim().is_empty()),
            Some(AnyAstroFrontmatterElement::AstroBogusFrontmatter(_)) => true,
            None => false,
        };
        if frontmatter_needs_terminator {
            printer.map_nil();
            printer.write("{};");
        }
        printer.map_nil();
        printer.write("<Fragment>\n");
        body_start = printer.position();

        let mut prev_end = body_text_start;
        for element in body.iter() {
            let element_range = element.range();
            emit_source_gap(printer, prev_end, u32::from(element_range.start()));
            render_element(printer, element);
            prev_end = u32::from(element_range.end());
        }
        emit_source_gap(printer, prev_end, printer.source.len() as u32);
        // Anchored at EOF so trailing whitespace still has a mapping.
        printer.map_to_offset(printer.source.len() as u32);
        printer.write("\n");

        let body_end = printer.position();
        printer.body_range = GeneratedRange::new(body_start, body_end);

        printer.map_nil();
        printer.write("</Fragment>\n");
    } else {
        // Stands in for the trailing newline a body would have provided.
        if frontmatter.is_some() {
            printer.map_nil();
            printer.write("\n");
        }
        printer.body_range = GeneratedRange::new(printer.position(), printer.position());
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

fn frontmatter_info(
    frontmatter: &Option<AnyAstroFrontmatterElement>,
    source_len: u32,
) -> FrontmatterInfo {
    match frontmatter {
        None => FrontmatterInfo::default(),
        Some(AnyAstroFrontmatterElement::AstroBogusFrontmatter(node)) => FrontmatterInfo {
            status: FrontmatterStatus::Open,
            source: SourceRange::new(range_start(node.range()), source_len),
        },
        Some(AnyAstroFrontmatterElement::AstroFrontmatterElement(node)) => {
            let start = node
                .l_fence_token()
                .map(|token| range_start(token.text_trimmed_range()))
                .unwrap_or_else(|_| range_start(node.range()));
            match node.r_fence_token() {
                Ok(r_fence) => FrontmatterInfo {
                    status: FrontmatterStatus::Closed,
                    source: SourceRange::new(start, u32::from(r_fence.text_trimmed_range().end())),
                },
                Err(_) => FrontmatterInfo {
                    status: FrontmatterStatus::Open,
                    source: SourceRange::new(start, source_len),
                },
            }
        }
    }
}

/// The frontmatter's JS text (trivia included) and its source offset.
fn frontmatter_content(node: &AnyAstroFrontmatterElement) -> Option<(String, u32)> {
    if let AnyAstroFrontmatterElement::AstroFrontmatterElement(frontmatter) = node
        && let Ok(content) = frontmatter.content()
        && let Some(token) = content.content_token()
    {
        return Some((token.text().to_string(), range_start(token.text_range())));
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

/// Offset just after the opening fence, where an empty frontmatter's newline
/// lives. Editors insert imports there, so it must carry a mapping.
fn frontmatter_anchor(frontmatter: &AnyAstroFrontmatterElement) -> Option<u32> {
    let AnyAstroFrontmatterElement::AstroFrontmatterElement(node) = frontmatter else {
        return None;
    };
    node.l_fence_token()
        .ok()
        .map(|token| u32::from(token.text_trimmed_range().end()))
}

fn emit_frontmatter(
    printer: &mut Printer,
    frontmatter: &AnyAstroFrontmatterElement,
    rewritten: Option<&(String, u32)>,
    anchor: Option<u32>,
) {
    let frontmatter_start = printer.position();

    match frontmatter {
        AnyAstroFrontmatterElement::AstroFrontmatterElement(_) => {
            printer.map_to_offset(0);
            let emitted_content = match rewritten {
                Some((text, start)) => {
                    printer.write_with_mapping(text, *start);
                    !text.is_empty()
                }
                None => false,
            };
            match anchor.filter(|_| !emitted_content) {
                Some(anchor) if printer.source[anchor as usize..].starts_with('\n') => {
                    printer.map_to_offset(anchor)
                }
                _ => printer.map_nil(),
            }
            printer.write("\n");
        }
        AnyAstroFrontmatterElement::AstroBogusFrontmatter(_) => {
            printer.map_to_offset(0);
            printer.write(frontmatter.syntax().text_trimmed().to_string().as_str());
        }
    }

    let frontmatter_end = printer.position();
    printer.frontmatter_range = GeneratedRange::new(frontmatter_start, frontmatter_end);
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
            let text = slice_source(printer.source, range);
            printer.write_with_mapping(text, range_start(range));
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
            // `{{ … }}` is Vue's syntax — Astro doesn't use it, but we
            // accept it for HTML mode by passing through the raw text.
            let range = node.range();
            let text = slice_source(printer.source, range);
            printer.write_with_mapping(text, range_start(range));
        }
        AnyHtmlTextExpression::HtmlBogusTextExpression(node) => {
            let range = node.range();
            let text = slice_source(printer.source, range);
            printer.write_with_mapping(text, range_start(range));
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
            emit_expression_body(printer, raw, original_start);
        }
    } else {
        printer.map_nil();
        printer.write("(void 0)");
    }

    printer.map_to_offset(range_start(r_curly_range));
    printer.write("}");
}

/// Parses an expression body so markup is located by the tree rather than by
/// scanning bytes, which is what keeps strings and generics from being rewritten.
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

fn emit_expression_body(printer: &mut Printer, raw: &str, original_start: u32) {
    let parse = parse_expression_body(raw, original_start);
    if parse.diagnostics().is_empty() {
        let syntax = parse.syntax();
        emit_expression_tree(printer, syntax.inner(), original_start);
        return;
    }
    printer.has_expression_errors = true;
    for diagnostic in parse.diagnostics() {
        printer.diagnostics.push(Diagnostic {
            message: diagnostic.message.to_string(),
            severity: DiagnosticSeverity::Error,
            source: diagnostic_source_range(diagnostic, original_start, raw.len() as u32),
        });
    }
    printer.write_with_mapping(raw, original_start);
}

/// Expression-body diagnostics already carry document offsets thanks to the
/// parse offset, but a spanless one falls back to the whole body.
fn diagnostic_source_range(
    diagnostic: &biome_parser::diagnostic::ParseDiagnostic,
    start: u32,
    len: u32,
) -> SourceRange {
    match biome_diagnostics::Diagnostic::location(diagnostic).span {
        Some(span) => SourceRange::new(u32::from(span.start()), u32::from(span.end())),
        None => SourceRange::new(start, start + len),
    }
}

fn render_html_element(printer: &mut Printer, node: HtmlElement) {
    let Ok(opening) = node.opening_element() else {
        return;
    };
    let Some(name) = opening.name() else {
        // Fragment shorthand: `<>...</>`, which JSX accepts as `Fragment`.
        render_fragment_shorthand(printer, &node);
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
        let text = slice_source(printer.source, range);
        printer.write_with_mapping(text, range_start(range));
        return;
    }

    let tag_name = tag_name_text(&name);
    let attributes: Vec<AnyHtmlAttribute> = opening.attributes().iter().collect();

    emit_open_tag(
        printer,
        &tag_name,
        u32::from(open_l_angle.text_trimmed_range().start()),
        u32::from(name.range().start()),
        &attributes,
    );

    let open_r_angle = open_r_angle.expect("checked above");
    let r_angle_start = u32::from(open_r_angle.text_trimmed_range().start());
    let attrs_end = attributes
        .last()
        .map(|attr| u32::from(attr.range().end()))
        .unwrap_or_else(|| u32::from(name.range().end()));
    emit_intra_tag_space(printer, attrs_end, r_angle_start);
    printer.map_to_offset(r_angle_start);
    printer.write(">");

    // `<Script>` is a component, not the HTML element, so match the node kind.
    let is_html_tag = matches!(name, AnyHtmlTagName::HtmlTagName(_));
    let is_script = is_html_tag && tag_name.eq_ignore_ascii_case("script");
    let is_style = is_html_tag && tag_name.eq_ignore_ascii_case("style");

    let children = node.children();
    let body_start = printer.position();

    let opening_end = u32::from(open_r_angle.text_trimmed_range().end());
    let element_is_raw = attributes
        .iter()
        .any(|a| attribute_key(a).as_deref() == Some("is:raw"));
    // Script and style win over `is:raw` when classifying the body.
    let inline_raw_body = element_is_raw && !is_script && !is_style;

    let closing_inner_start = node
        .closing_element()
        .ok()
        .and_then(|c| c.l_angle_token().ok())
        .map(|t| u32::from(t.text_trimmed_range().start()));

    if is_script || is_style {
        // The tag still prints; only its text content is left out.
    } else if inline_raw_body {
        // Unclosed raw-text content runs to the end of the node the parser built.
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
    } else {
        let mut prev_end: Option<u32> = None;
        for child in children.iter() {
            let child_range = child.range();
            let child_start = u32::from(child_range.start());
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
                    classify_script_label(&attributes),
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
        let l_angle = closing.l_angle_token().ok();
        let r_angle = closing.r_angle_token().ok();
        let closing_name = closing.name();

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

/// Only a bare `<script>` is processed by Astro; any attribute (`is:inline`,
/// `define:vars`, …) leaves the script inline, sharing one scope with its peers.
fn classify_script_label(attrs: &[AnyHtmlAttribute]) -> &'static str {
    if attrs.is_empty() {
        return "processed-module";
    }
    if attrs
        .iter()
        .any(|a| attribute_key(a).as_deref() == Some("is:raw"))
    {
        return "raw";
    }
    script_label_for_type(find_attr_value(attrs, "type").as_deref())
}

pub(crate) fn script_label_for_type(type_value: Option<&str>) -> &'static str {
    let Some(value) = type_value else {
        return "inline";
    };
    match classify_script_type(Some(value)) {
        ScriptKind::Script => {
            if value.trim().eq_ignore_ascii_case("module") {
                "module"
            } else {
                "inline"
            }
        }
        ScriptKind::Json => "json",
        ScriptKind::Unknown => "unknown",
    }
}

fn style_lang_label(attrs: &[AnyHtmlAttribute]) -> String {
    find_attr_value(attrs, "lang")
        .filter(|lang| !lang.is_empty())
        .unwrap_or_else(|| "css".to_string())
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
        if !token.text_trimmed().eq_ignore_ascii_case(name) {
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
    let attributes: Vec<AnyHtmlAttribute> = node.attributes().iter().collect();
    let tag_name = tag_name_text(&name);

    emit_open_tag(
        printer,
        &tag_name,
        u32::from(l_angle.text_trimmed_range().start()),
        u32::from(name.range().start()),
        &attributes,
    );

    // The probe point sits before the slash: `<Foo />` keeps its space, `<Foo/>` stays tight.
    let r_angle_start = u32::from(r_angle.text_trimmed_range().start());
    let pre_slash = node
        .slash_token()
        .map(|s| u32::from(s.text_trimmed_range().start()))
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

/// Anything between trimmed tag-header tokens is whitespace or comment
/// trivia; both collapse to the single space JSX expects.
fn emit_intra_tag_space(printer: &mut Printer, from: u32, to: u32) {
    if from < to {
        printer.map_to_offset(from);
        printer.write(" ");
    }
}

fn emit_open_tag(
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

    for attr in attrs {
        if let Some(name) = attribute_key(attr)
            && !is_valid_tsx_attribute_name(&name)
        {
            invalid.push(attr);
            continue;
        }
        emit_attribute(printer, attr);
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
            let token_start = u32::from(value_token.text_trimmed_range().start());
            // Astro allows unquoted values, so the token may carry no quotes to strip.
            let quoted = (raw.starts_with('"') && raw.ends_with('"'))
                || (raw.starts_with('\'') && raw.ends_with('\''));
            let (inner, value_start) = if quoted {
                (&raw[1..raw.len() - 1], token_start + 1)
            } else {
                (raw.as_str(), token_start)
            };
            let inner_end = value_start + inner.len() as u32;

            printer.map_to_offset(eq_start);
            printer.write("=");
            if quoted {
                printer.map_to_offset(token_start);
            } else {
                printer.map_nil();
            }
            printer.write("\"");
            let generated_start = printer.position();
            printer.write_attribute_value_with_mapping(inner, value_start);
            let generated_end = printer.position();
            if quoted {
                printer.map_to_offset(inner_end);
            } else {
                printer.map_nil();
            }
            printer.write("\"");

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
            printer.map_nil();
            printer.write("{");
            match literal {
                Some(literal) if !literal.text_trimmed().trim().is_empty() => {
                    let value = literal.text_trimmed().to_string();
                    emit_expression_body(
                        printer,
                        &value,
                        range_start(literal.text_trimmed_range()),
                    );
                }
                // TSX rejects an empty attribute expression.
                _ => {
                    printer.map_nil();
                    printer.write("(void 0)");
                }
            }
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
    let text = slice_source(printer.source, range);
    printer.map_nil();
    printer.write(" ");
    printer.write_with_mapping(text, range_start(range));
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
    printer.map_nil();
    printer.write("\"");
    printer.write_with_mapping(&key_text, key_start);
    printer.map_nil();
    printer.write("\"");

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

/// HTML comments never become nodes; the lexer stores them as token trivia.
fn comment_trivia_ranges(root: &HtmlRoot) -> Vec<TextRange> {
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

/// Emits `source[from..to]`, rewriting `<!-- -->` to `{/** */}` for TSX.
fn emit_source_gap(printer: &mut Printer, from: u32, to: u32) {
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
        let (start, end) = (
            u32::from(range.start()) as usize,
            u32::from(range.end()) as usize,
        );
        if start >= to {
            break;
        }
        if start < cursor || end > to {
            continue;
        }
        printer.write_with_mapping(&source[cursor..start], cursor as u32);
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
    printer.write_with_mapping(&source[cursor..to], cursor as u32);
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

fn slice_source(source: &str, range: TextRange) -> &str {
    let start = u32::from(range.start()) as usize;
    let end = u32::from(range.end()) as usize;
    source.get(start..end).unwrap_or("")
}
