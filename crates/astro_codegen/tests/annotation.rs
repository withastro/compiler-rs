use astro_codegen::{TransformOptions, transform};
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn compile(source: &str, annotate: bool) -> String {
    compile_with_filename(source, annotate, Some("/src/pages/index.astro"))
}

fn compile_with_filename(source: &str, annotate: bool, filename: Option<&str>) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, SourceType::astro()).parse_astro();
    assert!(ret.errors.is_empty(), "Parse errors: {:?}", ret.errors);

    let mut options = TransformOptions::new()
        .with_internal_url("http://localhost:3000/")
        .with_annotate_source_file(annotate);
    if let Some(filename) = filename {
        options = options.with_filename(filename);
    }

    transform(&allocator, source, options, &ret.root).code
}

#[test]
fn annotates_html_elements_when_enabled() {
    let output = compile("<h1>Hello</h1>", true);

    assert!(output.contains(
        "<h1 data-astro-source-file=\"/src/pages/index.astro\" data-astro-source-loc=\"1:5\">Hello</h1>"
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
        "<span data-astro-source-file=\"/src/pages/index.astro\" data-astro-source-loc=\"1:9\">first</span>"
    ));
    assert!(output.contains(
        "<div data-astro-source-file=\"/src/pages/index.astro\" data-astro-source-loc=\"2:8\">second</div>"
    ));
}

#[test]
fn uses_first_child_location_and_tag_name_for_empty_elements() {
    let output = compile(
        "<div><span>x</span></div><p></p><aside><!-- c -->x</aside>",
        true,
    );

    assert!(output.contains("data-astro-source-loc=\"1:7\"><span"));
    assert!(output.contains("data-astro-source-loc=\"1:12\">x</span>"));
    assert!(output.contains(
        "<p data-astro-source-file=\"/src/pages/index.astro\" data-astro-source-loc=\"1:27\"></p>"
    ));
    assert!(output.contains("data-astro-source-loc=\"1:44\"><!-- c -->x</aside>"));
}

#[test]
fn safely_escapes_filename_and_skips_annotations_without_a_filename() {
    let escaped = compile_with_filename("<div>x</div>", true, Some("/src/a&\".astro"));
    assert!(escaped.contains("data-astro-source-file=\"/src/a&amp;&quot;.astro\""));

    let missing = compile_with_filename("<div>x</div>", true, None);
    assert!(!missing.contains("data-astro-source-file"));
    assert!(!missing.contains("data-astro-source-loc"));
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

    assert!(output.contains("\"data-astro-source-file\": \"/src/pages/index.astro\""));
    assert!(output.contains("\"data-astro-source-loc\": \"1:2\""));
}
