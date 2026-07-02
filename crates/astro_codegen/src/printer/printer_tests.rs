// Printer unit tests

use crate::{
    StyleBlock, TransformOptions, extract_styles, printer::tests::compile_astro_with_options,
};
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn compile_astro(source: &str) -> String {
    compile_astro_with_options(
        source,
        TransformOptions::new().with_internal_url("http://localhost:3000/"),
    )
    .code
}

#[test]
fn test_basic_no_frontmatter() {
    let source = "<button>Click</button>";
    let output = compile_astro(source);

    assert!(output.contains("import {"));
    assert!(output.contains("$$createComponent"));
    assert!(output.contains("<button>Click</button>"));
    assert!(output.contains("export default $$Component"));
}

#[test]
fn test_basic_with_frontmatter() {
    let source = r"---
const href = '/about';
---
<a href={href}>About</a>";
    let output = compile_astro(source);

    assert!(
        output.contains("const href = \"/about\""),
        "Missing const declaration"
    );
    assert!(
        output.contains("$$addAttribute(href, \"href\")"),
        "Missing $$addAttribute"
    );
}

#[test]
fn test_component_rendering() {
    let source = r"---
import Component from 'test';
---
<Component />";
    let output = compile_astro(source);

    assert!(
        output.contains("import Component from \"test\""),
        "Missing import"
    );
    assert!(
        output.contains("$$renderComponent"),
        "Missing $$renderComponent"
    );
    assert!(output.contains("\"Component\""), "Missing component name");
}

#[test]
fn test_doctype() {
    let source = "<!DOCTYPE html><div></div>";
    let output = compile_astro(source);

    assert!(output.contains("<div></div>"), "Missing div element");
    assert!(
        output.contains("$$maybeRenderHead"),
        "Missing maybeRenderHead"
    );
}

#[test]
fn test_fragment() {
    let source = "<><div>1</div><div>2</div></>";
    let output = compile_astro(source);

    assert!(
        output.contains("$$renderComponent"),
        "Missing renderComponent"
    );
    assert!(output.contains("Fragment"), "Missing Fragment reference");
    assert!(output.contains("<div>1</div>"), "Missing first div");
    assert!(output.contains("<div>2</div>"), "Missing second div");
}

#[test]
fn test_html_head_body() {
    let source = r"<html>
  <head>
<title>Test</title>
  </head>
  <body>
<h1>Hello</h1>
  </body>
</html>";
    let output = compile_astro(source);

    assert!(
        output.contains("$$renderHead($$result)"),
        "Missing renderHead in head"
    );
    assert!(output.contains("<title>Test</title>"), "Missing title");
    assert!(output.contains("<h1>Hello</h1>"), "Missing h1");
}

#[test]
fn test_expression_in_attribute() {
    let source = r#"---
const src = "image.png";
---
<img src={src} alt="test" />"#;
    let output = compile_astro(source);

    assert!(
        output.contains("$$addAttribute(src, \"src\")"),
        "Missing dynamic src attribute"
    );
    assert!(
        output.contains("alt=\"test\""),
        "Missing static alt attribute"
    );
}

#[test]
fn test_expression_in_content() {
    let source = r#"---
const name = "World";
---
<h1>Hello {name}!</h1>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("Hello ${name}!"),
        "Missing interpolated expression"
    );
}

#[test]
fn test_slots_basic() {
    let source = r#"---
import Component from "test";
---
<Component>
<div>Default</div>
<div slot="named">Named</div>
</Component>"#;
    let output = compile_astro(source);

    assert!(output.contains("\"default\":"), "Missing default slot");
    assert!(output.contains("\"named\":"), "Missing named slot");
}

#[test]
fn test_conditional_slot() {
    let source = r#"---
import Component from "test";
---
<Component>{value && <div slot="test">foo</div>}</Component>"#;
    let output = compile_astro(source);

    assert!(output.contains("\"test\":"), "Missing named slot 'test'");
    assert!(
        !output.contains("slot=\"test\""),
        "Slot attribute should be removed from element"
    );
    assert!(
        output.contains("<div>foo</div>"),
        "Missing div element without slot attr"
    );
}

#[test]
fn test_expression_slot_multiple() {
    let source = r#"---
import Component from "test";
---
<Component>{true && <div slot="a">A</div>}{false && <div slot="b">B</div>}</Component>"#;
    let output = compile_astro(source);

    assert!(output.contains("\"a\":"), "Missing named slot 'a'");
    assert!(output.contains("\"b\":"), "Missing named slot 'b'");
    assert!(
        !output.contains("\"default\":"),
        "Should not have default slot"
    );
}

#[test]
fn test_slot_same_named_direct_and_expression_group_into_one_key() {
    // A direct element and an expression slot with the same name render under a
    // single slot key (bodies concatenated), not a duplicate object key where JS
    // drops all but the last. Matches how same-named expression slots already
    // group, and the Go compiler.
    let source = r#"---
import Component from "test";
---
<Component><div slot="x">A</div>{b && <div slot="x">B</div>}</Component>"#;
    let output = compile_astro(source);

    assert!(
        output.matches("\"x\":").count() == 1,
        "Mixed direct and expression slots should produce one slot key, not duplicates: {output}"
    );
    assert!(
        output.contains("<div>A</div>") && output.contains("b &&"),
        "Both the direct element and the branch body should survive in the merged slot: {output}"
    );
}

#[test]
fn test_slot_same_named_mixed_keeps_source_order() {
    // The merged slot body concatenates siblings in source order, so an expression
    // slot authored before a direct element renders before it.
    let source = r#"---
import Component from "test";
---
<Component>{b && <div slot="x">B</div>}<div slot="x">A</div></Component>"#;
    let output = compile_astro(source);

    let expr_pos = output.find("b &&").expect("expression slot missing");
    let direct_pos = output.find("<div>A</div>").expect("direct element missing");
    assert!(
        expr_pos < direct_pos,
        "Merged slot body should keep source order (expression before direct element): {output}"
    );
}

#[test]
fn test_client_load_directive() {
    let source = r"---
import Component from 'test';
---
<Component client:load />";
    let output = compile_astro(source);

    assert!(
        output.contains("client:component-hydration") && output.contains("load"),
        "Missing hydration directive"
    );
}

#[test]
fn test_void_elements() {
    let source = r#"<meta charset="utf-8"><input type="text"><br><img src="x.png"><link rel="stylesheet" href="style.css"><hr>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("<meta charset=\"utf-8\">"),
        "Missing meta tag"
    );
    assert!(
        output.contains("<input type=\"text\">"),
        "Missing input tag"
    );
    assert!(output.contains("<br>"), "Missing br tag");
    assert!(output.contains("<img"), "Missing img tag");
    assert!(output.contains("<link"), "Missing link tag");
    assert!(output.contains("<hr>"), "Missing hr tag");

    assert!(
        !output.contains("</meta>"),
        "Found </meta> - void elements should not have closing tags"
    );
    assert!(
        !output.contains("</input>"),
        "Found </input> - void elements should not have closing tags"
    );
    assert!(
        !output.contains("</br>"),
        "Found </br> - void elements should not have closing tags"
    );
    assert!(
        !output.contains("</img>"),
        "Found </img> - void elements should not have closing tags"
    );
    assert!(
        !output.contains("</link>"),
        "Found </link> - void elements should not have closing tags"
    );
    assert!(
        !output.contains("</hr>"),
        "Found </hr> - void elements should not have closing tags"
    );
}

#[test]
fn test_no_maybe_render_head_with_explicit_head() {
    let source = r"<html>
  <head>
<title>Test</title>
  </head>
  <body>
<main>
  <h1>Hello</h1>
</main>
  </body>
</html>";
    let output = compile_astro(source);

    assert!(
        output.contains("$$renderHead($$result)"),
        "Missing $$renderHead in head"
    );

    assert!(
        !output.contains("$$maybeRenderHead("),
        "Body should not have $$maybeRenderHead when explicit <head> exists"
    );
}

#[test]
fn test_head_elements_skip_maybe_render_head() {
    let source = r#"<Component /><link href="style.css"><meta charset="utf-8"><script src="app.js"></script>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("<link href=\"style.css\">"),
        "Missing link element"
    );
    assert!(
        output.contains("<meta charset=\"utf-8\">"),
        "Missing meta element"
    );

    assert!(
        !output.contains("$$maybeRenderHead("),
        "Head elements should not trigger $$maybeRenderHead"
    );
}

#[test]
fn test_custom_element() {
    let source = r#"<my-element foo="bar"></my-element>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("$$renderComponent"),
        "Custom elements should use $$renderComponent"
    );
    assert!(
        output.contains("\"my-element\"") && output.matches("\"my-element\"").count() >= 2,
        "Custom element should have tag name as both display name and quoted identifier"
    );

    assert!(
        !output.contains("$$maybeRenderHead("),
        "Custom elements should not trigger $$maybeRenderHead"
    );

    assert!(
        !output.contains("<my-element"),
        "Custom elements should not be rendered as HTML tags"
    );
}

#[test]
fn test_html_comments_preserved() {
    let source = r#"<!-- Global Metadata -->
<meta charset="utf-8">
<!-- Another comment -->
<link rel="icon" href="/favicon.ico" />"#;
    let output = compile_astro(source);

    assert!(
        output.contains("<!-- Global Metadata -->"),
        "Missing first HTML comment"
    );
    assert!(
        output.contains("<!-- Another comment -->"),
        "Missing second HTML comment"
    );
    assert!(
        output.contains("<meta charset=\"utf-8\">"),
        "Missing meta tag"
    );
    assert!(
        output.contains("<link rel=\"icon\" href=\"/favicon.ico\">"),
        "Missing link tag"
    );
}

// === Metadata tests ===

#[test]
fn test_hydrated_component_metadata_default_import() {
    let source = r#"---
import One from "../components/one.jsx";
---
<One client:load />"#;
    let result = compile_astro_with_options(
        source,
        TransformOptions::new().with_internal_url("http://localhost:3000/"),
    );

    assert_eq!(result.hydrated_components.len(), 1);
    let c = &result.hydrated_components[0];
    assert_eq!(c.export_name, "default");
    assert_eq!(c.local_name, "One");
    assert_eq!(c.specifier, "../components/one.jsx");
}

#[test]
fn test_hydrated_component_metadata_named_import() {
    let source = r#"---
import { Three } from "../components/three.tsx";
---
<Three client:load />"#;
    let result = compile_astro_with_options(
        source,
        TransformOptions::new().with_internal_url("http://localhost:3000/"),
    );

    assert_eq!(result.hydrated_components.len(), 1);
    let c = &result.hydrated_components[0];
    assert_eq!(c.export_name, "Three");
    assert_eq!(c.local_name, "Three");
    assert_eq!(c.specifier, "../components/three.tsx");
}

#[test]
fn test_hydrated_component_metadata_namespace_dot_notation() {
    let source = r#"---
import * as Two from "../components/two.jsx";
---
<Two.someName client:load />"#;
    let result = compile_astro_with_options(
        source,
        TransformOptions::new().with_internal_url("http://localhost:3000/"),
    );

    assert_eq!(result.hydrated_components.len(), 1);
    let c = &result.hydrated_components[0];
    assert_eq!(c.export_name, "someName");
    assert_eq!(c.local_name, "Two.someName");
    assert_eq!(c.specifier, "../components/two.jsx");
}

#[test]
fn test_hydrated_component_metadata_namespace_deep_dot_notation() {
    let source = r#"---
import * as four from "../components/four.jsx";
---
<four.nested.deep.Component client:load />"#;
    let result = compile_astro_with_options(
        source,
        TransformOptions::new().with_internal_url("http://localhost:3000/"),
    );

    assert_eq!(result.hydrated_components.len(), 1);
    let c = &result.hydrated_components[0];
    assert_eq!(c.export_name, "nested.deep.Component");
    assert_eq!(c.local_name, "four.nested.deep.Component");
    assert_eq!(c.specifier, "../components/four.jsx");
}

#[test]
fn test_client_only_component_metadata() {
    let source = r#"---
import Five from "../components/five.jsx";
---
<Five client:only="react" />"#;
    let result = compile_astro_with_options(
        source,
        TransformOptions::new().with_internal_url("http://localhost:3000/"),
    );

    assert_eq!(result.client_only_components.len(), 1);
    let c = &result.client_only_components[0];
    assert_eq!(c.export_name, "default");
    assert_eq!(c.local_name, "Five");
    assert_eq!(c.specifier, "../components/five.jsx");
}

#[test]
fn test_client_only_component_metadata_named() {
    let source = r#"---
import { Named } from "../components/named.jsx";
---
<Named client:only="react" />"#;
    let result = compile_astro_with_options(
        source,
        TransformOptions::new().with_internal_url("http://localhost:3000/"),
    );

    assert_eq!(result.client_only_components.len(), 1);
    let c = &result.client_only_components[0];
    assert_eq!(c.export_name, "Named");
    assert_eq!(c.local_name, "Named");
    assert_eq!(c.specifier, "../components/named.jsx");
}

#[test]
fn test_client_only_component_metadata_star_export() {
    let source = r#"---
import * as Five from "../components/five.jsx";
---
<Five.someName client:only />"#;
    let result = compile_astro_with_options(
        source,
        TransformOptions::new().with_filename("/users/astro/apps/pacman/src/pages/index.astro"),
    );

    assert_eq!(result.client_only_components.len(), 1);
    let c = &result.client_only_components[0];
    assert_eq!(c.export_name, "someName");
    assert_eq!(c.specifier, "../components/five.jsx");
}

#[test]
fn test_client_only_component_metadata_deep_nested() {
    let source = r#"---
import * as eight from "../components/eight.jsx";
---
<eight.nested.deep.Component client:only />"#;
    let result = compile_astro_with_options(
        source,
        TransformOptions::new().with_filename("/users/astro/apps/pacman/src/pages/index.astro"),
    );

    assert_eq!(result.client_only_components.len(), 1);
    let c = &result.client_only_components[0];
    assert_eq!(c.export_name, "nested.deep.Component");
    assert_eq!(c.specifier, "../components/eight.jsx");
}

#[test]
fn test_server_deferred_component_metadata() {
    let source = r#"---
import Avatar from "../components/Avatar.jsx";
import { Other } from "../components/Other.jsx";
---
<Avatar server:defer />
<Other server:defer />"#;
    let result = compile_astro_with_options(
        source,
        TransformOptions::new().with_internal_url("http://localhost:3000/"),
    );

    assert_eq!(result.server_components.len(), 2);

    let c0 = &result.server_components[0];
    assert_eq!(c0.export_name, "default");
    assert_eq!(c0.local_name, "Avatar");
    assert_eq!(c0.specifier, "../components/Avatar.jsx");

    let c1 = &result.server_components[1];
    assert_eq!(c1.export_name, "Other");
    assert_eq!(c1.local_name, "Other");
    assert_eq!(c1.specifier, "../components/Other.jsx");

    assert!(result.propagation, "server:defer should enable propagation");
}

#[test]
fn test_contains_head_metadata() {
    let source = r"<html>
<head><title>Test</title></head>
<body><p>content</p></body>
</html>";
    let result = compile_astro_with_options(
        source,
        TransformOptions::new().with_internal_url("http://localhost:3000/"),
    );

    assert!(result.contains_head, "Should detect explicit <head>");
}

#[test]
fn test_no_head_metadata() {
    let source = "<p>no head here</p>";
    let result = compile_astro_with_options(
        source,
        TransformOptions::new().with_internal_url("http://localhost:3000/"),
    );

    assert!(
        !result.contains_head,
        "Should not detect <head> when absent"
    );
}

#[test]
fn test_resolve_path_filepath_join_fallback() {
    let source = r#"---
import Counter from "../components/Counter.jsx";
---
<Counter client:load />"#;
    let result = compile_astro_with_options(
        source,
        TransformOptions::new()
            .with_internal_url("http://localhost:3000/")
            .with_filename("src/pages/index.astro"),
    );

    assert_eq!(result.hydrated_components.len(), 1);
    let c = &result.hydrated_components[0];
    assert_eq!(c.specifier, "../components/Counter.jsx");
    assert_eq!(c.resolved_path, "src/components/Counter.jsx");
}

#[test]
fn test_resolve_path_custom_function() {
    let source = r#"---
import Counter from "../components/Counter.jsx";
---
<Counter client:load />"#;
    let result = compile_astro_with_options(
        source,
        TransformOptions::new()
            .with_internal_url("http://localhost:3000/")
            .with_filename("src/pages/index.astro")
            .with_resolve_path(|specifier| format!("/resolved{specifier}")),
    );

    assert_eq!(result.hydrated_components.len(), 1);
    let c = &result.hydrated_components[0];
    assert_eq!(c.resolved_path, "/resolved../components/Counter.jsx");

    assert!(
        !result.code.contains("$$createMetadata"),
        "Should skip $$createMetadata"
    );
    assert!(
        !result.code.contains("$$metadata"),
        "Should skip $$metadata export"
    );
    assert!(
        !result.code.contains("$$module1"),
        "Should skip $$module imports"
    );
}

#[test]
fn test_resolve_path_bare_specifier_fallback() {
    let source = r#"---
import Counter from "some-package";
---
<Counter client:load />"#;
    let result = compile_astro_with_options(
        source,
        TransformOptions::new()
            .with_internal_url("http://localhost:3000/")
            .with_filename("src/pages/index.astro"),
    );

    assert_eq!(result.hydrated_components.len(), 1);
    assert_eq!(result.hydrated_components[0].resolved_path, "some-package");
}

#[test]
fn test_server_defer_skips_attribute() {
    let source = r"---
import Avatar from './Avatar.jsx';
---
<Avatar server:defer />";
    let result = compile_astro_with_options(
        source,
        TransformOptions::new().with_internal_url("http://localhost:3000/"),
    );

    assert!(
        !result.code.contains("\"server:defer\""),
        "server:defer should be stripped from props"
    );
}

#[test]
fn test_typescript_satisfies_stripped() {
    let source = r"---
interface SEOProps { title: string; }
const seo = { title: 'Hello' } satisfies SEOProps;
---
<h1>{seo.title}</h1>";
    let output = compile_astro(source);

    assert!(
        !output.contains("satisfies"),
        "satisfies keyword should be stripped: {output}"
    );
    assert!(
        !output.contains("interface SEOProps"),
        "interface should be stripped: {output}"
    );
    assert!(
        output.contains("title: \"Hello\"") || output.contains("title: 'Hello'"),
        "value expression should remain: {output}"
    );
}

#[test]
fn test_type_only_import_stripped() {
    let source = r"---
import type { Props } from './types';
const x: Props = { title: 'hi' };
---
<h1>{x.title}</h1>";
    let output = compile_astro(source);

    assert!(
        !output.contains("import type"),
        "import type should be stripped: {output}"
    );
}

// === Bug #2 regression: semicolons after return in slot-aware statements ===

#[test]
fn test_slot_aware_return_has_semicolon() {
    // A ternary in a component child that routes to named slots triggers
    // print_slot_aware_statement → ReturnStatement. The return must end
    // with a semicolon to avoid ASI hazards.
    let source = r#"---
import Component from "test";
---
<Component>{(() => {
  if (condition) {
return <div slot="a">A</div>;
  }
  return <div slot="b">B</div>;
})()}</Component>"#;
    let output = compile_astro(source);

    // The compiled output should not contain "return " followed by something
    // without a semicolon before the newline. We check that every "return "
    // in the output has a matching ";" on the same statement.
    assert!(
        !output.contains("return \n"),
        "Return statement should not be followed by bare newline (ASI hazard): {output}"
    );
}

// === Bug #3 regression: transition:persist-props on HTML elements ===

#[test]
fn test_transition_persist_props_html_element() {
    // transition:persist-props on an HTML element should produce a simple
    // rename to data-astro-transition-persist-props, NOT trigger
    // $$createTransitionScope hash generation in the template body.
    let source = r#"<div transition:persist-props="all">content</div>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("data-astro-transition-persist-props"),
        "Should rename transition:persist-props to data-astro-transition-persist-props: {output}"
    );
    // $$createTransitionScope appears in the import, but should NOT appear
    // in the template body (i.e. inside the $$render`` template literal).
    let template_start = output.find("$$render`").unwrap();
    let template_body = &output[template_start..];
    assert!(
        !template_body.contains("$$createTransitionScope("),
        "transition:persist-props should NOT invoke $$createTransitionScope in template: {output}"
    );
}

#[test]
fn test_transition_persist_and_persist_props_together() {
    // When both transition:persist and transition:persist-props are on the
    // same element, persist should get its normal handling and persist-props
    // should be a simple rename.
    let source = r#"<div transition:persist transition:persist-props="all">content</div>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("data-astro-transition-persist"),
        "Should have data-astro-transition-persist: {output}"
    );
    assert!(
        output.contains("data-astro-transition-persist-props"),
        "Should rename transition:persist-props: {output}"
    );
}

#[test]
fn test_transitions_animation_url_option() {
    // When transitionsAnimationURL is provided, the compiler should use it
    // instead of the default "transitions.css" bare specifier.
    let source = r#"<div transition:persist>content</div>"#;
    let result = compile_astro_with_options(
        source,
        TransformOptions::new()
            .with_internal_url("http://localhost:3000/")
            .with_transitions_animation_url("astro/transitions.css"),
    );

    assert!(
        result.code.contains(r#"import "astro/transitions.css";"#),
        "Should use the provided transitionsAnimationURL: {}",
        result.code
    );
    assert!(
        !result.code.contains(r#"import "transitions.css";"#),
        "Should NOT use the default transitions.css when URL is provided: {}",
        result.code
    );
}

#[test]
fn test_transitions_default_url_without_option() {
    // When transitionsAnimationURL is NOT provided, fall back to "transitions.css".
    let source = r#"<div transition:persist>content</div>"#;
    let output = compile_astro(source);

    assert!(
        output.contains(r#"import "transitions.css";"#),
        "Should use default transitions.css when no URL option is provided: {output}"
    );
}

// === CSS scope injection tests ===

#[test]
fn test_dynamic_class_with_scoped_styles_no_extra_brace() {
    // When a dynamic class expression is used on an element with scoped styles,
    // the output should not have an extra closing brace leaking into HTML.
    // Regression: `class}=""` was produced due to `}}}}` in format string.
    let source = r#"---
const myClass = "foo";
---
<svg class={myClass}><path d="M0 0"/></svg>
<style>svg { color: red; }</style>"#;
    let output = compile_astro(source);

    // The template literal expression should end with `)}` not `)}}`
    assert!(
        !output.contains("}}"),
        "Should not have double closing braces in template expression: {output}"
    );
    // And there should be no `class}` in the output
    assert!(
        !output.contains("class}"),
        "Should not have malformed class}} attribute: {output}"
    );
    // The $$addAttribute call for class should be well-formed
    assert!(
        output.contains(r#", "class")"#),
        "Should have well-formed $$addAttribute call for class: {output}"
    );
}

#[test]
fn test_component_static_class_merged_with_scope() {
    // When a component has a static class="foo" and scoped styles are active,
    // the scope class should be merged into the value: "class":"foo astro-HASH"
    // NOT two separate "class" keys (which would lose the first one in JS).
    let source = r#"---
import Comp from './Comp.astro';
---
<Comp class="hide" />
<style>div { color: red; }</style>"#;
    let output = compile_astro(source);

    // Should have merged class value like "hide astro-XXXX"
    assert!(
        output.contains(r#""hide astro-"#),
        "Component static class should be merged with scope class: {output}"
    );
    // Should NOT have two separate "class" keys
    let class_count = output.matches(r#""class""#).count();
    assert_eq!(
        class_count, 1,
        "Should have exactly one \"class\" key, got {class_count}: {output}"
    );
}

#[test]
fn test_component_dynamic_class_merged_with_scope() {
    // When a component has a dynamic class={expr} and scoped styles are active,
    // the scope class should be merged: "class":(((expr) ?? "") + " astro-HASH")
    let source = r#"---
import Comp from './Comp.astro';
const cls = "foo";
---
<Comp class={cls} />
<style>div { color: red; }</style>"#;
    let output = compile_astro(source);

    // Should have the ?? "" + " astro-" pattern
    assert!(
        output.contains(r#"?? "") + " astro-"#),
        "Component dynamic class should use nullish coalescing with scope class: {output}"
    );
    // Should NOT have two separate "class" keys
    let class_count = output.matches(r#""class""#).count();
    assert_eq!(
        class_count, 1,
        "Should have exactly one \"class\" key, got {class_count}: {output}"
    );
}

#[test]
fn test_component_no_class_gets_scope_class() {
    // When a component has no class attribute and scoped styles are active,
    // a separate "class":"astro-HASH" should be added.
    let source = r#"---
import Comp from './Comp.astro';
---
<Comp variant="primary" />
<style>div { color: red; }</style>"#;
    let output = compile_astro(source);

    assert!(
        output.contains(r#""astro-"#),
        "Component without class should get scope class prop: {output}"
    );
    // Should have exactly one "class" key
    let class_count = output.matches(r#""class""#).count();
    assert_eq!(
        class_count, 1,
        "Should have exactly one \"class\" key, got {class_count}: {output}"
    );
}

// === Test 3: Adversarial/edge-case input tests ===

#[test]
fn test_template_literal_injection_in_text() {
    // Backticks in text content should be escaped since output
    // is inside a template literal.
    let source = r"<div>some `backtick` text</div>";
    let output = compile_astro(source);

    assert!(
        output.contains("\\`backtick\\`"),
        "Backticks in text should be escaped: {output}"
    );
}

#[test]
fn test_empty_frontmatter() {
    let source = "---\n---\n<p>hello</p>";
    let output = compile_astro(source);

    assert!(
        output.contains("<p>hello</p>"),
        "Should render content after empty frontmatter: {output}"
    );
    assert!(
        output.contains("$$createComponent"),
        "Should still create component wrapper: {output}"
    );
}

#[test]
fn test_empty_frontmatter_no_template() {
    // Edge case: empty frontmatter with no template content
    let source = "---\n---\n";
    let output = compile_astro(source);

    assert!(
        output.contains("$$createComponent"),
        "Should still create component wrapper: {output}"
    );
}

#[test]
fn test_deeply_nested_ternary_in_expression() {
    let source = r#"<div>{a ? b ? c ? "deep" : "d" : "e" : "f"}</div>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("deep"),
        "Should handle deeply nested ternary: {output}"
    );
}

#[test]
fn test_html_entity_in_text_content() {
    // HTML entities in text should be decoded when used in expressions
    let source = "<div>&lt;script&gt;</div>";
    let output = compile_astro(source);

    // The text content should appear in the template literal
    assert!(
        output.contains("&lt;script&gt;") || output.contains("<script>"),
        "Should handle HTML entities in text: {output}"
    );
}

#[test]
fn test_attribute_with_special_characters() {
    let source = r#"<div data-value="a&b<c>d&quot;e"></div>"#;
    let output = compile_astro(source);

    // The attribute should be preserved or properly escaped
    assert!(
        output.contains("data-value="),
        "Should include the attribute: {output}"
    );
}

// === Test 5: JSX statement coverage (for/while/try/throw in JSX) ===

#[test]
fn test_for_statement_in_jsx_expression() {
    let source = r"<div>{(() => {
  const items = [];
  for (let i = 0; i < 3; i++) {
items.push(i);
  }
  return items;
})()}</div>";
    let output = compile_astro(source);

    assert!(
        output.contains("for"),
        "Should handle for statement in JSX: {output}"
    );
}

#[test]
fn test_while_statement_in_jsx_expression() {
    let source = r"<div>{(() => {
  let i = 0;
  while (i < 3) {
i++;
  }
  return i;
})()}</div>";
    let output = compile_astro(source);

    assert!(
        output.contains("while"),
        "Should handle while statement in JSX: {output}"
    );
}

#[test]
fn test_try_catch_in_jsx_expression() {
    let source = r#"<div>{(() => {
  try {
return riskyCall();
  } catch (e) {
return "fallback";
  }
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("try") && output.contains("catch"),
        "Should handle try/catch in JSX: {output}"
    );
}

#[test]
fn test_try_catch_with_component_return() {
    let source = r#"---
import Welcome from '../components/Welcome.astro';
import Layout from '../layouts/Layout.astro';
---
<Layout>
	{
		["abc"].map((item) => {
			try {
				return (<Welcome />);
			} catch (error) {
				console.error(error);
				return item;
			}
		})
	}
	<Welcome />
</Layout>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("try") && output.contains("catch"),
        "Should preserve try/catch in JSX expression: {output}"
    );
    assert!(
        output.contains("$$renderComponent") && output.contains("Welcome"),
        "Should render Welcome component inside try block: {output}"
    );
}

#[test]
fn test_try_catch_finally_in_jsx_expression() {
    let source = r#"<div>{(() => {
  try {
return <span>ok</span>;
  } catch (e) {
return <span>err</span>;
  } finally {
cleanup();
  }
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("try") && output.contains("catch") && output.contains("finally"),
        "Should handle try/catch/finally in JSX: {output}"
    );
}

// === Variable declarations with JSX initializers (issue #19) ===

#[test]
fn test_variable_declaration_with_jsx_initializer() {
    let source = r#"<table>
{features.flatMap((feature) => {
  const mainRow = (
<tr><td>{feature.title}</td></tr>
  );
  const detailRows = feature.description
? feature.description.map((line) => (
    <tr><td>{line}</td></tr>
  ))
: [];
  return [mainRow, ...detailRows];
})}
</table>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("= (\n    <tr>") && !output.contains("= (<tr>"),
        "Should not emit raw JSX in variable declaration initializer: {output}"
    );
    assert!(
        output.contains("$$render`<tr>"),
        "Should wrap JSX initializer in $$render: {output}"
    );
}

#[test]
fn test_variable_declaration_with_component_initializer() {
    let source = r#"---
import Comp from './Comp.astro';
---
<div>{(() => {
  const el = <Comp />;
  return el;
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("= <Comp"),
        "Should not emit raw JSX component in variable initializer: {output}"
    );
}

// === Loop and control flow statements with JSX-aware rendering ===

#[test]
fn test_for_statement_with_component_return() {
    let source = r#"---
import Comp from './Comp.astro';
---
<div>{(() => {
  for (let i = 0; i < items.length; i++) {
if (items[i].match) {
  return (<Comp />);
}
  }
  return null;
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("return <Comp"),
        "Should not emit raw JSX in for-loop: {output}"
    );
}

#[test]
fn test_for_in_statement_with_component_return() {
    let source = r#"---
import Comp from './Comp.astro';
---
<div>{(() => {
  for (const key in obj) {
return (<Comp />);
  }
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("return <Comp"),
        "Should not emit raw JSX in for-in: {output}"
    );
}

#[test]
fn test_for_of_statement_with_component_return() {
    let source = r#"---
import Comp from './Comp.astro';
---
<div>{(() => {
  for (const item of items) {
return (<Comp />);
  }
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("return <Comp"),
        "Should not emit raw JSX in for-of: {output}"
    );
}

#[test]
fn test_while_statement_with_component_return() {
    let source = r#"---
import Comp from './Comp.astro';
---
<div>{(() => {
  while (hasNext()) {
return (<Comp />);
  }
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("return <Comp"),
        "Should not emit raw JSX in while-loop: {output}"
    );
}

#[test]
fn test_do_while_statement_with_component_return() {
    let source = r#"---
import Comp from './Comp.astro';
---
<div>{(() => {
  do {
return (<Comp />);
  } while (false);
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("return <Comp"),
        "Should not emit raw JSX in do-while: {output}"
    );
}

#[test]
fn test_labeled_statement_with_component_return() {
    let source = r#"---
import Comp from './Comp.astro';
---
<div>{(() => {
  outer: for (const item of items) {
return (<Comp />);
  }
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("return <Comp"),
        "Should not emit raw JSX in labeled statement: {output}"
    );
}

// === Slot collection from loop/control-flow statements ===
// These use .map() callbacks so the slot collector walks through
// collect_slots_from_call_arguments -> collect_slots_from_function_body ->
// collect_slots_from_statement. Assertions check for "content": (property key)
// to verify the slot was collected as a named slot, not just present in HTML.

#[test]
fn test_slot_collection_for_of_statement() {
    let source = r#"---
import Layout from './Layout.astro';
---
<Layout>
  {items.map((item) => {
for (const x of nested) {
  return (<div slot="content">{x}</div>);
}
  })}
</Layout>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("\"content\":"),
        "Should collect named slot from for-of body: {output}"
    );
}

#[test]
fn test_slot_collection_for_statement() {
    let source = r#"---
import Layout from './Layout.astro';
---
<Layout>
  {items.map((item) => {
for (let i = 0; i < n; i++) {
  return (<div slot="content">{i}</div>);
}
  })}
</Layout>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("\"content\":"),
        "Should collect named slot from for-loop body: {output}"
    );
}

#[test]
fn test_slot_collection_for_in_statement() {
    let source = r#"---
import Layout from './Layout.astro';
---
<Layout>
  {items.map((item) => {
for (const key in obj) {
  return (<div slot="content">{key}</div>);
}
  })}
</Layout>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("\"content\":"),
        "Should collect named slot from for-in body: {output}"
    );
}

#[test]
fn test_slot_collection_while_statement() {
    let source = r#"---
import Layout from './Layout.astro';
---
<Layout>
  {items.map((item) => {
while (hasNext()) {
  return (<div slot="content">item</div>);
}
  })}
</Layout>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("\"content\":"),
        "Should collect named slot from while body: {output}"
    );
}

#[test]
fn test_slot_collection_do_while_statement() {
    let source = r#"---
import Layout from './Layout.astro';
---
<Layout>
  {items.map((item) => {
do {
  return (<div slot="content">item</div>);
} while (false);
  })}
</Layout>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("\"content\":"),
        "Should collect named slot from do-while body: {output}"
    );
}

#[test]
fn test_slot_collection_labeled_statement() {
    let source = r#"---
import Layout from './Layout.astro';
---
<Layout>
  {items.map((item) => {
outer: for (const x of nested) {
  return (<div slot="content">{x}</div>);
}
  })}
</Layout>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("\"content\":"),
        "Should collect named slot from labeled statement body: {output}"
    );
}

#[test]
fn test_slot_collection_try_catch() {
    let source = r#"---
import Layout from './Layout.astro';
---
<Layout>
  {items.map((item) => {
try {
  return (<div slot="content">ok</div>);
} catch (e) {
  return (<div slot="error">{e}</div>);
}
  })}
</Layout>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("\"content\":") && output.contains("\"error\":"),
        "Should collect slots from try and catch blocks as property keys: {output}"
    );
    assert!(
        !output.contains("slot="),
        "Should strip slot attributes when collecting named slots: {output}"
    );
}

// === Parity tests: patterns verified against the Go compiler ===

#[test]
fn test_parity_let_reassignment_with_jsx() {
    // Go: el = $$render`<span>updated</span>`;
    let source = r#"<div>{(() => {
  let el = <span>initial</span>;
  if (cond) {
el = <span>updated</span>;
  }
  return el;
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("= <span"),
        "Should not emit raw JSX in assignment: {output}"
    );
}

#[test]
fn test_parity_nested_map_with_jsx() {
    let source = r#"<div>{items.map((item) => {
  return item.children.map((child) => (
<span>{child}</span>
  ));
})}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("return (<span") && !output.contains("(\n    <span"),
        "Should not emit raw JSX in nested map: {output}"
    );
}

#[test]
fn test_parity_function_expression_with_jsx() {
    // Go: return $$render`<span>from function</span>`;
    let source = r#"<div>{(function() {
  return <span>from function</span>;
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("return <span"),
        "Should not emit raw JSX in function expression: {output}"
    );
}

#[test]
fn test_parity_for_of_push_jsx() {
    // Go: items.push($$render`<li>${x}</li>`);
    let source = r#"<ul>{(() => {
  const items = [];
  for (const x of data) {
items.push(<li>{x}</li>);
  }
  return items;
})()}</ul>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("push(<li>"),
        "Should not emit raw JSX in push() argument: {output}"
    );
}

#[test]
fn test_parity_ternary_jsx_in_variable() {
    // Go: const el = cond ? $$render`<span>yes</span>` : $$render`<span>no</span>`;
    let source = r#"<div>{(() => {
  const el = cond ? <span>yes</span> : <span>no</span>;
  return el;
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("? <span"),
        "Should not emit raw JSX in ternary variable init: {output}"
    );
}

#[test]
fn test_parity_sequence_expression_with_jsx() {
    // Go: (setup(), $$render`<span>result</span>`)
    let source = r#"<div>{(setup(), <span>result</span>)}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains(", <span"),
        "Should not emit raw JSX in sequence expression: {output}"
    );
}

#[test]
fn test_parity_slot_in_ternary_map() {
    // Go: $$mergeSlots({}, hasItems ? items.map(...({"content": ...})) : {"empty": ...})
    let source = r#"---
import Layout from './Layout.astro';
---
<Layout>
  {hasItems
? items.map((item) => (<div slot="content">{item}</div>))
: <div slot="empty">No items</div>
  }
</Layout>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("$$mergeSlots"),
        "Should use $$mergeSlots for ternary with different slots: {output}"
    );
    assert!(
        output.contains("\"content\":") || output.contains("\"content\" :"),
        "Should have content slot as property key: {output}"
    );
    assert!(
        output.contains("\"empty\":") || output.contains("\"empty\" :"),
        "Should have empty slot as property key: {output}"
    );
}

#[test]
fn test_parity_object_with_jsx_values() {
    let source = r#"<div>{(() => {
  return { header: <span>H</span>, body: <span>B</span> };
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains(": <span"),
        "Should not emit raw JSX in object property values: {output}"
    );
}

#[test]
fn test_parity_logical_or_assignment_jsx() {
    let source = r#"<div>{(() => {
  let el;
  el ||= <span>fallback</span>;
  return el;
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("||= <span"),
        "Should not emit raw JSX in logical or assignment: {output}"
    );
}

#[test]
fn test_parity_new_expression_with_jsx_arg() {
    let source = r#"<div>{(() => {
  return new Container(<span>child</span>);
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("Container(<span"),
        "Should not emit raw JSX in new expression arg: {output}"
    );
}

#[test]
fn test_throw_in_jsx_expression() {
    let source = r#"<div>{(() => {
  if (!data) {
throw new Error("missing");
  }
  return data;
})()}</div>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("throw"),
        "Should handle throw in JSX: {output}"
    );
}

// === Test 7: Empty frontmatter variations ===

#[test]
fn test_frontmatter_only_comments() {
    let source = "---\n// just a comment\n---\n<p>hi</p>";
    let output = compile_astro(source);

    assert!(
        output.contains("<p>hi</p>"),
        "Should handle comment-only frontmatter: {output}"
    );
}

#[test]
fn test_no_frontmatter_at_all() {
    let source = "<p>no frontmatter</p>";
    let output = compile_astro(source);

    assert!(
        output.contains("<p>no frontmatter</p>"),
        "Should handle missing frontmatter: {output}"
    );
    assert!(
        output.contains("$$createComponent"),
        "Should still create component: {output}"
    );
}

// === Spread attributes ===

#[test]
fn test_spread_attributes_on_element() {
    let source = r#"---
const props = { class: "foo", id: "bar" };
---
<div {...props}>content</div>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("$$spreadAttributes"),
        "Should use $$spreadAttributes for spread: {output}"
    );
}

// === Boolean and valueless attributes ===

#[test]
fn test_boolean_attribute() {
    let source = r"<input disabled />";
    let output = compile_astro(source);

    assert!(
        output.contains("disabled"),
        "Should handle boolean attribute: {output}"
    );
}

// === set:html and set:text directives ===

#[test]
fn test_set_html_directive() {
    let source = r#"---
const html = "<strong>bold</strong>";
---
<div set:html={html} />"#;
    let output = compile_astro(source);

    assert!(
        output.contains("$$unescapeHTML"),
        "Should use $$unescapeHTML for set:html: {output}"
    );
    // set:html should not appear as an attribute
    assert!(
        !output.contains("set:html="),
        "set:html should be stripped from attributes: {output}"
    );
}

#[test]
fn test_set_text_directive() {
    let source = r#"---
const text = "hello <world>";
---
<div set:text={text} />"#;
    let output = compile_astro(source);

    // set:text should not appear as an attribute
    assert!(
        !output.contains("set:text="),
        "set:text should be stripped from attributes: {output}"
    );
}

// === Conditional rendering patterns ===

#[test]
fn test_logical_and_rendering() {
    let source = r"<div>{show && <p>visible</p>}</div>";
    let output = compile_astro(source);

    assert!(
        output.contains("show") && output.contains("<p>visible</p>"),
        "Should handle logical AND rendering: {output}"
    );
}

#[test]
fn test_ternary_rendering() {
    let source = r"<div>{show ? <p>yes</p> : <p>no</p>}</div>";
    let output = compile_astro(source);

    assert!(
        output.contains("<p>yes</p>") && output.contains("<p>no</p>"),
        "Should handle ternary rendering: {output}"
    );
}

// === Map/iteration patterns ===

#[test]
fn test_array_map_rendering() {
    let source = r#"---
const items = ["a", "b", "c"];
---
<ul>{items.map(item => <li>{item}</li>)}</ul>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("<li>"),
        "Should handle array map rendering: {output}"
    );
}

// === Transition directives ===

#[test]
fn test_transition_name_on_element() {
    let source = r#"<div transition:name="fade">content</div>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("$$renderTransition") || output.contains("data-astro-transition-scope"),
        "Should handle transition:name: {output}"
    );
    assert!(
        output.contains("fade"),
        "Should include transition name: {output}"
    );
}

#[test]
fn test_transition_persist_on_element() {
    let source = r"<div transition:persist>content</div>";
    let output = compile_astro(source);

    assert!(
        output.contains("data-astro-transition-persist")
            || output.contains("$$createTransitionScope"),
        "Should handle transition:persist: {output}"
    );
}

// === Test 4: Slot analysis tests (through full compilation) ===

#[test]
fn test_slot_ternary_multiple_named_slots() {
    // Ternary expression with different slot names on each branch
    // should trigger $$mergeSlots.
    let source = r#"---
import Component from "test";
---
<Component>{cond ? <div slot="a">A</div> : <div slot="b">B</div>}</Component>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("$$mergeSlots"),
        "Ternary with different slot names should use $$mergeSlots: {output}"
    );
}

#[test]
fn test_slot_ternary_same_slot_name() {
    // Two slotted elements merge via $$mergeSlots even with the same slot name,
    // so the runtime conditional decides presence. Matches the Go compiler.
    let source = r#"---
import Component from "test";
---
<Component>{cond ? <div slot="x">A</div> : <span slot="x">B</span>}</Component>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("$$mergeSlots"),
        "Ternary with two same-named slots should use $$mergeSlots: {output}"
    );
    assert!(
        output.matches("\"x\": () =>").count() == 2,
        "Each branch should be wrapped in its own slot object: {output}"
    );
}

#[test]
fn test_slot_all_falsy_nested_ternary_is_runtime_conditional() {
    // An all-falsy nested ternary of slot="aside" elements must route through
    // $$mergeSlots (leaving the slot absent) rather than emit a static key.
    let source = r#"---
import Probe from "test";
---
<Probe>{false ? <span slot="aside">x</span> : false ? <span slot="aside">y</span> : (false && <span slot="aside">z</span>)}</Probe>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("$$mergeSlots"),
        "All-falsy nested ternary should use $$mergeSlots: {output}"
    );
    assert!(
        output.contains("false && { \"aside\":"),
        "The `&&` branch should wrap its slot element so it stays runtime-conditional: {output}"
    );
}

#[test]
fn test_slot_same_named_expression_slots_group_into_one_key() {
    // Two sibling expression slots with the same name render under a single slot
    // key (bodies concatenated), not a duplicate object key where JS drops all but
    // the last. Matches how direct element slots already group, and the Go compiler.
    let source = r#"---
import Component from "test";
---
<Component>{a && <div slot="x">A</div>}{b && <div slot="x">B</div>}</Component>"#;
    let output = compile_astro(source);

    assert!(
        output.matches("\"x\":").count() == 1,
        "Same-named expression slots should produce one slot key, not duplicates: {output}"
    );
    assert!(
        output.contains("a &&") && output.contains("b &&"),
        "Both branch bodies should survive in the merged slot: {output}"
    );
}

#[test]
fn test_slot_toplevel_logical_with_conditional_wraps_branches() {
    // Branches wrap into slot objects, and parens are kept so `a && (b ? X : Y)`
    // does not re-bind as `(a && b) ? X : Y`.
    let source = r#"---
import Component from "test";
---
<Component>{a && (b ? <x slot="s">X</x> : <y slot="s">Y</y>)}</Component>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("a && (b ?"),
        "Parens around the conditional right operand must be preserved: {output}"
    );
    assert!(
        output.matches("\"s\": () =>").count() == 2,
        "Both branches should be wrapped as slot objects: {output}"
    );
}

#[test]
fn test_slot_bare_fragment_children_are_not_parent_slots() {
    // A bare `<>` is opaque: its `slot=` children belong to the fragment, so the
    // expression becomes the parent's default content, not parent slots. Matches Go.
    let source = r#"---
import Component from "test";
---
<Component>{cond && <><a slot="x">A</a><b slot="y">B</b></>}</Component>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("$$mergeSlots"),
        "Bare fragment must not route children onto the parent via $$mergeSlots: {output}"
    );
    assert!(
        output.contains("\"default\":") && output.contains("Fragment"),
        "Bare fragment should be the parent's default content, rendered as a Fragment: {output}"
    );
}

#[test]
fn test_slot_single_logical_and_stays_static_key() {
    // A single slotted element under `&&` keeps a static key, so has() stays true.
    let source = r#"---
import Probe from "test";
---
<Probe>{false && <span slot="aside">z</span>}</Probe>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("$$mergeSlots"),
        "Single slotted element should not need $$mergeSlots: {output}"
    );
    assert!(
        output.contains("\"aside\":"),
        "Single slotted element should keep a static slot key: {output}"
    );
}

#[test]
fn test_slot_logical_and_named() {
    // Logical AND with a named slot
    let source = r#"---
import Component from "test";
---
<Component>{show && <div slot="footer">Footer</div>}</Component>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("\"footer\":"),
        "Logical AND with named slot should produce slot: {output}"
    );
}

#[test]
fn test_slot_default_and_named_together() {
    // Mix of default and named slot children
    let source = r#"---
import Component from "test";
---
<Component>
<p>Default content</p>
<div slot="header">Header</div>
</Component>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("\"default\":"),
        "Should have default slot: {output}"
    );
    assert!(
        output.contains("\"header\":"),
        "Should have header slot: {output}"
    );
}

#[test]
fn test_slot_no_children() {
    // Component with no children — no slots at all
    let source = r#"---
import Component from "test";
---
<Component />"#;
    let output = compile_astro(source);

    // Should not have any slot definitions
    assert!(
        !output.contains("\"default\":") || output.contains("\"default\": () =>"),
        "Self-closing component should have no slots: {output}"
    );
}

#[test]
fn test_slot_dynamic_slot_attribute() {
    // Dynamic slot name: slot={name}
    let source = r#"---
import Component from "test";
const slotName = "dynamic";
---
<Component><div slot={slotName}>Content</div></Component>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("slotName"),
        "Dynamic slot name should reference the variable: {output}"
    );
}

#[test]
fn test_slot_fragment_children() {
    // Fragment as component child
    let source = r#"---
import Component from "test";
---
<Component><>fragment child</></Component>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("fragment child"),
        "Fragment children should be rendered: {output}"
    );
}

// === Test 6: JSXAttributeValue::Element/Fragment paths ===

#[test]
fn test_jsx_element_as_attribute_value_on_component() {
    // JSX element as attribute value on a component renders as "[JSX]"
    let source = r#"---
import Comp from "test";
---
<Comp attr=<span>hi</span> />"#;
    let output = compile_astro(source);

    assert!(
        output.contains("[JSX]"),
        "JSX element as component attribute should render as [JSX]: {output}"
    );
}

#[test]
fn test_jsx_fragment_as_attribute_value_on_component() {
    // JSX fragment as attribute value on a component renders as "[Fragment]"
    let source = r#"---
import Comp from "test";
---
<Comp attr=<>fragment</> />"#;
    let output = compile_astro(source);

    assert!(
        output.contains("[Fragment]"),
        "JSX fragment as component attribute should render as [Fragment]: {output}"
    );
}

// === Test 8: Dynamic slot names on <slot> element ===

#[test]
fn test_slot_element_with_static_name() {
    // <slot name="header" /> should generate a renderSlot call with "header"
    let source = r#"<slot name="header" />"#;
    let output = compile_astro(source);

    assert!(
        output.contains("$$renderSlot") && output.contains("header"),
        "Static <slot name> should use $$renderSlot with 'header': {output}"
    );
}

#[test]
fn test_slot_element_default() {
    // <slot /> without name should use "default"
    let source = "<slot />";
    let output = compile_astro(source);

    assert!(
        output.contains("$$renderSlot") && output.contains("default"),
        "Default <slot /> should use $$renderSlot with 'default': {output}"
    );
}

#[test]
fn test_slot_element_with_dynamic_name() {
    // <slot name={expr} /> should generate a dynamic slot name
    let source = r#"---
const slotName = "dynamic";
---
<slot name={slotName} />"#;
    let output = compile_astro(source);

    assert!(
        output.contains("$$renderSlot"),
        "Dynamic <slot name={{expr}}> should use $$renderSlot: {output}"
    );
    assert!(
        output.contains("slotName"),
        "Dynamic slot name should reference the variable: {output}"
    );
}

#[test]
fn test_slot_element_with_fallback_content() {
    // <slot>fallback</slot> should include fallback content
    let source = "<slot><p>Fallback content</p></slot>";
    let output = compile_astro(source);

    assert!(
        output.contains("$$renderSlot"),
        "Slot with fallback should use $$renderSlot: {output}"
    );
    assert!(
        output.contains("Fallback content"),
        "Fallback content should be present: {output}"
    );
}

// -------------------------------------------------------------------------
// extract_styles tests
// -------------------------------------------------------------------------

fn parse_and_extract_styles(source: &str) -> Vec<StyleBlock> {
    let allocator = Allocator::default();
    let source_type = SourceType::astro();
    let ret = Parser::new(&allocator, source, source_type).parse_astro();
    assert!(ret.errors.is_empty(), "Parse errors: {:?}", ret.errors);
    extract_styles(&ret.root)
}

#[test]
fn test_extract_styles_boolean_attr_is_global() {
    // Boolean attribute `is:global` must be present in attrs with an empty-string
    // value. The caller (TS side) uses `'is:global' in attrs` to detect it, so the
    // key must exist — the value being "" vs true is intentional for the Rust path.
    let source = "<style is:global>h1 { color: red; }</style>";
    let blocks = parse_and_extract_styles(source);

    assert_eq!(blocks.len(), 1, "Should extract one style block");
    let attrs = &blocks[0].attrs;
    let is_global = attrs.iter().find(|(k, _)| k == "is:global");
    assert!(
        is_global.is_some(),
        "is:global key must be present in attrs: {attrs:?}"
    );
    assert_eq!(
        is_global.unwrap().1,
        "",
        "Boolean attr value must be empty string for Rust compiler path: {attrs:?}"
    );
    assert_eq!(
        blocks[0].content.trim(),
        "h1 { color: red; }",
        "Style content should be extracted"
    );
}

#[test]
fn test_extract_styles_quoted_attr_lang() {
    // Quoted attribute `lang="scss"` must round-trip with its value preserved.
    let source = r#"<style lang="scss">$color: red;</style>"#;
    let blocks = parse_and_extract_styles(source);

    assert_eq!(blocks.len(), 1, "Should extract one style block");
    let attrs = &blocks[0].attrs;
    let lang = attrs.iter().find(|(k, _)| k == "lang");
    assert!(lang.is_some(), "lang key must be present: {attrs:?}");
    assert_eq!(
        lang.unwrap().1,
        "scss",
        "lang value must be 'scss': {attrs:?}"
    );
}

#[test]
fn test_extract_styles_is_inline_not_extracted() {
    // `<style is:inline>` must NOT be extracted — it's rendered as-is in HTML.
    let source = "<style is:inline>h1 { color: red; }</style>";
    let blocks = parse_and_extract_styles(source);

    assert_eq!(
        blocks.len(),
        0,
        "is:inline style must not be extracted: {blocks:?}"
    );
}

#[test]
fn test_extract_styles_inside_svg_not_extracted() {
    // Styles inside <svg> are non-hoistable and must not be extracted.
    let source = "<svg><style>circle { fill: red; }</style></svg>";
    let blocks = parse_and_extract_styles(source);

    assert_eq!(
        blocks.len(),
        0,
        "Style inside <svg> must not be extracted: {blocks:?}"
    );
}

#[test]
fn test_extract_styles_inside_noscript_not_extracted() {
    let source = "<noscript><style>h1 { color: red; }</style></noscript>";
    let blocks = parse_and_extract_styles(source);

    assert_eq!(
        blocks.len(),
        0,
        "Style inside <noscript> must not be extracted: {blocks:?}"
    );
}

#[test]
fn test_extract_styles_expression_attr_omitted() {
    // Expression attributes like `define:vars={...}` must be omitted from attrs
    // (they can't be serialised to string), matching the Go compiler's GetAttrs.
    let source = "<style define:vars={{ color: 'red' }}>h1 { color: var(--color); }</style>";
    let blocks = parse_and_extract_styles(source);

    assert_eq!(
        blocks.len(),
        1,
        "Style with define:vars should still be extracted"
    );
    let has_define_vars = blocks[0].attrs.iter().any(|(k, _)| k == "define:vars");
    assert!(
        !has_define_vars,
        "Expression attr define:vars must not appear in attrs: {:?}",
        blocks[0].attrs
    );
}

#[test]
fn test_extract_styles_multiple_blocks_sequential_indices() {
    // Multiple <style> blocks must get sequential indices 0, 1, 2 in document order.
    let source = r"<style>a { color: red; }</style>
<div></div>
<style>b { color: blue; }</style>
<style>c { color: green; }</style>";
    let blocks = parse_and_extract_styles(source);

    assert_eq!(
        blocks.len(),
        3,
        "Should extract three style blocks: {blocks:?}"
    );
    assert_eq!(blocks[0].index, 0);
    assert_eq!(blocks[1].index, 1);
    assert_eq!(blocks[2].index, 2);
    assert!(
        blocks[0].content.contains("red"),
        "First block should have red style"
    );
    assert!(
        blocks[1].content.contains("blue"),
        "Second block should have blue style"
    );
    assert!(
        blocks[2].content.contains("green"),
        "Third block should have green style"
    );
}

#[test]
fn test_extract_styles_multiple_attrs() {
    // A style with both boolean and quoted attrs — both must appear.
    let source = r#"<style is:global lang="scss">h1 { color: red; }</style>"#;
    let blocks = parse_and_extract_styles(source);

    assert_eq!(blocks.len(), 1);
    let attrs = &blocks[0].attrs;
    assert!(
        attrs
            .iter()
            .any(|(k, v): &(String, String)| k == "is:global" && v.is_empty()),
        "is:global must be present with empty value: {attrs:?}"
    );
    assert!(
        attrs
            .iter()
            .any(|(k, v): &(String, String)| k == "lang" && v == "scss"),
        "lang must be present with value 'scss': {attrs:?}"
    );
}

// -------------------------------------------------------------------------
// HTML element directive stripping tests
// -------------------------------------------------------------------------

#[test]
fn test_is_global_stripped_from_html_output() {
    // `is:global` is a compile-time directive and must NOT appear in the
    // rendered HTML string output.
    let source = "<style is:global>h1 { color: red; }</style>";
    let output = compile_astro(source);

    assert!(
        !output.contains("is:global"),
        "is:global must be stripped from compiled output: {output}"
    );
}

#[test]
fn test_is_inline_stripped_from_style_output() {
    // `is:inline` is a compile-time directive and must NOT appear in the
    // rendered HTML string output.
    let source = "<style is:inline>h1 { color: red; }</style>";
    let output = compile_astro(source);

    assert!(
        !output.contains("is:inline"),
        "is:inline must be stripped from compiled output: {output}"
    );
}

#[test]
fn test_is_inline_stripped_from_script_output() {
    let source = "<script is:inline>console.log(1)</script>";
    let output = compile_astro(source);

    assert!(
        !output.contains("is:inline"),
        "is:inline must be stripped from script output: {output}"
    );
    assert!(
        output.contains("console.log(1)"),
        "Inline script content should be in output: {output}"
    );
}

#[test]
fn test_define_vars_stripped_from_style_output() {
    // `define:vars` on a style element must not appear as an HTML attribute.
    let source = "<style define:vars={{ color: 'red' }}>h1 { color: var(--color); }</style>";
    let output = compile_astro(source);

    assert!(
        !output.contains("define:vars"),
        "define:vars must be stripped from style HTML output: {output}"
    );
}

// -------------------------------------------------------------------------
// Component boolean attribute tests — must emit `true` in JS props object
// -------------------------------------------------------------------------

#[test]
fn test_component_boolean_attr_emits_true() {
    // A boolean (valueless) attribute on a component element must be emitted
    // as the JS boolean `true` in the props object, matching Go compiler output.
    let source = r"---
import MyComp from 'test';
---
<MyComp disabled />";
    let output = compile_astro(source);

    assert!(
        output.contains("\"disabled\":true") || output.contains("\"disabled\": true"),
        "Boolean component attr must emit true in props: {output}"
    );
}

#[test]
fn test_component_boolean_attr_is_global_emits_true() {
    // Even Astro-namespace boolean attrs on components must emit `true`, not "".
    let source = r"---
import MyComp from 'test';
---
<MyComp is:global />";
    let output = compile_astro(source);

    assert!(
        output.contains("\"is:global\":true") || output.contains("\"is:global\": true"),
        "Boolean is:global on component must emit true in props: {output}"
    );
}

#[test]
fn test_component_quoted_attr_preserves_value() {
    let source = r#"---
import MyComp from 'test';
---
<MyComp class="foo" />"#;
    let output = compile_astro(source);

    assert!(
        output.contains("\"class\":\"foo\"") || output.contains("\"class\": \"foo\""),
        "Quoted component attr must preserve its value: {output}"
    );
}

// -------------------------------------------------------------------------
// should_hoist_script / scanner tests
// -------------------------------------------------------------------------

#[test]
fn test_script_plain_is_hoisted() {
    // A plain `<script>` with no attributes must be hoisted as inline.
    let source = "<script>console.log('hello')</script>";
    let output = compile_astro(source);

    // Hoisted scripts appear in the `scripts` metadata array, not inline in the template.
    assert!(
        output.contains("\"inline\"") || output.contains("type:\"inline\""),
        "Plain script must be hoisted as inline: {output}"
    );
}

#[test]
fn test_script_type_module_is_hoisted() {
    // `<script type="module">` must be hoisted.
    let source = r#"<script type="module">console.log('hello')</script>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("\"inline\"") || output.contains("type:\"inline\""),
        "type=module script must be hoisted as inline: {output}"
    );
}

#[test]
fn test_script_src_only_is_hoisted_external() {
    // `<script src="...">` with ONLY `src` must be hoisted as external.
    let source = r#"<script src="/script.js"></script>"#;
    let output = compile_astro(source);

    assert!(
        output.contains("\"external\"") || output.contains("type:\"external\""),
        "src-only script must be hoisted as external: {output}"
    );
    assert!(
        output.contains("/script.js"),
        "Hoisted external script must preserve src: {output}"
    );
}

#[test]
fn test_script_src_with_extra_attr_not_hoisted() {
    // `<script src="..." type="module">` must NOT be hoisted (extra attrs).
    // It stays as inline HTML in the template string.
    let source = r#"<script src="/script.js" type="module"></script>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("\"external\""),
        "Script with src+type must not be hoisted as external: {output}"
    );
    // The raw tag should appear in the template string.
    assert!(
        output.contains("/script.js"),
        "Non-hoisted script should still appear in template: {output}"
    );
}

#[test]
fn test_script_is_inline_not_hoisted() {
    // `<script is:inline>` must NEVER be hoisted.
    let source = "<script is:inline>console.log('hello')</script>";
    let output = compile_astro(source);

    assert!(
        !output.contains("type:\"inline\"") && !output.contains("\"inline\":{"),
        "is:inline script must not be hoisted: {output}"
    );
    // Should appear literally in the template.
    assert!(
        output.contains("console.log"),
        "is:inline script content should be in template: {output}"
    );
}

#[test]
fn test_script_inside_html_comment_not_hoisted() {
    // Full no_extra_script_tag fixture content
    let source = r#"<!-- Global Metadata -->
<meta charset="utf-8">
<meta name="viewport" content="width=device-width">

<link rel="icon" type="image/svg+xml" href="/favicon.svg" />
<link rel="alternate icon" type="image/x-icon" href="/favicon.ico" />

<link rel="sitemap" href="/sitemap.xml"/>

<!-- Global CSS -->
<link rel="stylesheet" href="/theme.css" />
<link rel="stylesheet" href="/code.css" />
<link rel="stylesheet" href="/index.css" />

<!-- Preload Fonts -->
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:ital@0;1&display=swap" rel="stylesheet">

<!-- Scrollable a11y code helper -->
<script type="module" src="/make-scrollable-code-focusable.js" />

<!-- This is intentionally inlined to avoid FOUC -->
<script is:inline>
  const root = document.documentElement;
  const theme = localStorage.getItem('theme');
  if (theme === 'dark' || (!theme) && window.matchMedia('(prefers-color-scheme: dark)').matches) {
root.classList.add('theme-dark');
  } else {
root.classList.remove('theme-dark');
  }
</script>

<!-- Global site tag (gtag.js) - Google Analytics -->
<!-- <script async src="https://www.googletagmanager.com/gtag/js?id=G-TEL60V1WM9"></script>
<script>
  window.dataLayer = window.dataLayer || [];
  function gtag(){dataLayer.push(arguments);}
  gtag('js', new Date());
  gtag('config', 'G-TEL60V1WM9');
</script> -->"#;

    let output = compile_astro(source);
    // The commented-out scripts should be inside an HTML comment in the template,
    // NOT in hoisted metadata. Verify hoisted is empty.
    assert!(
        output.contains("hoisted: []"),
        "hoisted should be empty: {output}"
    );
    // Verify the HTML comment containing script tags IS rendered in the template
    assert!(
        output.contains("<!-- <script async"),
        "HTML comment with script tags should be rendered: {output}"
    );
}

#[test]
fn test_script_src_with_data_attr_not_hoisted() {
    // `<script src="..." data-foo="bar">` — any extra attr blocks hoisting.
    let source = r#"<script src="/script.js" data-foo="bar"></script>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("\"external\""),
        "Script with extra data-attr must not be hoisted: {output}"
    );
}

// -------------------------------------------------------------------------
// set:html must always use $$unescapeHTML (including template literals)
// -------------------------------------------------------------------------

#[test]
fn test_set_html_variable_uses_unescape_html() {
    // `set:html={someVar}` must wrap the value in $$unescapeHTML.
    let source = r"---
const html = '<b>bold</b>';
---
<div set:html={html} />";
    let output = compile_astro(source);

    assert!(
        output.contains("$$unescapeHTML"),
        "set:html with variable must use $$unescapeHTML: {output}"
    );
    assert!(
        !output.contains("set:html"),
        "set:html directive must be stripped from output: {output}"
    );
}

#[test]
fn test_set_html_template_literal_uses_unescape_html() {
    // `set:html={`template ${literal}`}` must wrap with $$unescapeHTML.
    // The $$render tagged template escapes HTML by default, so all set:html
    // expressions — including template literals — need $$unescapeHTML to render
    // as raw HTML.
    let source = r#"---
const name = 'world';
---
<div set:html={`Hello <b>${name}</b>`} />"#;
    let output = compile_astro(source);

    assert!(
        output.contains("$$unescapeHTML(`Hello <b>${name}</b>`)"),
        "set:html with template literal must use $$unescapeHTML: {output}"
    );
}

#[test]
fn test_set_html_string_literal_uses_unescape_html() {
    // Even a plain string literal must go through $$unescapeHTML.
    let source = r#"<div set:html={"<b>bold</b>"} />"#;
    let output = compile_astro(source);

    assert!(
        output.contains("$$unescapeHTML"),
        "set:html with string literal must use $$unescapeHTML: {output}"
    );
}

#[test]
fn test_set_text_does_not_use_unescape_html() {
    // `set:text` is the safe (escaped) counterpart — the rendered expression must
    // NOT be wrapped in $$unescapeHTML(). $$unescapeHTML may still appear in the
    // import statement, so we check the template body specifically.
    let source = r"---
const text = '<b>bold</b>';
---
<div set:text={text} />";
    let output = compile_astro(source);

    // The template render string must contain the raw variable, not wrapped.
    assert!(
        output.contains("${text}"),
        "set:text should emit the variable directly: {output}"
    );
    assert!(
        !output.contains("$$unescapeHTML(text)"),
        "set:text must not wrap in $$unescapeHTML(): {output}"
    );
    assert!(
        !output.contains("set:text"),
        "set:text directive must be stripped from output: {output}"
    );
}

// -------------------------------------------------------------------------
// server:defer prop injection tests
// -------------------------------------------------------------------------

#[test]
fn test_server_defer_injects_component_directive() {
    let source = r"---
import MyComp from './MyComp.astro';
---
<MyComp server:defer />";
    let output = compile_astro(source);

    assert!(
        output.contains("server:component-directive"),
        "server:defer must inject server:component-directive: {output}"
    );
}

#[test]
fn test_server_defer_injects_component_path() {
    let source = r"---
import MyComp from './MyComp.astro';
---
<MyComp server:defer />";
    let output = compile_astro(source);

    assert!(
        output.contains("server:component-path"),
        "server:defer must inject server:component-path: {output}"
    );
}

#[test]
fn test_server_defer_injects_component_export() {
    let source = r"---
import MyComp from './MyComp.astro';
---
<MyComp server:defer />";
    let output = compile_astro(source);

    assert!(
        output.contains("server:component-export"),
        "server:defer must inject server:component-export: {output}"
    );
}

#[test]
fn test_server_defer_does_not_set_client_hydration() {
    // server:defer is NOT a client hydration directive — must not emit
    // client:component-hydration or any transition-related props.
    let source = r"---
import MyComp from './MyComp.astro';
---
<MyComp server:defer />";
    let output = compile_astro(source);

    assert!(
        !output.contains("client:component-hydration"),
        "server:defer must not emit client:component-hydration: {output}"
    );
}

// -------------------------------------------------------------------------
// client:only import suppression tests
// -------------------------------------------------------------------------

#[test]
fn test_client_only_import_is_suppressed() {
    // When a component is used exclusively with client:only, its import must be
    // suppressed (the server never needs to load it).
    let source = r"---
import MyComp from 'test';
---
<MyComp client:only='react' />";
    let output = compile_astro(source);

    // The import must be replaced / suppressed — should not appear as a live import.
    assert!(
        !output.contains("import MyComp from \"test\""),
        "client:only import must be suppressed: {output}"
    );
}

#[test]
fn test_non_client_only_import_not_suppressed() {
    // A component used with client:load (not client:only) must keep its import.
    let source = r"---
import MyComp from 'test';
---
<MyComp client:load />";
    let output = compile_astro(source);

    assert!(
        output.contains("import MyComp from \"test\""),
        "client:load import must NOT be suppressed: {output}"
    );
}

#[test]
fn test_mixed_import_not_suppressed_when_also_client_only() {
    // If the SAME import specifier exports one component used normally and another
    // exclusively as client:only, the import must be kept (not suppressed).
    let source = r"---
import { CompA, CompB } from 'test';
---
<CompA />
<CompB client:only='react' />";
    let output = compile_astro(source);

    // CompA is used normally so the import must be preserved.
    assert!(
        output.contains("import") && output.contains("\"test\""),
        "Mixed import must not be suppressed when one specifier is used normally: {output}"
    );
}

#[test]
fn test_client_only_import_kept_when_also_used_plainly() {
    let source = r"---
import MyComp from 'test';
---
<MyComp client:only='react' />
<MyComp />";
    let output = compile_astro(source);
    assert!(
        output.contains("import MyComp from \"test\""),
        "import needed for plain usage must NOT be suppressed: {output}"
    );
}

#[test]
fn test_client_only_import_kept_when_also_used_with_client_load() {
    let source = r"---
import MyComp from 'test';
---
<MyComp client:only='react' />
<MyComp client:load />";
    let output = compile_astro(source);
    assert!(
        output.contains("import MyComp from \"test\""),
        "import needed for client:load usage must NOT be suppressed: {output}"
    );
}

#[test]
fn test_client_only_namespace_import_kept_when_member_used_plainly() {
    let source = r"---
import * as Scope from 'test';
---
<Scope.Foo client:only='react' />
<Scope.Bar />";
    let output = compile_astro(source);
    assert!(
        output.contains("import * as Scope from \"test\""),
        "namespace import needed for Scope.Bar must NOT be suppressed: {output}"
    );
}

#[test]
fn test_client_only_namespace_import_suppressed_when_member_exclusive() {
    let source = r"---
import * as Scope from 'test';
---
<Scope.Foo client:only='react' />";
    let output = compile_astro(source);
    assert!(
        !output.contains("import * as Scope from \"test\""),
        "exclusively-client:only namespace import must be suppressed: {output}"
    );
}

#[test]
fn test_client_only_import_kept_when_referenced_in_frontmatter() {
    let source = r"---
import MyComp from 'test';
const also = MyComp;
---
<MyComp client:only='react' />";
    let output = compile_astro(source);
    assert!(
        output.contains("import MyComp from \"test\""),
        "import referenced in frontmatter must NOT be suppressed: {output}"
    );
}

#[test]
fn test_client_only_import_kept_when_referenced_in_attribute_expression() {
    // The binding is referenced in the client:only element's own props, which are
    // built server-side, so the import must survive.
    let source = r"---
import MyComp from 'test';
---
<MyComp client:only='react' label={MyComp.label} />";
    let output = compile_astro(source);
    assert!(
        output.contains("import MyComp from \"test\""),
        "import referenced in an attribute expression must NOT be suppressed: {output}"
    );
}

#[test]
fn test_client_only_import_kept_when_referenced_in_template_expression() {
    let source = r"---
import MyComp from 'test';
---
<MyComp client:only='react' />
{MyComp.displayName}";
    let output = compile_astro(source);
    assert!(
        output.contains("import MyComp from \"test\""),
        "import referenced in a template expression must NOT be suppressed: {output}"
    );
}

// -------------------------------------------------------------------------
// transition:persist tests
// -------------------------------------------------------------------------

#[test]
fn test_transition_persist_with_explicit_value_on_html_element() {
    // transition:persist="form" on an HTML element should use the string value
    // directly as data-astro-transition-persist, not a generated hash.
    let source = r#"<form transition:persist="form"><input /></form>"#;
    let output = compile_astro(source);

    assert!(
        output.contains(r#"data-astro-transition-persist="form""#),
        "transition:persist with explicit value must use that value: {output}"
    );
    // The output must not contain a $$createTransitionScope( call (only the import alias is ok)
    assert!(
        !output.contains("$$createTransitionScope("),
        "transition:persist with explicit value must not call $$createTransitionScope: {output}"
    );
}

#[test]
fn test_transition_persist_with_transition_name_on_html_element() {
    // transition:persist + transition:name="counter" — persist ID should be "counter"
    let source = r#"<div transition:persist transition:name="counter"></div>"#;
    let output = compile_astro(source);

    assert!(
        output.contains(r#"data-astro-transition-persist="counter""#),
        "transition:persist must use transition:name value as persist ID: {output}"
    );
}

#[test]
fn test_transition_persist_with_explicit_value_on_component() {
    // transition:persist="form" on a component should use the string value directly.
    let source = r#"---
import MyComp from './MyComp.astro';
---
<MyComp transition:persist="form" />"#;
    let output = compile_astro(source);

    assert!(
        output.contains(r#""data-astro-transition-persist": "form""#),
        "component transition:persist with explicit value must use that value: {output}"
    );
    assert!(
        !output.contains("$$createTransitionScope("),
        "component transition:persist with explicit value must not call $$createTransitionScope: {output}"
    );
}

#[test]
fn test_transition_persist_with_transition_name_on_component() {
    // transition:persist + transition:name="counter" on a component.
    let source = r#"---
import MyComp from './MyComp.astro';
---
<MyComp transition:persist transition:name="counter" />"#;
    let output = compile_astro(source);

    assert!(
        output.contains(r#""data-astro-transition-persist": "counter""#),
        "component transition:persist must use transition:name value: {output}"
    );
}

#[test]
fn test_array_expression_in_jsx_transforms_children() {
    // JSX elements inside array expressions must be transformed.
    // Previously they were left as raw JSX in output.
    let source = r#"---
import Foo from './Foo.astro';
import Bar from './Bar.astro';
---
<div>{[<Foo/>, <Bar/>]}</div>"#;
    let output = compile_astro(source);

    assert!(
        !output.contains("<Foo/>") && !output.contains("<Bar/>"),
        "JSX elements inside arrays must be transformed: {output}"
    );
    assert!(
        output.contains("$$renderComponent"),
        "JSX elements inside arrays must use $$renderComponent: {output}"
    );
}

// -- Expression edge whitespace trimming tests --
//
// Implicit JSX fragments inside expressions ({ <el/>... }) should not
// have whitespace-only text nodes at the leading/trailing edges inside
// the $$render backtick. Explicit fragments (<>...</>) preserve their
// whitespace.

#[test]
fn test_expression_implicit_fragment_trims_leading_whitespace() {
    // Leading \n\t before <li> should be trimmed
    let source = "<ul>{\n\t<li>one</li>\n}</ul>";
    let output = compile_astro(source);
    // The backtick should open right before <li>, not with leading whitespace
    assert!(
        output.contains("$$render`<li>one</li>"),
        "implicit fragment should trim leading whitespace: {output}"
    );
}

#[test]
fn test_expression_implicit_fragment_trims_trailing_whitespace() {
    // Trailing \n after </li> should be trimmed
    let source = "<ul>{<li>one</li>\n}</ul>";
    let output = compile_astro(source);
    assert!(
        output.contains("<li>one</li>`"),
        "implicit fragment should trim trailing whitespace: {output}"
    );
}

#[test]
fn test_expression_implicit_fragment_preserves_interior_whitespace() {
    // Whitespace between elements should be preserved
    let source = "<ul>{\n\t<li>one</li>\n\t<li>two</li>\n}</ul>";
    let output = compile_astro(source);
    assert!(
        output.contains("<li>one</li>\n\t<li>two</li>"),
        "implicit fragment should preserve interior whitespace: {output}"
    );
}

#[test]
fn test_expression_explicit_fragment_preserves_all_whitespace() {
    // Explicit <>...</> should keep leading/trailing whitespace
    let source = "<div>{<>\n  <span>hi</span>\n</>}</div>";
    let output = compile_astro(source);
    // The whitespace after <> and before </> should be inside the backtick
    assert!(
        output.contains("$$render`\n  <span>hi</span>\n`"),
        "explicit fragment should preserve all whitespace: {output}"
    );
}

#[test]
fn test_expression_fragment_slot_is_async_with_await() {
    // Regression for #46: await inside a fragment-in-expression must make the slot async.
    let source = "---\nconst item = { cover: '' };\n---\n{item.cover && <>\n  <Image src={item.cover} style={{ ...await getThumb(item.cover) }} />\n</>}";
    let output = compile_astro(source);
    let compact: String = output.split_whitespace().collect();
    assert!(
        compact.contains("\"Fragment\",Fragment,{},{\"default\":async()=>"),
        "fragment slot in expression should be async when await is used: {output}"
    );
}

#[test]
fn test_expression_fragment_in_arrow_slot_is_async_with_await() {
    // #46, other shape: fragment as the body of an async arrow passed to `.map(...)`.
    let source = "---\nconst items = [];\n---\n{items.map(async (item) => <>\n  <img src={item.cover} alt={await describe(item)} />\n</>)}";
    let output = compile_astro(source);
    let compact: String = output.split_whitespace().collect();
    assert!(
        compact.contains("\"Fragment\",Fragment,{},{\"default\":async()=>"),
        "fragment slot in arrow should be async when await is used: {output}"
    );
}

#[test]
fn test_expression_fragment_slot_not_async_without_await() {
    // Without `await` anywhere, the slot stays a plain arrow (no spurious `async`).
    let source = "{cond && <>\n  <span>hi</span>\n</>}";
    let output = compile_astro(source);
    let compact: String = output.split_whitespace().collect();
    assert!(
        compact.contains("\"Fragment\",Fragment,{},{\"default\":()=>"),
        "fragment slot should not be async without await: {output}"
    );
}

#[test]
fn test_async_slot_is_precise_not_file_wide() {
    // A slot is async only if its OWN body awaits, not when the file does. Here
    // await is frontmatter-only, so both slots stay plain (Go marks both async).
    let source = "---\nimport C from 'c';\nconst d = await load();\n---\n<C>plain</C>{cond && <><span>no await</span></>}";
    let output = compile_astro(source);
    let compact: String = output.split_whitespace().collect();
    assert!(
        !compact.contains("\"default\":async()=>"),
        "slots without their own await must not be async: {output}"
    );
    // The wrapper still gets `async` for the top-level frontmatter await.
    assert!(
        output.contains("async ($$result"),
        "wrapper should be async: {output}"
    );
}

#[test]
fn test_async_slot_only_on_awaiting_named_slot() {
    // Named slot "a" awaits; named slot "b" does not. Only "a" should be async.
    let source = "---\nimport C from 'c';\n---\n<C><div slot=\"a\">{await x()}</div><div slot=\"b\">plain</div></C>";
    let output = compile_astro(source);
    let compact: String = output.split_whitespace().collect();
    assert!(
        compact.contains("\"a\":async()=>"),
        "awaiting named slot should be async: {output}"
    );
    assert!(
        compact.contains("\"b\":()=>") && !compact.contains("\"b\":async()=>"),
        "non-awaiting named slot should stay plain: {output}"
    );
}

#[test]
fn test_for_await_frontmatter_makes_wrapper_async() {
    // Regression for #47: for-await needs an async wrapper though it's not an await expression.
    let source = "---\nasync function* g() { yield 1; }\nconst nums = [];\nfor await (const n of g()) { nums.push(n); }\n---\n<h1>{nums.join(',')}</h1>";
    let output = compile_astro(source);
    assert!(
        output.contains("async ($$result"),
        "for-await in frontmatter should make the wrapper async: {output}"
    );
}

#[test]
fn test_await_using_frontmatter_makes_wrapper_async() {
    // await using also needs async, though it's not an await expression.
    let source = "---\nawait using r = getResource();\n---\n<h1>hi</h1>";
    let output = compile_astro(source);
    assert!(
        output.contains("async ($$result"),
        "await-using in frontmatter should make the wrapper async: {output}"
    );
}

#[test]
fn test_async_slot_detects_nested_for_await() {
    // for-await can only sit in a nested function, so the slot is over-marked async by AwaitDetector's subtree scan — not the frontmatter scanner.
    let source = "---\nasync function* stream() { yield 'a'; }\n---\n<Wrapper>{(async () => {\n  let out = '';\n  for await (const chunk of stream()) { out += chunk; }\n  return out;\n})()}</Wrapper>";
    let output = compile_astro(source);
    let compact: String = output.split_whitespace().collect();
    assert!(
        compact.contains("\"default\":async()=>"),
        "slot with nested for-await should be async: {output}"
    );
}

#[test]
fn test_async_slot_detects_nested_await_using() {
    // Same for await-using: nested-only, so the slot is over-marked async via the subtree scan.
    let source = "<Wrapper>{(async () => {\n  await using res = getResource();\n  return res.value;\n})()}</Wrapper>";
    let output = compile_astro(source);
    let compact: String = output.split_whitespace().collect();
    assert!(
        compact.contains("\"default\":async()=>"),
        "slot with nested await-using should be async: {output}"
    );
}

#[test]
fn test_expression_implicit_fragment_inline_elements_no_extra_space() {
    // Inline elements shouldn't get extra whitespace injected
    let source = "<span>{\n  <strong>hello</strong>\n  <em>world</em>\n}</span>";
    let output = compile_astro(source);
    // Leading/trailing \n should be trimmed — no extra space at edges
    assert!(
        output.contains("$$render`<strong>hello</strong>"),
        "should not inject leading whitespace before inline element: {output}"
    );
    assert!(
        output.contains("<em>world</em>`"),
        "should not inject trailing whitespace after inline element: {output}"
    );
}

#[test]
fn test_compact_whitespace_before_style_no_trailing_space() {
    let source = "// @config compact=html\n<a href=\"#\"><slot /></a>\n\n<style>\n  a { color: red; }\n</style>";
    let result = compile_astro_with_options(
        source,
        crate::TransformOptions::new()
            .with_internal_url("http://localhost:3000/")
            .with_compact(crate::CompactMode::Html),
    );
    let output = result.code;
    eprintln!("COMPACT OUTPUT:\n{output}");
    assert!(
        !output.contains("</a> "),
        "Should not have trailing space after </a> when next sibling is extracted style: {output}"
    );
}

#[test]
fn test_whitespace_between_inline_elements_preserved() {
    let source = "<span>hello</span>\n<span>world</span>";
    let output = compile_astro(source);

    // The whitespace between two inline elements must be preserved
    // so they don't collapse into "helloworld"
    assert!(
        output.contains("</span>\n<span>") || output.contains("</span> <span>"),
        "Whitespace between inline elements should be preserved: {output}"
    );
}

#[test]
fn test_no_trailing_whitespace_before_style_noncompact() {
    let source = "<a href=\"#\"><slot /></a>\n\n<style>\n  a { color: red; }\n</style>";
    let output = compile_astro(source);
    assert!(
        !output.contains("</a>\n\n"),
        "Non-compact: should not have trailing whitespace before extracted style: {output}"
    );
}

#[test]
fn test_no_trailing_whitespace_before_style_compact() {
    let source = "<a href=\"#\"><slot /></a>\n\n<style>\n  a { color: red; }\n</style>";
    let result = compile_astro_with_options(
        source,
        crate::TransformOptions::new()
            .with_internal_url("http://localhost:3000/")
            .with_compact(crate::CompactMode::Html),
    );
    assert!(
        !result.code.contains("</a> "),
        "Compact: should not have trailing space before extracted style: {}",
        result.code
    );
}

#[test]
fn test_whitespace_before_style_inside_div() {
    let source =
        "<div>\n  <span>hello</span>\n\n  <style>\n    span { color: red; }\n  </style>\n</div>";
    let output = compile_astro(source);
    // The whitespace between </span> and <style> inside a div
    // should also be stripped since the style is extracted
    assert!(
        !output.contains("</span>\n\n"),
        "Should strip whitespace before extracted style inside div: {output}"
    );
}

#[test]
fn test_whitespace_before_style_all_compact_modes() {
    let source = "<a href=\"#\"><slot /></a>\n\n<style>\n  a { color: red; }\n</style>";

    // compact=false (disabled)
    let output = compile_astro(source);
    assert!(
        !output.contains("</a>\n\n") && !output.contains("</a> "),
        "compact=false: trailing whitespace before style should be stripped: {output}"
    );

    // compact=html
    let result = compile_astro_with_options(
        source,
        crate::TransformOptions::new()
            .with_internal_url("http://localhost:3000/")
            .with_compact(crate::CompactMode::Html),
    );
    assert!(
        !result.code.contains("</a> ") && !result.code.contains("</a>\n"),
        "compact=html: trailing whitespace before style should be stripped: {}",
        result.code
    );

    // compact=jsx
    let result = compile_astro_with_options(
        source,
        crate::TransformOptions::new()
            .with_internal_url("http://localhost:3000/")
            .with_compact(crate::CompactMode::Jsx),
    );
    assert!(
        !result.code.contains("</a> ") && !result.code.contains("</a>\n\n"),
        "compact=jsx: trailing whitespace before style should be stripped: {}",
        result.code
    );
}

#[test]
fn test_span_before_style_no_trailing_space() {
    // From https://github.com/withastro/astro/issues/14593
    let source = "<span>\n  <slot />\n</span>\n\n<style>\n  span { color: red; }\n</style>";
    let output = compile_astro(source);
    assert!(
        !output.contains("</span>\n\n") && !output.contains("</span> "),
        "Should not have trailing whitespace after inline element before style: {output}"
    );
}

#[test]
fn test_fragment_workaround_not_needed() {
    // The fragment workaround from Starlight should produce the same
    // result as the direct version now
    let direct = "<span>\n  <slot />\n</span>\n\n<style>\n  span { color: red; }\n</style>";
    let fragment =
        "<>\n  <span>\n    <slot />\n  </span>\n</>\n\n<style>\n  span { color: red; }\n</style>";

    let direct_output = compile_astro(direct);
    let fragment_output = compile_astro(fragment);

    // Neither should have trailing whitespace
    assert!(
        !direct_output.contains("</span>\n\n") && !direct_output.contains("</span> "),
        "Direct: should not have trailing whitespace: {direct_output}"
    );
    assert!(
        !fragment_output.contains("</span>\n\n") && !fragment_output.contains("</span> "),
        "Fragment: should not have trailing whitespace: {fragment_output}"
    );
}

// Regression test for https://github.com/withastro/compiler-rs/issues/51:
// a non-identifier `{expr}` is a shorthand whose prop name is the expression text.
#[test]
fn test_shorthand_object_attribute() {
    let source = r"---
const sum = (a, b) => a + b;
---
<Debug {{answer: sum(2, 4)}} />";
    let output = compile_astro(source);

    assert!(
        output.contains(r#""{answer: sum(2, 4)}":"#),
        "Missing shorthand prop name derived from the expression, got:\n{output}"
    );
    assert!(
        output.contains("{ answer: sum(2, 4) }"),
        "Missing shorthand prop value, got:\n{output}"
    );
}

// Diverges from Go: the shorthand prop name is escaped so quotes/newlines still emit valid JS.
#[test]
fn test_shorthand_attribute_name_is_escaped() {
    let source = "---\n---\n<Debug {{ \"a\": 1 }} />";
    let output = compile_astro(source);

    assert!(
        output.contains(r#""{ \"a\": 1 }":"#),
        "Shorthand prop name should be escaped, got:\n{output}"
    );

    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &output, SourceType::mjs()).parse();
    assert!(
        parsed.errors.is_empty(),
        "Generated module is not valid JS: {:?}",
        parsed.errors
    );
}

// On a plain element the shorthand goes through `$$addAttribute`, with the same
// name escaping as the component path. A newline in the name must also be escaped.
#[test]
fn test_shorthand_attribute_name_is_escaped_on_element() {
    let source = "---\n---\n<div {{ \"a\": 1 }}></div>";
    let output = compile_astro(source);

    assert!(
        output.contains(r#", "{ \"a\": 1 }")"#),
        "Element shorthand name should be escaped in $$addAttribute, got:\n{output}"
    );

    let multiline = compile_astro("---\n---\n<div {{\n  \"a\": 1\n}}></div>");
    assert!(
        multiline.contains(r#"\n"#),
        "Newline in shorthand name should be escaped, got:\n{multiline}"
    );

    let allocator = Allocator::default();
    for out in [&output, &multiline] {
        let parsed = Parser::new(&allocator, out, SourceType::mjs()).parse();
        assert!(
            parsed.errors.is_empty(),
            "Generated module is not valid JS: {:?}",
            parsed.errors
        );
    }
}

// Unquoted hex-color attributes (`#` + digit) must not error — the JS lexer's
// "Invalid Character" from pre-lexing `#18b218` as a private name is dropped.
#[test]
fn test_unquoted_hash_color_attribute() {
    let output = compile_astro("<div color=#18b218 background=#686868>x</div>");
    assert!(
        output.contains(r##"<div color="#18b218" background="#686868">"##),
        "Unquoted hex colors should emit as quoted attributes, got:\n{output}"
    );
}

#[test]
fn test_jsx_object_literal_lookup() {
    let source = include_str!("../../tests/fixtures/jsx_object_literal_lookup.astro");
    let output = compile_astro(source);

    assert!(
        !output.contains("main: <Widget />"),
        "JSX object literal values should be transformed, got:\n{output}"
    );
    assert!(
        output.contains("$$renderComponent"),
        "Expected transformed JSX in object literal lookup, got:\n{output}"
    );

    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &output, SourceType::mjs()).parse();
    assert!(
        parsed.errors.is_empty(),
        "Generated module is not valid JS: {:?}",
        parsed.errors
    );
}
