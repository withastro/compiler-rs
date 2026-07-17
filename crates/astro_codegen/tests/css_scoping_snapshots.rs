//! Snapshot tests for `:global()` CSS scoping.
//!
//! The glob-based `snapshots.rs` harness only captures the generated JS
//! (`TransformResult::code`), where a top-level `<style>` is replaced by a
//! virtual-module import. The *scoped CSS* itself lives on
//! `TransformResult::css`, so these tests run the full compiler on a small set
//! of representative `:global()` shapes and snapshot the extracted CSS.
//!
//! These are deliberately focused spot-checks of the end-to-end pipeline; the
//! exhaustive per-branch coverage lives in the `css_scoping` unit tests.

use astro_codegen::{ScopedStyleStrategy, TransformOptions, transform};
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

/// Compile an `.astro` source and return the extracted, scoped CSS.
fn scoped_css(source: &str, strategy: ScopedStyleStrategy) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, SourceType::astro()).parse_astro();
    assert!(ret.errors.is_empty(), "parse errors: {:?}", ret.errors);

    let options = TransformOptions::new()
        .with_internal_url("http://localhost:3000/")
        .with_astro_global_args("\"https://astro.build\"")
        .with_scoped_style_strategy(strategy);

    transform(&allocator, source, options, &ret.root)
        .css
        .join("\n")
}

/// Snapshot the scoped CSS with stable, prefix-free snapshot names co-located in
/// `tests/snapshots/`.
macro_rules! assert_css_snapshot {
    ($name:expr, $css:expr) => {
        insta::with_settings!({ prepend_module_to_snapshot => false }, {
            insta::assert_snapshot!($name, $css);
        });
    };
}

/// A `:global()` selector list under a scoped ancestor distributes the scope
/// prefix across every list item.
#[test]
fn global_selector_list_scoped_ancestor() {
    let css = scoped_css(
        "<div class=\"wrapper\"><ul></ul></div><style>.wrapper :global(ul, ol) { margin: 0; }</style>",
        ScopedStyleStrategy::Where,
    );
    assert_css_snapshot!("global_selector_list_scoped_ancestor", css);
}

/// A class/attribute suffixed onto a leading `:global()` stays global (unscoped).
#[test]
fn global_suffix_stays_global() {
    let css = scoped_css(
        "<a class=\"cls\"></a><style>:global(a).cls { color: red; } :global(a)[role=\"button\"] { color: blue; }</style>",
        ScopedStyleStrategy::Where,
    );
    assert_css_snapshot!("global_suffix_stays_global", css);
}

/// A `:global()` list whose items carry leading combinators keeps each item's
/// combinator and distributes the scope prefix.
#[test]
fn global_leading_combinator_list() {
    let css = scoped_css(
        "<div class=\"panel\"></div><style>.panel :global(> .a, > .b) { color: green; }</style>",
        ScopedStyleStrategy::Where,
    );
    assert_css_snapshot!("global_leading_combinator_list", css);
}

/// Two list `:global()` in one selector expand as a cartesian product.
#[test]
fn global_selector_list_cartesian() {
    let css = scoped_css(
        "<style>:global(a, b) :global(c, d) { color: red; }</style>",
        ScopedStyleStrategy::Where,
    );
    assert_css_snapshot!("global_selector_list_cartesian", css);
}

/// The same list distribution under the attribute scoping strategy.
#[test]
fn global_selector_list_attribute_strategy() {
    let css = scoped_css(
        "<div class=\"wrapper\"><ul></ul></div><style>.wrapper :global(ul, ol) { margin: 0; }</style>",
        ScopedStyleStrategy::Attribute,
    );
    assert_css_snapshot!("global_selector_list_attribute_strategy", css);
}
