use astro_codegen::{TransformOptions, transform};
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn compile(source: &str, annotate: bool) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, SourceType::astro()).parse_astro();
    assert!(ret.errors.is_empty(), "Parse errors: {:?}", ret.errors);

    let options = TransformOptions::new()
        .with_internal_url("http://localhost:3000/")
        .with_filename("/src/pages/index.astro")
        .with_annotate_source_file(annotate);

    transform(&allocator, source, options, &ret.root).code
}

#[test]
fn annotates_html_elements_when_enabled() {
    let output = compile("<h1>Hello</h1>", true);

    assert!(output.contains(
        "<h1 data-astro-source-file=\"/src/pages/index.astro\" data-astro-source-loc=\"1:0\">Hello</h1>"
    ));
}

#[test]
fn leaves_html_elements_unannotated_when_disabled() {
    let output = compile("<h1>Hello</h1>", false);

    assert!(!output.contains("data-astro-source-file"));
    assert!(!output.contains("data-astro-source-loc"));
}

#[test]
fn reports_multiline_and_utf16_source_locations() {
    let output = compile("😀<span>first</span>\n  <div>second</div>", true);

    assert!(output.contains(
        "<span data-astro-source-file=\"/src/pages/index.astro\" data-astro-source-loc=\"1:2\">first</span>"
    ));
    assert!(output.contains(
        "<div data-astro-source-file=\"/src/pages/index.astro\" data-astro-source-loc=\"2:2\">second</div>"
    ));
}

#[test]
fn skips_components_and_never_scoped_elements() {
    let source = r#"---
import Component from "./Component.astro";
---
<html><head><title>Title</title></head><Component /><div>Body</div></html>"#;
    let output = compile(source, true);

    assert_eq!(output.matches("data-astro-source-file").count(), 1);
    assert!(output.contains("<div data-astro-source-file=\"/src/pages/index.astro\""));
}

#[test]
fn annotates_custom_elements() {
    let output = compile("<my-element>Hi</my-element>", true);

    assert!(output.contains("\"data-astro-source-file\":\"/src/pages/index.astro\""));
    assert!(output.contains("\"data-astro-source-loc\":\"1:0\""));
}
