//! Assertions over `convert_to_tsx` output for a spread of Astro inputs.

use astro2tsx::{ConvertOptions, convert_to_tsx};

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
    assert!(result.frontmatter.end > result.frontmatter.start);
    let frontmatter_slice =
        &result.code[result.frontmatter.start as usize..result.frontmatter.end as usize];
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

    let tag = convert_to_tsx("<style>.a{color:red}</style>", ConvertOptions::default());
    let range = tag.styles[0].range;
    assert!(
        tag.code
            .get(range.start as usize..range.end as usize)
            .is_some()
    );
}
