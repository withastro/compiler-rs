//! Style extraction helpers.
//!
//! Contains the [`StyleBlock`] type and the [`extract_styles`] public entry
//! point used by the "Rust extract → TS preprocess → Rust compile" pipeline,
//! along with the private free-function helpers it delegates to.

use oxc_ast::ast::*;

use crate::css_scoping;
use crate::scanner::{get_jsx_attribute_name, get_jsx_element_name, is_equal_jsx_attribute_name};

use super::{AstroCodegen, expr_to_string};

/// Metadata about an extractable `<style>` block in an Astro component.
///
/// Returned by [`extract_styles`] so that callers (e.g. the TS wrapper) can
/// preprocess style content before compilation.
#[derive(Debug, Clone)]
pub struct StyleBlock {
    /// Zero-based index of this style block among all extractable styles.
    pub index: usize,
    /// The CSS/preprocessor text content between `<style>` and `</style>`.
    pub content: String,
    /// The element's attributes as key-value pairs.
    /// Only quoted and empty (boolean) attributes are included — expression
    /// attributes (like `define:vars={...}`) are omitted, matching the Go
    /// compiler's `GetAttrs` behavior.
    pub attrs: Vec<(String, String)>,
}

/// Extract style block metadata from an Astro AST without performing compilation.
///
/// This walks the template to find all extractable `<style>` elements (i.e.
/// those that are **not** `is:inline`, `set:html`, `set:text`, or inside
/// non-hoistable contexts like `<svg>`/`<noscript>`/`<template>`).
///
/// For each extractable style, it returns a [`StyleBlock`] with the text
/// content and attributes. The blocks are returned in document order and
/// their `index` fields are sequential (0, 1, 2, …).
///
/// This is the first step in the "Rust extract → TS preprocess → Rust compile"
/// pipeline for `preprocessStyle` support.
pub fn extract_styles<'a>(root: &'a AstroRoot<'a>) -> Vec<StyleBlock> {
    let mut blocks = Vec::new();
    extract_styles_from_children(&root.body, false, &mut blocks);
    blocks
}

fn extract_styles_from_children<'a>(
    children: &[JSXChild<'a>],
    in_non_hoistable: bool,
    blocks: &mut Vec<StyleBlock>,
) {
    for child in children {
        match child {
            JSXChild::Element(el) => {
                let name = get_jsx_element_name(&el.opening_element.name);
                if name == "style" && !in_non_hoistable && should_extract_style_element(el) {
                    // Extract content and attrs
                    let content = extract_style_text(el);
                    let attrs = extract_style_attrs(el);
                    blocks.push(StyleBlock {
                        index: blocks.len(),
                        content,
                        attrs,
                    });
                } else {
                    let non_hoistable = in_non_hoistable
                        || matches!(name.as_ref(), "svg" | "noscript" | "template");
                    extract_styles_from_children(&el.children, non_hoistable, blocks);
                }
            }
            JSXChild::Fragment(frag) => {
                extract_styles_from_children(&frag.children, in_non_hoistable, blocks);
            }
            _ => {}
        }
    }
}

/// Check if a `<style>` element should be extracted (same logic as
/// `AstroCodegen::should_extract_style` but as a free function).
pub(super) fn should_extract_style_element(el: &JSXElement<'_>) -> bool {
    let attrs = &el.opening_element.attributes;
    for attr in attrs {
        if let JSXAttributeItem::Attribute(attr) = attr {
            let name = get_jsx_attribute_name(&attr.name);
            if name == "is:inline" || name == "set:html" || name == "set:text" {
                return false;
            }
        }
    }
    true
}

/// Extract the text content of a `<style>` element.
pub(super) fn extract_style_text(el: &JSXElement<'_>) -> String {
    let mut text = String::new();
    for child in &el.children {
        if let JSXChild::Text(t) = child {
            text.push_str(t.value.as_str());
        }
    }
    text
}

/// Extract quoted and empty (boolean) attributes from a `<style>` element.
/// Expression attributes (like `define:vars={...}`) are omitted, matching
/// the Go compiler's `GetAttrs` behavior.
pub(super) fn extract_style_attrs(el: &JSXElement<'_>) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    for attr_item in &el.opening_element.attributes {
        if let JSXAttributeItem::Attribute(attr) = attr_item {
            let name = get_jsx_attribute_name(&attr.name);
            match &attr.value {
                None => {
                    // Boolean/empty attribute (e.g. `is:global`)
                    attrs.push((name.into_owned(), String::new()));
                }
                Some(JSXAttributeValue::StringLiteral(lit)) => {
                    // Quoted attribute (e.g. `lang="scss"`)
                    attrs.push((name.into_owned(), lit.value.as_str().to_string()));
                }
                _ => {
                    // Expression attributes are skipped
                }
            }
        }
    }
    attrs
}

impl<'a> AstroCodegen<'a> {
    /// Pre-scan the template body to extract `<style>` elements.
    /// This populates `self.extracted_css` and `self.has_scoped_styles`
    /// before any printing happens.
    pub(super) fn prescan_styles(&mut self, body: &[JSXChild<'a>]) {
        for child in body {
            self.prescan_styles_child(child, false);
        }
    }

    pub(super) fn prescan_styles_child(&mut self, child: &JSXChild<'a>, in_non_hoistable: bool) {
        match child {
            JSXChild::Element(el) => {
                let name = get_jsx_element_name(&el.opening_element.name);
                if name == "style" && !in_non_hoistable && self.should_extract_style(el) {
                    self.extract_style_element(el);
                } else {
                    // Mark descendant context as non-hoistable for svg/noscript/template
                    let non_hoistable = in_non_hoistable
                        || matches!(name.as_ref(), "svg" | "noscript" | "template");
                    for c in &el.children {
                        self.prescan_styles_child(c, non_hoistable);
                    }
                }
            }
            JSXChild::Fragment(frag) => {
                for c in &frag.children {
                    self.prescan_styles_child(c, in_non_hoistable);
                }
            }
            _ => {}
        }
    }

    /// Print CSS import statements, one per extracted `<style>` tag.
    /// Format: `import "filename?astro&type=style&index=N&lang.css";`
    pub(super) fn print_css_imports(&mut self) {
        if self.extracted_css.is_empty() {
            return;
        }
        let filename = self
            .options
            .filename
            .clone()
            .unwrap_or_else(|| "<stdin>".to_string());

        for i in 0..self.extracted_css.len() {
            self.println(&format!(
                "import \"{filename}?astro&type=style&index={i}&lang.css\";"
            ));
        }
    }

    /// Check if a `<style>` element should be extracted (removed from template
    /// and its CSS emitted separately).
    ///
    /// A `<style>` is extracted unless:
    /// - It has `is:inline` (render as raw HTML)
    /// - It has `set:html` or `set:text` (directive-driven content)
    /// - It is inside an `<svg>`, `<noscript>`, or other non-hoistable context
    ///   (approximated by checking element_depth — styles at depth 0 are always
    ///   hoisted; the Go compiler has a more precise `IsHoistable` check)
    pub(super) fn should_extract_style(&self, el: &JSXElement<'a>) -> bool {
        let attrs = &el.opening_element.attributes;

        // Don't extract if has is:inline
        if Self::has_is_inline_attribute(attrs) {
            return false;
        }

        // Don't extract if has set:html or set:text
        if Self::extract_set_directive(attrs).is_some() {
            return false;
        }

        true
    }

    /// Extract a `<style>` element: collect its CSS content, apply scoping,
    /// and skip the element from the template output.
    pub(super) fn extract_style_element(&mut self, el: &JSXElement<'a>) {
        // Capture the current extraction index and bump it for the next style
        let current_index = self.style_extraction_index;
        self.style_extraction_index += 1;

        // Check for is:global attribute
        let is_global = el.opening_element.attributes.iter().any(|attr| {
            if let JSXAttributeItem::Attribute(attr) = attr {
                is_equal_jsx_attribute_name(&attr.name, "is:global")
            } else {
                false
            }
        });

        // Check for define:vars attribute and collect its value
        let has_define_vars = self.collect_define_vars(el);

        // Get CSS content: use preprocessed content if available, otherwise read from AST.
        let css_content = if let Some(preprocessed) = self
            .options
            .preprocessed_styles
            .as_ref()
            .and_then(|styles| styles.get(current_index))
            .and_then(|entry| entry.as_ref())
        {
            preprocessed.clone()
        } else {
            extract_style_text(el)
        };
        let css_trimmed = css_content.trim();

        if css_trimmed.is_empty() {
            // Empty style — still counts for scoping purposes if not global
            // or if it has define:vars
            if !is_global || has_define_vars {
                self.has_scoped_styles = true;
            }
            return;
        }

        // Apply CSS scoping (unless is:global)
        let scoped_css = if is_global {
            css_trimmed.to_string()
        } else {
            self.has_scoped_styles = true;
            css_scoping::scope_css(
                css_trimmed,
                &self.source_hash,
                self.options.scoped_style_strategy(),
            )
        };

        self.extracted_css.push(scoped_css);
    }

    /// Check for `define:vars` attribute on a `<style>` element and collect its value.
    /// Returns `true` if `define:vars` was found.
    pub(super) fn collect_define_vars(&mut self, el: &JSXElement<'a>) -> bool {
        for attr in &el.opening_element.attributes {
            if let JSXAttributeItem::Attribute(attr) = attr
                && is_equal_jsx_attribute_name(&attr.name, "define:vars")
            {
                match &attr.value {
                    Some(JSXAttributeValue::StringLiteral(lit)) => {
                        self.define_vars_values
                            .push(format!("'{}'", lit.value.as_str()));
                    }
                    Some(JSXAttributeValue::ExpressionContainer(expr)) => {
                        if let Some(e) = expr.expression.as_expression() {
                            self.define_vars_values.push(expr_to_string(e));
                        }
                    }
                    _ => {}
                }
                return true;
            }
        }
        false
    }
}
