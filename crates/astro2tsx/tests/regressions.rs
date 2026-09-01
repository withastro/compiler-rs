//! Assertions over `convert_to_tsx` output for a spread of Astro inputs.

mod common;

use astro2tsx::{ConvertOptions, convert_to_tsx};
use common::assert_mapped_runs_are_verbatim;

const PREFIX: &str = "/* @jsxImportSource astro */\n\n";

fn convert(source: &str) -> String {
    convert_to_tsx(source, ConvertOptions::default()).code
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
fn body_range_is_recorded() {
    let result = convert_to_tsx("<h1>Hi</h1>", ConvertOptions::default());
    assert!(result.body.end > result.body.start);
    let body_slice = &result.code[result.body.start as usize..result.body.end as usize];
    assert!(body_slice.contains("<h1>"));
    assert!(body_slice.contains("</h1>"));
}

#[test]
fn unparseable_expression_bodies_are_not_silent() {
    let broken = convert_to_tsx("<div>{x ==}</div>", ConvertOptions::default());
    assert!(broken.has_parse_errors, "raw fallback must flag the result");

    let empty = convert_to_tsx("<div>{}</div>", ConvertOptions::default());
    assert!(!empty.has_parse_errors, "an empty expression is fine");
}

#[test]
fn parser_errors_surface_but_do_not_block_emission() {
    // Unbalanced curly brace — bogus text expression recovery.
    let result = convert_to_tsx(
        "---\nconst items = [1];\n---\n{items.map(i => <div>{i}</div>)",
        ConvertOptions::default(),
    );
    assert!(result.has_parse_errors);
    assert!(result.code.starts_with(PREFIX));
}

/// Excluded bodies must stay reachable through the extracted arrays.
#[test]
fn unclosed_raw_text_element_still_accounts_for_its_content() {
    let raw = convert("<div is:raw>lost\n<p>after</p>");
    assert!(
        raw.contains("lost") && raw.contains("after"),
        "is:raw content should stay inline:\n{raw}"
    );

    let style = convert_to_tsx(
        "<style>.a{color:red}\n<div>after</div>",
        ConvertOptions::default(),
    );
    assert!(!style.code.contains("color:red"), "{}", style.code);
    assert_eq!(style.styles.len(), 1);
    assert!(style.styles[0].content.starts_with(".a{color:red}"));

    let script = convert_to_tsx(
        "<script type=\"application/json\">{\"a\":1}",
        ConvertOptions::default(),
    );
    assert!(!script.code.contains("\"a\":1"), "{}", script.code);
    assert_eq!(script.scripts.len(), 1);
    assert_eq!(script.scripts[0].content, "{\"a\":1}");
}

#[test]
fn unclosed_style_still_extracts_its_css() {
    let result = convert_to_tsx("<style>.a{color:red}", ConvertOptions::default());
    assert_eq!(result.styles.len(), 1);
    assert!(result.styles[0].content.contains(".a{color:red}"));
}

#[test]
fn valueless_expression_attribute_keeps_a_value() {
    let actual = convert("<div @click={} />");
    assert!(actual.contains("{...{\"@click\":true}}"), "{actual}");
}

#[test]
fn spread_object_entries_are_comma_separated_without_a_leading_comma() {
    let actual = convert("<div @click={} @other={} />");
    assert!(
        actual.contains("{...{\"@click\":true,\"@other\":true}}"),
        "{actual}"
    );

    // A dotted directive reaches the spread but emits no entry of its own.
    for input in [
        "<div client:load.foo @z={} />",
        "<div @z={} client:load.foo />",
    ] {
        let actual = convert(input);
        assert!(
            actual.contains("{...{\"@z\":true}}"),
            "stray separator for {input:?}:\n{actual}"
        );
    }
}

#[test]
fn expression_string_literals_keep_their_value() {
    for (input, expected) in [
        ("<div>{\"<br/>\"}</div>", "{\"<br/>\"}"),
        ("<div>{'<b>y</b>'}</div>", "{'<b>y</b>'}"),
        ("<div>{`<b>y</b>`}</div>", "{`<b>y</b>`}"),
    ] {
        let actual = convert(input);
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
        let actual = convert(input);
        assert!(
            actual.contains(expected),
            "generics were treated as markup for {input:?}:\n{actual}"
        );
    }
}

#[test]
fn only_adjacent_siblings_are_wrapped_in_a_fragment() {
    let adjacent = convert("<div>{c && <span>a</span> <span>b</span>}</div>");
    assert!(
        adjacent.contains("{c && <Fragment><span>a</span> <span>b</span></Fragment>}"),
        "adjacent siblings were not wrapped:\n{adjacent}"
    );

    for input in [
        "<div>{c && <span>a</span>}</div>",
        "<div>{l.map(i => <span>{i}</span>)}</div>",
    ] {
        let actual = convert(input);
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
        let actual = convert(input);
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

#[test]
fn get_static_paths_needs_a_real_export() {
    let mentioned = convert("---\n// see getStaticPaths in the docs\nexport const x = 1;\n---\n");
    assert!(
        !mentioned.contains("ASTRO__InferredGetStaticPath"),
        "a mention injected the inferred-props machinery:\n{mentioned}"
    );
    assert!(
        mentioned.contains("_props: Record<string, any>"),
        "a mention changed the _props type:\n{mentioned}"
    );

    let referenced = convert("---\nexport const handler = getStaticPaths;\n---\n");
    assert!(
        !referenced.contains("ASTRO__InferredGetStaticPath"),
        "a reference is not an export:\n{referenced}"
    );

    for input in [
        "---\nexport const getStaticPaths = async () => [];\n---\n",
        "---\nexport async function getStaticPaths() { return []; }\n---\n",
        "---\nconst paths = async () => [];\nexport { paths as getStaticPaths };\n---\n",
    ] {
        let actual = convert(input);
        assert!(
            actual.contains("ASTRO__InferredGetStaticPath"),
            "missed a real export for {input:?}:\n{actual}"
        );
    }
}

#[test]
fn extracted_ranges_are_generated_offsets() {
    let styled = convert_to_tsx("<div style=\"color:red\"></div>", ConvertOptions::default());
    let range = styled.styles[0].range;
    assert_eq!(
        &styled.code[range.start as usize..range.end as usize],
        "color:red"
    );

    let scripted = convert_to_tsx("<div onclick=\"go()\"></div>", ConvertOptions::default());
    let range = scripted.scripts[0].range;
    assert_eq!(
        &scripted.code[range.start as usize..range.end as usize],
        "go()"
    );

    // An excluded body leaves an empty range between the tags.
    let tag = convert_to_tsx("<style>.a{color:red}</style>", ConvertOptions::default());
    let range = tag.styles[0].range;
    assert_eq!(range.start, range.end);
    assert!(tag.code[..range.start as usize].ends_with("<style>"));
    assert!(tag.code[range.end as usize..].starts_with("</style>"));
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
        let actual = convert(input);
        assert_eq!(
            actual.contains("_props: Props"),
            has_props,
            "wrong Props detection for {input:?}:\n{actual}"
        );
    }
}

#[test]
fn frontmatter_is_terminated_even_when_a_comment_ends_with_a_semicolon() {
    let actual = convert("---\nconst x = foo\n// note;\n---\n<div/>");
    assert!(
        actual.contains("{};<Fragment>"),
        "`<Fragment>` can continue the unterminated expression:\n{actual}"
    );
}

#[test]
fn comments_before_the_first_element_survive() {
    for input in [
        "<!-- leading -->\n<div>x</div>",
        "---\nconst a = 1;\n---\n<!-- between -->\n<div>x</div>",
    ] {
        let actual = convert(input);
        assert!(
            actual.contains("{/** leading */}") || actual.contains("{/** between */}"),
            "a leading comment was dropped for {input:?}:\n{actual}"
        );
        assert!(!actual.contains("<!--"), "untranslated comment:\n{actual}");
    }
}

#[test]
fn extracted_tags_carry_source_ranges() {
    let source = "---\nconst x = 1;\n---\n<style>.a{color:red}</style>\n<div onclick=\"go()\" style=\"color:red\"></div>\n<script>run();</script>";
    let result = convert_to_tsx(source, ConvertOptions::default());

    let slice = |range: astro2tsx::SourceRange| &source[range.start as usize..range.end as usize];
    assert_eq!(slice(result.styles[0].source), ".a{color:red}");
    assert_eq!(slice(result.styles[1].source), "color:red");
    assert_eq!(slice(result.scripts[0].source), "go()");
    assert_eq!(slice(result.scripts[1].source), "run();");
}

#[test]
fn frontmatter_status_and_source_are_reported() {
    use astro2tsx::FrontmatterStatus;

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
fn diagnostics_carry_positions_pointing_at_the_problem() {
    let source = "<div>{x ==}</div>";
    let result = convert_to_tsx(source, ConvertOptions::default());
    assert!(!result.diagnostics.is_empty());
    for diagnostic in &result.diagnostics {
        assert!(!diagnostic.message.is_empty());
        assert!(
            diagnostic.source.end as usize <= source.len(),
            "{diagnostic:?} runs past the source"
        );
        assert!(diagnostic.source.start <= diagnostic.source.end);
    }
}

#[test]
fn tolerates_line_separators() {
    for input in [
        "\u{2028}",
        "something\u{2029}something",
        "something\u{2028}\u{2029}",
        "\u{2028}\u{2029}\u{2028}",
    ] {
        let actual = convert(input);
        assert!(
            actual.starts_with(PREFIX),
            "input {input:?} produced no usable output"
        );
    }
}

#[test]
fn handles_non_latin_identifiers() {
    let frontmatter = "var π = Math.PI;\nvar ಠ_ಠ = eval;\nvar ლ_ಠ益ಠ_ლ = 42;\nvar λ = function() {};\nvar Ꙭൽↈⴱ = 'huh';\nvar 〱〱 = 2;\nvar Ⅳ = 4;";
    let input = format!("---\n{frontmatter}\n---\n\n<div></div>\n");
    let actual = convert(&input);
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
fn malformed_input_never_panics_in_the_parser() {
    for input in ["<div></{<//", "<a></", "</", "<//", "<div></{", "{<//}"] {
        let result = convert_to_tsx(input, ConvertOptions::default());
        assert!(result.code.starts_with(PREFIX), "no output for {input:?}");
    }
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
            astro2tsx::FrontmatterStatus::Closed,
            "frontmatter ended early for {input:?}"
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.code.contains("<p>x</p>"), "body lost for {input:?}");
    }
}

#[test]
fn components_are_never_treated_as_html_script_or_style() {
    for input in [
        "<Script>alert(1)</Script>",
        "<Style>.a{color:red}</Style>",
        "{cond && <Script>alert(1)</Script>}",
    ] {
        let result = convert_to_tsx(input, ConvertOptions::default());
        assert!(
            result.scripts.is_empty() && result.styles.is_empty(),
            "{input:?}"
        );
        assert!(
            result.code.contains("alert(1)") || result.code.contains("color:red"),
            "component body was dropped for {input:?}:\n{}",
            result.code
        );
    }
}

#[test]
fn extracted_tag_sources_slice_to_their_content() {
    let source = "---\nconst x = 1;\n---\n<style>.a{color:red}</style>\n<div onclick=\"go()\" style=color:red data-x='a\"b'></div>\n<script>run();</script>";
    let result = convert_to_tsx(source, ConvertOptions::default());
    assert!(!result.scripts.is_empty() && !result.styles.is_empty());
    for tag in result.scripts.iter().chain(result.styles.iter()) {
        assert_eq!(
            &source[tag.source.start as usize..tag.source.end as usize],
            tag.content,
            "{tag:?} does not slice back to its content"
        );
    }
}

/// Distinct names per route file; the language server keys auto-imports on them.
#[test]
fn dynamic_routes_keep_their_component_name() {
    for (filename, expected) in [
        ("src/pages/[slug].astro", "Slug__AstroComponent_"),
        ("src/pages/my-comp.astro", "MyComp__AstroComponent_"),
        // Names that cannot be identifiers fall back to the bare placeholder.
        ("src/pages/404.astro", "__AstroComponent_"),
        ("src/pages/[...path].astro", "__AstroComponent_"),
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
        let actual = convert(input);
        assert!(
            !actual.contains("<!"),
            "the doctype survived for {input:?}:\n{actual}"
        );
    }

    let actual = convert("<!doctype html>\n<html><body>x</body></html>");
    assert!(
        actual.contains("<html>"),
        "the html element was lost:\n{actual}"
    );

    let sibling = convert("<div>hi</div>\n<!DOCTYPE html>\n<p>after</p>");
    assert!(
        sibling.contains("<p>after</p>"),
        "content after a stray doctype was lost:\n{sibling}"
    );
}

#[test]
fn unclosed_elements_are_emitted_as_written_and_flagged() {
    for (input, expected) in [
        ("<div>hello", "<div>hello"),
        ("<Card>\n  <p>text</p>", "<Card>\n  <p>text</p>"),
        ("<p>one<p>two", "<p>one<p>two"),
        ("<>{1}<p>x</p>", "<>{1}<p>x</p>"),
        (
            "<table><tr><td>a<td>b</tr></table>",
            "<td>a<td>b</tr></table>",
        ),
    ] {
        let result = convert_external(input);
        assert!(
            result.code.contains(expected),
            "content lost for {input:?}:\n{}",
            result.code
        );
        assert!(
            !result.code.contains("</div>") || input.contains("</div>"),
            "a closing tag was synthesized for {input:?}:\n{}",
            result.code
        );
        assert!(result.has_parse_errors, "{input:?} must stay flagged");
        assert_mapped_runs_are_verbatim(input, &result, "unclosed as written");
    }
}

/// The one invariant every consumer leans on, checked against every fixture.
#[test]
fn every_fixture_keeps_mapped_runs_verbatim() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "astro") {
            continue;
        }
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let raw = std::fs::read_to_string(&path).unwrap();
        let (source, options) = common::parse_fixture(&raw);
        let result = convert_to_tsx(
            &source,
            ConvertOptions {
                sourcemap: astro2tsx::SourceMapMode::External,
                ..options
            },
        );
        assert_mapped_runs_are_verbatim(&source, &result, &name);
        checked += 1;
    }
    assert!(
        checked > 50,
        "expected to check most fixtures, got {checked}"
    );
}

fn convert_external(source: &str) -> astro2tsx::ConvertResult {
    convert_to_tsx(
        source,
        ConvertOptions {
            sourcemap: astro2tsx::SourceMapMode::External,
            ..Default::default()
        },
    )
}

#[test]
fn bare_less_than_in_text_is_escaped() {
    for input in ["<p>a < b</p>", "<div>5 < 10 is true</div>"] {
        let result = convert_external(input);
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
fn bare_return_stays_terminal_for_narrowing() {
    let bare = convert("---\nif (cond) {\n\treturn;\n}\nreturn\n---\n<p/>");
    assert!(bare.contains("throw undefined;"), "{bare}");
    assert!(bare.contains("throw undefined\n"), "{bare}");
    assert!(!bare.contains("throw ;"), "{bare}");
    assert!(!bare.contains("void 0"), "{bare}");

    let valued = convert("---\nif (cond) return Astro.redirect('/x');\n---\n<p/>");
    assert!(valued.contains("throw  Astro.redirect"), "{valued}");
    assert!(!valued.contains("return Astro"), "{valued}");
}

#[test]
fn variable_length_return_rewrites_keep_runs_verbatim() {
    let source = "---\nconst é = 1;\nif (é) {\n\treturn;\n}\nconst after = é;\n---\n<p>{after}</p>";
    let result = convert_external(source);
    assert!(result.code.contains("throw undefined;"), "{}", result.code);
    assert!(result.code.contains("const after = é;"), "{}", result.code);
    assert_mapped_runs_are_verbatim(source, &result, "bare return drift");
}

#[test]
fn single_quoted_attributes_round_trip_with_their_quotes() {
    let source = "<div data-x='a\"b' title='plain'></div>";
    let result = convert_external(source);
    assert!(result.code.contains("data-x='a\"b'"), "{}", result.code);
    assert_mapped_runs_are_verbatim(source, &result, "single quotes");
}

#[test]
fn raw_template_escapes_are_present_but_unmapped() {
    let source = "<div is:raw>a`b ${x}</div>";
    let result = convert_external(source);
    assert!(result.code.contains("{`a\\`b \\${x}`}"), "{}", result.code);
    assert_mapped_runs_are_verbatim(source, &result, "raw escapes");
}

#[test]
fn multiline_tag_headers_keep_their_whitespace() {
    let source = "<Comp\n  foo={bar}\n/>";
    let result = convert_external(source);
    assert!(result.code.contains("foo={bar}\n/>"), "{}", result.code);
    assert_mapped_runs_are_verbatim(source, &result, "multiline tag");
}

#[test]
fn extracted_attribute_ranges_slice_to_content_with_inner_quotes() {
    let result = convert_to_tsx(
        "<div style='a:\"x\"' onclick='go(\"y\")'></div>",
        ConvertOptions::default(),
    );
    let style = &result.styles[0];
    assert_eq!(
        &result.code[style.range.start as usize..style.range.end as usize],
        style.content
    );
    let script = &result.scripts[0];
    assert_eq!(
        &result.code[script.range.start as usize..script.range.end as usize],
        script.content
    );
}

#[test]
fn jsx_path_extracts_event_and_style_attributes() {
    let source = "{x && <div onclick=\"go()\" style=\"color:red\"></div>}";
    let result = convert_to_tsx(source, ConvertOptions::default());
    assert_eq!(result.scripts.len(), 1, "{}", result.code);
    assert_eq!(result.scripts[0].content, "go()");
    assert_eq!(result.styles.len(), 1, "{}", result.code);
    assert_eq!(result.styles[0].content, "color:red");
    for tag in result.scripts.iter().chain(result.styles.iter()) {
        assert_eq!(
            &result.code[tag.range.start as usize..tag.range.end as usize],
            tag.content,
            "generated range does not slice to content"
        );
        assert_eq!(
            &source[tag.source.start as usize..tag.source.end as usize],
            tag.content,
            "source range does not slice to content"
        );
    }
}

#[test]
fn script_types_reflect_what_is_statically_knowable() {
    use astro2tsx::ExtractedScriptType;

    for (input, expected) in [
        (
            "<script>const a = 1;</script>",
            ExtractedScriptType::ProcessedModule,
        ),
        (
            "<script type=\"module\">x</script>",
            ExtractedScriptType::Module,
        ),
        ("<script is:inline>x</script>", ExtractedScriptType::Inline),
        (
            "<script type={mime}>wat</script>",
            ExtractedScriptType::Unknown,
        ),
        (
            "{x && <script type={mime}>wat</script>}",
            ExtractedScriptType::Unknown,
        ),
    ] {
        let result = convert_to_tsx(input, ConvertOptions::default());
        assert_eq!(
            result.scripts.first().and_then(|tag| tag.script_type),
            Some(expected),
            "wrong script type for {input:?}"
        );
    }
}

#[test]
fn stripping_the_doctype_keeps_mapped_runs_verbatim() {
    let source = "---\nconst é = 1;\n---\n\n<!doctype html>\n<html lang=en data-x=\"𝒳\"><body>{é}</body></html>\n";
    for ambient_types in [false, true] {
        let result = convert_to_tsx(
            source,
            ConvertOptions {
                sourcemap: astro2tsx::SourceMapMode::External,
                ambient_types,
                ..Default::default()
            },
        );
        assert!(!result.code.contains("<!"), "{}", result.code);
        assert_mapped_runs_are_verbatim(source, &result, "doctype");
    }
}

#[test]
fn ambient_types_are_appended_only_on_request() {
    let source = "---\nconst title = Astro.props.title;\n---\n<h1>{title}</h1>";

    let plain = convert(source);
    assert!(!plain.contains("declare const Fragment"), "{plain}");
    assert!(!plain.contains("declare const Astro"), "{plain}");

    let ambient = convert_to_tsx(
        source,
        ConvertOptions {
            ambient_types: true,
            ..Default::default()
        },
    )
    .code;
    assert!(
        ambient.contains("declare const Fragment: any;\n"),
        "{ambient}"
    );
    assert!(
        ambient.contains(
            "declare const Astro: Readonly<import('astro').AstroGlobal<Record<string, any>, typeof __AstroComponent_>>"
        ),
        "{ambient}"
    );
}

/// A frontmatter `Props` already declares `Astro`; a second one would conflict.
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
        ambient.contains("AstroGlobal<Props, typeof __AstroComponent_>"),
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
    assert!(
        code.contains("function Ünicorn__AstroComponent_("),
        "{code}"
    );
}
