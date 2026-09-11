use biome_html_syntax::AnyAstroFrontmatterElement;
use biome_js_parser::{JsParserOptions, parse};
use biome_js_syntax::{
    AnyJsRoot, JsArrowFunctionExpression, JsReturnStatement, JsSyntaxKind,
    TsTypeAssertionExpression,
};
use biome_languages::JsFileSource;
use biome_rowan::{AstNode, AstSeparatedList, TextRange, WalkEvent};

use crate::printer::{Printer, range_start};
use crate::props::{PropsAnalysis, analyze as analyze_props};
use crate::types::{
    Diagnostic, DiagnosticSeverity, FrontmatterInfo, FrontmatterStatus, GeneratedRange, SourceRange,
};

pub(super) struct RenderedFrontmatter {
    pub(super) props_analysis: PropsAnalysis,
    pub(super) has_component_export: bool,
    pub(super) body_text_start: u32,
    pub(super) needs_terminator: bool,
}

enum Piece {
    Source(TextRange),
    Insert(&'static str),
}

struct Rewrite {
    range: TextRange,
    pieces: Vec<Piece>,
}

pub(super) fn render(
    printer: &mut Printer,
    frontmatter: Option<&AnyAstroFrontmatterElement>,
    component_alias: Option<&str>,
) -> RenderedFrontmatter {
    // Where frontmatter would be inserted, so editors can anchor an edit there.
    printer.frontmatter_range = GeneratedRange::new(printer.position(), printer.position());

    let content = frontmatter.and_then(frontmatter_content);
    let mut props_analysis = PropsAnalysis::default();
    let mut has_component_export = false;
    let mut rewritten = None;
    if let Some((text, start)) = &content
        && !text.is_empty()
    {
        let parse = parse(text, JsFileSource::astro(), JsParserOptions::default());
        let js_root = parse.tree();
        if !parse.diagnostics().is_empty() {
            printer.has_embedded_parse_errors = true;
            for diagnostic in parse.diagnostics() {
                let source = match biome_diagnostics::Diagnostic::location(diagnostic).span {
                    Some(span) => SourceRange::new(
                        start + u32::from(span.start()),
                        start + u32::from(span.end()),
                    ),
                    None => SourceRange::new(*start, start + text.len() as u32),
                };
                printer.diagnostics.push(Diagnostic {
                    message: diagnostic.message.to_string(),
                    severity: DiagnosticSeverity::Error,
                    source,
                });
            }
        }
        props_analysis = analyze_props(&js_root);
        has_component_export =
            component_alias.is_some_and(|name| crate::props::exports_name(&js_root, name));
        rewritten = Some((js_root, *start));
    }

    if let Some(node) = frontmatter {
        emit_frontmatter(printer, node, rewritten.as_ref());
    }
    if let Some((root, _)) = rewritten {
        crate::syntax::drop_syntax_tree(root.into_syntax());
    }

    printer.frontmatter_info = frontmatter_info(frontmatter, printer.source.len() as u32);
    RenderedFrontmatter {
        props_analysis,
        has_component_export,
        body_text_start: body_text_start_offset(frontmatter),
        needs_terminator: match frontmatter {
            Some(AnyAstroFrontmatterElement::AstroFrontmatterElement(_)) => content
                .as_ref()
                .is_some_and(|(text, _)| !text.trim().is_empty()),
            Some(AnyAstroFrontmatterElement::AstroBogusFrontmatter(_)) => true,
            None => false,
        },
    }
}

pub(super) fn emit_default_export(
    printer: &mut Printer,
    component: &str,
    analysis: &PropsAnalysis,
    ambient_types: bool,
) -> GeneratedRange {
    let (props_param, props_global) = if analysis.has_props {
        if analysis.generics_args.is_empty() {
            ("Props".to_string(), "Props".to_string())
        } else {
            (
                format!("Props{}", analysis.generics_args),
                format!("Parameters<typeof {component}>[0]"),
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

    printer.map_nil();
    printer.write("export default function ");
    let component_start = printer.position();
    printer.write(component);
    let component_range = GeneratedRange::new(component_start, printer.position());
    printer.write(&format!("{generics}(_props: {props_param}): any {{}}\n"));

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

    if ambient_types {
        printer.write("declare const Fragment: any;\n");
    }

    if analysis.has_props || analysis.has_get_static_paths || ambient_types {
        printer.write(
            "/**\n * Astro global available in all contexts in .astro files\n *\n * [Astro documentation](https://docs.astro.build/reference/api-reference/#astro-global)\n*/\n",
        );
        printer.write(&format!(
            "declare const Astro: Readonly<import('astro').AstroGlobal<{props_global}, typeof {component}"
        ));
        if analysis.has_get_static_paths {
            printer.write(", ASTRO__Get<ASTRO__InferredGetStaticPath, 'params'>");
        }
        printer.write(">>;\n");
    }
    component_range
}

fn body_text_start_offset(frontmatter: Option<&AnyAstroFrontmatterElement>) -> u32 {
    if let Some(AnyAstroFrontmatterElement::AstroFrontmatterElement(node)) = frontmatter
        && let Ok(r_fence) = node.r_fence_token()
    {
        return u32::from(r_fence.text_trimmed_range().end());
    }
    0
}

fn frontmatter_info(
    frontmatter: Option<&AnyAstroFrontmatterElement>,
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

fn frontmatter_content(node: &AnyAstroFrontmatterElement) -> Option<(String, u32)> {
    if let AnyAstroFrontmatterElement::AstroFrontmatterElement(frontmatter) = node
        && let Ok(content) = frontmatter.content()
        && let Some(token) = content.content_token()
    {
        return Some((token.text().to_string(), range_start(token.text_range())));
    }
    None
}

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
    rewritten: Option<&(AnyJsRoot, u32)>,
) {
    let frontmatter_start = printer.position();

    match frontmatter {
        AnyAstroFrontmatterElement::AstroFrontmatterElement(_) => {
            printer.map_to_offset(0);
            let emitted_content = match rewritten {
                Some((root, start)) => {
                    emit_tsx_frontmatter(printer, root, *start);
                    true
                }
                None => false,
            };
            // Empty frontmatter needs a mapped newline as an insertion point for editor imports.
            let anchor_newline = frontmatter_anchor(frontmatter)
                .filter(|_| !emitted_content)
                .and_then(|anchor| {
                    let rest = &printer.source[anchor as usize..];
                    if rest.starts_with("\r\n") {
                        Some((anchor, "\r\n"))
                    } else if rest.starts_with('\n') {
                        Some((anchor, "\n"))
                    } else {
                        None
                    }
                });
            match anchor_newline {
                Some((anchor, newline)) => {
                    printer.map_to_offset(anchor);
                    printer.write(newline);
                }
                None => {
                    printer.map_nil();
                    printer.write("\n");
                }
            }
        }
        AnyAstroFrontmatterElement::AstroBogusFrontmatter(_) => {
            printer.map_to_offset(0);
            printer.write(frontmatter.syntax().text_trimmed().to_string().as_str());
        }
    }

    let frontmatter_end = printer.position();
    printer.frontmatter_range = GeneratedRange::new(frontmatter_start, frontmatter_end);
}

fn emit_tsx_frontmatter(printer: &mut Printer, root: &AnyJsRoot, start: u32) {
    let mut rewrites: Vec<Rewrite> = find_top_level_returns(root)
        .into_iter()
        .map(|(range, has_argument)| Rewrite {
            range,
            pieces: vec![Piece::Insert(if has_argument {
                "throw "
            } else {
                "throw undefined"
            })],
        })
        .collect();
    for event in root.syntax().preorder() {
        match event {
            WalkEvent::Enter(node) => {
                if let Some(arrow) = JsArrowFunctionExpression::cast_ref(&node)
                    && let Some(parameters) = arrow.type_parameters()
                    && parameters.items().len() == 1
                    && parameters.items().trailing_separator().is_none()
                    && let Ok(end) = parameters.r_angle_token()
                {
                    rewrites.push(Rewrite {
                        range: TextRange::empty(end.text_trimmed_range().start()),
                        pieces: vec![Piece::Insert(",")],
                    });
                }
                if let Some(assertion) = TsTypeAssertionExpression::cast(node)
                    && let (Ok(left), Ok(right)) =
                        (assertion.l_angle_token(), assertion.r_angle_token())
                {
                    rewrites.push(Rewrite {
                        range: TextRange::new(
                            left.text_trimmed_range().start(),
                            right.text_trimmed_range().end(),
                        ),
                        pieces: vec![Piece::Insert("(")],
                    });
                }
            }
            WalkEvent::Leave(node) => {
                if let Some(assertion) = TsTypeAssertionExpression::cast(node)
                    && let (Ok(left), Ok(right)) =
                        (assertion.l_angle_token(), assertion.r_angle_token())
                {
                    rewrites.push(Rewrite {
                        range: TextRange::empty(assertion.syntax().text_trimmed_range().end()),
                        pieces: vec![
                            Piece::Insert(" as "),
                            Piece::Source(TextRange::new(
                                left.text_trimmed_range().end(),
                                right.text_trimmed_range().start(),
                            )),
                            Piece::Insert(")"),
                        ],
                    });
                }
            }
        }
    }
    rewrites.sort_by_key(|rewrite| rewrite.range.start());
    let write_source = |printer: &mut Printer, range: TextRange| {
        let from = start + u32::from(range.start());
        let to = start + u32::from(range.end());
        printer.write_with_mapping(&printer.source[from as usize..to as usize], from);
    };
    let mut cursor = root.syntax().text_range_with_trivia().start();
    for rewrite in rewrites {
        write_source(printer, TextRange::new(cursor, rewrite.range.start()));
        for piece in rewrite.pieces {
            match piece {
                Piece::Source(range) => write_source(printer, range),
                Piece::Insert(text) => {
                    printer.map_nil();
                    printer.write(text);
                }
            }
        }
        cursor = rewrite.range.end();
    }
    write_source(
        printer,
        TextRange::new(cursor, root.syntax().text_range_with_trivia().end()),
    );
}

fn find_top_level_returns(root: &AnyJsRoot) -> Vec<(TextRange, bool)> {
    let mut returns = Vec::new();
    let mut function_depth: u32 = 0;

    for event in root.syntax().preorder() {
        match event {
            WalkEvent::Enter(node) => {
                if is_function_like(node.kind()) {
                    function_depth += 1;
                } else if function_depth == 0
                    && node.kind() == JsSyntaxKind::JS_RETURN_STATEMENT
                    && let Some(stmt) = JsReturnStatement::cast(node)
                    && let Ok(token) = stmt.return_token()
                {
                    returns.push((token.text_trimmed_range(), stmt.argument().is_some()));
                }
            }
            WalkEvent::Leave(node) => {
                if is_function_like(node.kind()) {
                    function_depth = function_depth.saturating_sub(1);
                }
            }
        }
    }
    returns
}

fn is_function_like(kind: JsSyntaxKind) -> bool {
    matches!(
        kind,
        JsSyntaxKind::JS_FUNCTION_DECLARATION
            | JsSyntaxKind::JS_FUNCTION_EXPORT_DEFAULT_DECLARATION
            | JsSyntaxKind::JS_FUNCTION_EXPRESSION
            | JsSyntaxKind::JS_ARROW_FUNCTION_EXPRESSION
            | JsSyntaxKind::JS_METHOD_CLASS_MEMBER
            | JsSyntaxKind::JS_METHOD_OBJECT_MEMBER
            | JsSyntaxKind::JS_GETTER_CLASS_MEMBER
            | JsSyntaxKind::JS_SETTER_CLASS_MEMBER
            | JsSyntaxKind::JS_GETTER_OBJECT_MEMBER
            | JsSyntaxKind::JS_SETTER_OBJECT_MEMBER
            | JsSyntaxKind::JS_CONSTRUCTOR_CLASS_MEMBER
    )
}

#[cfg(test)]
mod tests {
    use biome_js_parser::{JsParserOptions, parse};
    use biome_languages::JsFileSource;

    use crate::test_utils::{assert_mapped_runs_are_verbatim, convert};
    use crate::{ConvertOptions, convert_to_tsx};

    #[test]
    fn typescript_frontmatter_is_disambiguated_as_tsx() {
        for code in [
            "const id = <T>(x: T) => x;",
            "const id = <T /* comment */>(x: T) => x;",
            "const id = <T,>(x: T) => x;",
            "const id = <T extends string>(x: T) => x;",
            "const id = <T = string>(x: T) => x;",
            "const value = <string>Astro.props.value;",
            "const value = <number><unknown>Astro.props.value;",
            "const value = (<{ x: number }>Astro.props.value).x;",
            "const value = < /* before */ string /* after */ >Astro.props.value;",
            "const value = <const>{ x: 1 };",
            "if (Astro.props.x) return <string>Astro.props.value;",
            "const id = <T>(x: T) => <T>x;",
            "const value = <string>((<T>(x: T) => x)('x'));",
        ] {
            convert(&format!("---\n{code}\n---\n<p/>"));
        }
    }

    #[test]
    fn fences_after_markup_are_template_text() {
        for prefix in ["<!-- Copyright -->", "<div/>", "<!DOCTYPE html>"] {
            let source = format!("{prefix}\n---\nconst title = 'hello';\n---\n<h1>hello</h1>");
            let result = convert(&source);
            assert_eq!(
                result.frontmatter.status,
                crate::FrontmatterStatus::DoesntExist
            );
            assert_eq!(result.frontmatter_range.start, result.frontmatter_range.end);
            assert!(result.code.contains("---\nconst title = 'hello';\n---"));
        }
    }

    #[test]
    fn frontmatter_range_is_recorded() {
        let result = convert_to_tsx(
            "---\nlet x = 1;\n---\n<div></div>",
            ConvertOptions::default(),
        );
        assert!(result.frontmatter_range.end > result.frontmatter_range.start);
        let frontmatter_slice = &result.code
            [result.frontmatter_range.start as usize..result.frontmatter_range.end as usize];
        assert!(frontmatter_slice.contains("let x = 1;"));
    }

    #[test]
    fn get_static_paths_needs_a_real_export() {
        let mentioned = convert_to_tsx(
            "---\n// see getStaticPaths in the docs\nexport const x = 1;\n---\n",
            ConvertOptions::default(),
        )
        .code;
        assert!(
            !mentioned.contains("ASTRO__InferredGetStaticPath"),
            "a mention injected the inferred-props machinery:\n{mentioned}"
        );
        assert!(
            mentioned.contains("_props: Record<string, any>"),
            "a mention changed the _props type:\n{mentioned}"
        );

        let referenced = convert_to_tsx(
            "---\nexport const handler = getStaticPaths;\n---\n",
            ConvertOptions::default(),
        )
        .code;
        assert!(
            !referenced.contains("ASTRO__InferredGetStaticPath"),
            "a reference is not an export:\n{referenced}"
        );

        for input in [
            "---\nexport const getStaticPaths = async () => [];\n---\n",
            "---\nexport async function getStaticPaths() { return []; }\n---\n",
            "---\nconst getStaticPaths = async () => [];\nexport { getStaticPaths };\n---\n",
        ] {
            let actual = convert_to_tsx(input, ConvertOptions::default()).code;
            assert!(
                actual.contains("ASTRO__InferredGetStaticPath"),
                "missed a real export for {input:?}:\n{actual}"
            );
        }
    }

    #[test]
    fn get_static_paths_inference_requires_a_local_named_export() {
        for (frontmatter, expected) in [
            ("export const getStaticPaths = () => [];", true),
            ("export const get\\u0053taticPaths = () => [];", true),
            ("export function getStaticPaths() { return []; }", true),
            (
                "const getStaticPaths = () => []; export { getStaticPaths };",
                true,
            ),
            (
                "const getStaticPaths = () => []; export { getStaticPaths as getStaticPaths };",
                true,
            ),
            (
                "const getStaticPaths = () => []; export { getStaticPaths as 'getStaticPaths' };",
                true,
            ),
            (
                "const getStaticPaths = () => []; export { getStaticPaths as 'get\\u0053taticPaths' };",
                true,
            ),
            ("export function f(getStaticPaths) {};", false),
            ("export function f(get\\u0053taticPaths) {};", false),
            (
                "export function f() { function getStaticPaths() {} }",
                false,
            ),
            (
                "export function f() { const get\\u0053taticPaths = () => []; }",
                false,
            ),
            ("export const f = function getStaticPaths() {};", false),
            ("export class Routes { getStaticPaths() {} }", false),
            (
                "export default function getStaticPaths() { return []; }",
                false,
            ),
            (
                "const paths = () => []; export { paths as getStaticPaths };",
                false,
            ),
            (
                "const getStaticPaths = () => []; export { getStaticPaths as paths };",
                false,
            ),
            ("export { getStaticPaths } from './paths';", false),
            ("export { foo as getStaticPaths } from './paths';", false),
        ] {
            let actual = convert_to_tsx(
                &format!("---\n{frontmatter}\n---\n"),
                ConvertOptions::default(),
            )
            .code;
            assert_eq!(
                actual.contains("ReturnType<typeof getStaticPaths>"),
                expected,
                "wrong inference for {frontmatter:?}:\n{actual}"
            );
        }
    }

    #[test]
    fn props_binding_needs_a_local_name() {
        for (input, has_props) in [
            ("---\nimport Foo from './Props';\nFoo;\n---\n<div/>", false),
            (
                "---\nimport { Props as Other } from './t';\n---\n<div/>",
                false,
            ),
            ("---\nexport { Props } from './t';\n---\n<div/>", false),
            (
                "---\n// mentions Props only in a comment\n---\n<div/>",
                false,
            ),
            (
                "---\nimport { Other as Props } from './t';\n---\n<div/>",
                true,
            ),
            ("---\nimport type { Props } from './t';\n---\n<div/>", true),
            ("---\nimport Props from './t';\n---\n<div/>", true),
            (
                "---\nexport interface Props { a: string }\n---\n<div/>",
                true,
            ),
        ] {
            let actual = convert_to_tsx(input, ConvertOptions::default()).code;
            assert_eq!(
                actual.contains("_props: Props"),
                has_props,
                "wrong Props detection for {input:?}:\n{actual}"
            );
        }
    }

    #[test]
    fn frontmatter_is_terminated_even_when_a_comment_ends_with_a_semicolon() {
        let actual = convert_to_tsx(
            "---\nconst x = foo\n// note;\n---\n<div/>",
            ConvertOptions::default(),
        )
        .code;
        assert!(
            actual.contains("{};<Fragment>"),
            "`<Fragment>` can continue the unterminated expression:\n{actual}"
        );
    }

    #[test]
    fn frontmatter_status_and_source_are_reported() {
        use crate::FrontmatterStatus;

        let closed = convert_to_tsx("---\nlet x = 1;\n---\n<p/>", ConvertOptions::default());
        assert_eq!(closed.frontmatter.status, FrontmatterStatus::Closed);
        assert_eq!(closed.frontmatter.source.start, 0);
        assert_eq!(closed.frontmatter.source.end, 18);

        let open = convert_to_tsx("---\nlet x = 1;\n", ConvertOptions::default());
        assert_eq!(open.frontmatter.status, FrontmatterStatus::Open);

        let absent = convert_to_tsx("<p>hi</p>", ConvertOptions::default());
        assert_eq!(absent.frontmatter.status, FrontmatterStatus::DoesntExist);
    }

    #[test]
    fn invalid_frontmatter_reports_document_relative_diagnostics() {
        let source = "---\nconst = ;\n---\n<p/>";
        let result = convert_to_tsx(source, ConvertOptions::default());

        assert!(result.has_parse_errors);
        assert!(!result.diagnostics.is_empty());
        assert!(result.code.contains("const = ;"));
        for diagnostic in result.diagnostics {
            assert!(diagnostic.source.start >= 4, "{diagnostic:?}");
            assert!(diagnostic.source.end <= 13, "{diagnostic:?}");
        }
    }

    #[test]
    fn handles_non_latin_identifiers() {
        let frontmatter = "var π = Math.PI;\nvar ಠ_ಠ = eval;\nvar ლ_ಠ益ಠ_ლ = 42;\nvar λ = function() {};\nvar Ꙭൽↈⴱ = 'huh';\nvar 〱〱 = 2;\nvar Ⅳ = 4;";
        let input = format!("---\n{frontmatter}\n---\n\n<div></div>\n");
        let actual = convert_to_tsx(&input, ConvertOptions::default()).code;
        assert!(
            actual.contains(frontmatter),
            "non-latin frontmatter was not round-tripped verbatim:\n{actual}"
        );
    }

    #[test]
    fn handles_complex_generics() {
        let input = "---\nimport type { GetStaticPaths, MDXInstance } from \"$data/shared\";\n\nexport const getStaticPaths: GetStaticPaths = async () => {\n  const articles = await Astro.glob<Article>(\"/content/articles/**/*.mdx\");\n  return articles.map((article) => {\n    return { params: { slug: getSlugFromFile(article.file) } };\n  });\n};\n\nexport interface Props {\n  article: MDXInstance<Article>;\n}\n\nconst { article } = Astro.props;\n---\n\n<ArticleLayout article={article} />";
        let result = convert_to_tsx(input, ConvertOptions::default());
        assert!(
            result.code.contains("MDXInstance<Article>"),
            "complex generics were not preserved:\n{}",
            result.code
        );
        assert!(
            !result.has_parse_errors,
            "complex generics should parse cleanly"
        );
    }

    #[test]
    fn frontmatter_survives_closing_tags_in_its_code() {
        for input in [
            "---\nconst a = \"</script>\";\n---\n<p>x</p>",
            "---\nconst a = `</style>`;\n---\n<p>x</p>",
            "---\n// </script> in a comment\n---\n<p>x</p>",
        ] {
            let result = convert_to_tsx(input, ConvertOptions::default());
            assert_eq!(
                result.frontmatter.status,
                crate::FrontmatterStatus::Closed,
                "frontmatter ended early for {input:?}"
            );
            assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
            assert!(result.code.contains("<p>x</p>"), "body lost for {input:?}");
        }
    }

    // Route-specific component names keep language-server auto-imports distinct.
    #[test]
    fn dynamic_routes_keep_their_component_name() {
        for (filename, expected) in [
            ("src/pages/[slug].astro", "_Slug_AstroComponent"),
            ("src/pages/my-comp.astro", "MyCompAstroComponent"),
            ("src/pages/404.astro", "FourOhFourAstroComponent"),
            ("src/pages/[...path].astro", "AstroComponent"),
        ] {
            let code = convert_to_tsx(
                "<div/>",
                ConvertOptions {
                    filename: Some(filename.to_string()),
                    ..Default::default()
                },
            )
            .code;
            assert!(
                code.contains(&format!("function {expected}(")),
                "{filename} did not produce {expected}:\n{code}"
            );
        }
    }

    #[test]
    fn generated_get_static_paths_types_are_valid_tsx() {
        let input =
            "---\nexport const getStaticPaths = () => ([\n  { params: { id: '1' } }\n])\n---\n<p/>";
        let result = convert_to_tsx(input, ConvertOptions::default());

        assert!(!result.has_parse_errors, "{:?}", result.diagnostics);
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
        assert_mapped_runs_are_verbatim(input, &result, "getStaticPaths types");
    }

    #[test]
    fn bare_return_stays_terminal_for_narrowing() {
        let bare = convert_to_tsx(
            "---\nif (cond) {\n\treturn;\n}\nreturn\n---\n<p/>",
            ConvertOptions::default(),
        )
        .code;
        assert!(bare.contains("throw undefined;"), "{bare}");
        assert!(bare.contains("throw undefined\n"), "{bare}");
        assert!(!bare.contains("throw ;"), "{bare}");
        assert!(!bare.contains("void 0"), "{bare}");

        let valued = convert_to_tsx(
            "---\nif (cond) return Astro.redirect('/x');\n---\n<p/>",
            ConvertOptions::default(),
        )
        .code;
        assert!(valued.contains("throw  Astro.redirect"), "{valued}");
        assert!(!valued.contains("return Astro"), "{valued}");
    }

    #[test]
    fn returns_in_default_exported_functions_are_preserved() {
        let actual = convert_to_tsx(
            "---\nexport default function f() { return 1 }\n---\n",
            ConvertOptions::default(),
        )
        .code;
        assert!(actual.contains("function f() { return 1 }"), "{actual}");
        assert!(!actual.contains("function f() { throw  1 }"), "{actual}");
    }

    #[test]
    fn variable_length_return_rewrites_keep_runs_verbatim() {
        let source =
            "---\nconst é = 1;\nif (é) {\n\treturn;\n}\nconst after = é;\n---\n<p>{after}</p>";
        let result = convert_to_tsx(source, ConvertOptions::default());
        assert!(result.code.contains("throw undefined;"), "{}", result.code);
        assert!(result.code.contains("const after = é;"), "{}", result.code);
        assert_mapped_runs_are_verbatim(source, &result, "bare return drift");
    }

    #[test]
    fn ambient_types_are_appended_only_on_request() {
        let source = "---\nconst title = Astro.props.title;\n---\n<h1>{title}</h1>";

        let plain_result = convert_to_tsx(source, ConvertOptions::default());
        let plain = &plain_result.code;
        assert!(!plain.contains("declare const Fragment"), "{plain}");
        assert!(!plain.contains("declare const Astro"), "{plain}");

        let ambient_result = convert_to_tsx(
            source,
            ConvertOptions {
                ambient_types: true,
                ..Default::default()
            },
        );
        let ambient = &ambient_result.code;
        assert!(
            ambient.contains("declare const Fragment: any;\n"),
            "{ambient}"
        );
        assert!(
            ambient.contains(
                "declare const Astro: Readonly<import('astro').AstroGlobal<Record<string, any>, typeof AstroComponent>>"
            ),
            "{ambient}"
        );
        assert!(ambient.starts_with(plain));
        assert_eq!(ambient_result.mappings, plain_result.mappings);
    }

    #[test]
    fn ambient_types_never_declare_astro_twice() {
        let source = "---\ninterface Props { title: string }\n---\n<h1>{Astro.props.title}</h1>";
        let ambient = convert_to_tsx(
            source,
            ConvertOptions {
                ambient_types: true,
                ..Default::default()
            },
        )
        .code;
        assert_eq!(
            ambient.matches("declare const Astro").count(),
            1,
            "{ambient}"
        );
        assert!(
            ambient.contains("AstroGlobal<Props, typeof AstroComponent>"),
            "the Props-aware declaration must win:\n{ambient}"
        );
        assert_eq!(
            ambient.matches("declare const Fragment").count(),
            1,
            "{ambient}"
        );
    }

    #[test]
    fn unicode_filenames_still_name_their_component() {
        let code = convert_to_tsx(
            "<div/>",
            ConvertOptions {
                filename: Some("src/components/Ünicorn.astro".to_string()),
                ..Default::default()
            },
        )
        .code;
        assert!(code.contains("function ÜnicornAstroComponent("), "{code}");
    }
}
