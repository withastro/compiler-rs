//! Whitespace collapsing for compact mode.
//!
//! Implements the whitespace collapsing algorithms for `CompactMode::Html` and
//! `CompactMode::Jsx`.
//!
//! ## `CompactMode::Html`
//!
//! Follows the same rules as the Go Astro compiler's `compact: true` option,
//! which mirrors browser HTML whitespace rules:
//!
//! 1. Raw elements (`<pre>`, `<textarea>`, `<script>`, `<style>`, `is:raw`, …)
//!    are never touched.
//! 2. In expression containers `{…}`:
//!    - Leading whitespace is stripped from the **first** child text node.
//!    - Trailing whitespace is stripped from the **last** child text node.
//! 3. Whitespace-only text nodes:
//!    - If it is the only child, or its parent is a whitespace-insensitive
//!      element (`<head>`), collapse to an empty string.
//!    - Otherwise collapse to a single `" "` (preserves inter-element spacing).
//! 4. Leading whitespace on a text node that also has non-whitespace content:
//!    collapsed to `" "` (or `"\n"` if the original whitespace contained a newline).
//! 5. Trailing whitespace (same logic as leading).
//!
//! ## `CompactMode::Jsx`
//!
//! Strips all whitespace-only text nodes and all leading/trailing whitespace
//! from text content. Interior whitespace runs are collapsed to a single space.

use oxc_ast::ast::*;

use crate::scanner::get_jsx_attribute_name;

/// Returns true for elements whose text content should **never** be touched
/// by whitespace collapsing.
pub(super) fn is_raw_element_name(name: &str) -> bool {
    matches!(
        name,
        "pre"
            | "listing"
            | "iframe"
            | "noembed"
            | "noframes"
            | "math"
            | "plaintext"
            | "script"
            | "style"
            | "textarea"
            | "title"
            | "xmp"
    )
}

/// Returns true if the element opening tag carries `is:raw`.
pub(super) fn has_is_raw_attr(attrs: &[JSXAttributeItem<'_>]) -> bool {
    for attr in attrs {
        if let JSXAttributeItem::Attribute(attr) = attr {
            let name = get_jsx_attribute_name(&attr.name);
            if name == "is:raw" {
                return true;
            }
        }
    }
    false
}

/// Position of a text node relative to its siblings, for Html mode trimming.
#[derive(Clone, Copy)]
pub(super) struct TextPosition {
    /// Whether this text node is the only child (no siblings).
    pub is_lone_child: bool,
    /// Whether this text node is the first child of an expression container.
    pub is_first_in_expression: bool,
    /// Whether this text node is the last child of an expression container.
    pub is_last_in_expression: bool,
}

/// Apply `CompactMode::Html` whitespace collapsing to a text value.
///
/// `in_raw_element` – if true, return the text verbatim (inside `<pre>` etc.).
/// `in_whitespace_insensitive` – if true (e.g. `<head>`), whitespace-only
///   text is removed entirely rather than collapsed to a single space.
/// `pos` – contextual information about sibling position.
///
/// Returns `None` if the text should be omitted entirely (empty after
/// collapsing).  Returns `Some(s)` with the collapsed string otherwise.
pub(super) fn collapse_html(
    text: &str,
    in_raw_element: bool,
    in_whitespace_insensitive: bool,
    pos: TextPosition,
) -> Option<String> {
    if in_raw_element {
        return Some(text.to_string());
    }

    // Expression boundary trimming: fully strip leading whitespace from the
    // first child of an expression container, and trailing whitespace from the
    // last child. After trimming, if the result is empty we emit nothing.
    if pos.is_first_in_expression || pos.is_last_in_expression {
        let mut s = text.to_string();
        if pos.is_first_in_expression {
            // Trim leading whitespace
            s = s.trim_start().to_string();
        }
        if pos.is_last_in_expression {
            // Trim trailing whitespace
            s = s.trim_end().to_string();
        }
        if s.is_empty() {
            return None;
        }
        return Some(s);
    }

    // Pure whitespace text node?
    if text.chars().all(|c| c.is_ascii_whitespace()) {
        if pos.is_lone_child || in_whitespace_insensitive {
            return None; // Remove entirely
        }
        return Some(" ".to_string()); // Preserve as single space
    }

    // Text with real content — collapse leading and trailing whitespace.
    let mut result = text.to_string();

    // Find leading whitespace extent
    let leading_len = result.len() - result.trim_start().len();
    if leading_len > 0 {
        let leading = &result[..leading_len];
        let replacement = if leading.contains('\n') { "\n" } else { " " };
        result = format!("{}{}", replacement, &result[leading_len..]);
    }

    // Find trailing whitespace extent
    let trimmed_end = result.trim_end();
    let trailing_len = result.len() - trimmed_end.len();
    if trailing_len > 0 {
        let trailing = &result[result.len() - trailing_len..];
        let replacement = if trailing.contains('\n') { "\n" } else { " " };
        result = format!("{}{}", trimmed_end, replacement);
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Apply `CompactMode::Jsx` whitespace collapsing to a text value.
///
/// - Whitespace-only text → omitted (`None`).
/// - Text with real content → leading and trailing whitespace stripped
///   entirely; interior whitespace runs collapsed to a single space.
pub(super) fn collapse_jsx(text: &str, in_raw_element: bool) -> Option<String> {
    if in_raw_element {
        return Some(text.to_string());
    }

    if text.chars().all(|c| c.is_ascii_whitespace()) {
        return None;
    }

    // Collapse all interior whitespace runs to a single space, then trim ends.
    let collapsed: String = text.split_ascii_whitespace().collect::<Vec<_>>().join(" ");

    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(lone: bool, first_expr: bool, last_expr: bool) -> TextPosition {
        TextPosition {
            is_lone_child: lone,
            is_first_in_expression: first_expr,
            is_last_in_expression: last_expr,
        }
    }

    fn html(text: &str) -> Option<String> {
        collapse_html(text, false, false, pos(false, false, false))
    }

    fn html_lone(text: &str) -> Option<String> {
        collapse_html(text, false, false, pos(true, false, false))
    }

    fn html_insensitive(text: &str) -> Option<String> {
        collapse_html(text, false, true, pos(false, false, false))
    }

    // --- Html mode: whitespace-only text ---

    #[test]
    fn html_ws_only_with_siblings_becomes_single_space() {
        assert_eq!(html("   "), Some(" ".to_string()));
        assert_eq!(html("\t\n\t"), Some(" ".to_string()));
    }

    #[test]
    fn html_ws_only_lone_child_removed() {
        assert_eq!(html_lone("   "), None);
        assert_eq!(html_lone("\t"), None);
    }

    #[test]
    fn html_ws_only_in_insensitive_element_removed() {
        assert_eq!(html_insensitive("   "), None);
    }

    // --- Html mode: text with content ---

    #[test]
    fn html_trims_surrounding_whitespace() {
        assert_eq!(html("  hello  "), Some(" hello ".to_string()));
    }

    #[test]
    fn html_preserves_content_without_surrounding_ws() {
        assert_eq!(html("hello"), Some("hello".to_string()));
    }

    #[test]
    fn html_newline_in_leading_ws_becomes_newline() {
        assert_eq!(html("\n\n\thello"), Some("\nhello".to_string()));
    }

    #[test]
    fn html_newline_in_trailing_ws_becomes_newline() {
        assert_eq!(html("hello\n\n\t"), Some("hello\n".to_string()));
    }

    #[test]
    fn html_space_leading_becomes_single_space() {
        assert_eq!(html("   hello"), Some(" hello".to_string()));
    }

    // --- Html mode: raw elements ---

    #[test]
    fn html_raw_element_verbatim() {
        let p = pos(false, false, false);
        assert_eq!(
            collapse_html("  hello  ", true, false, p),
            Some("  hello  ".to_string())
        );
    }

    // --- Html mode: expression boundary ---

    #[test]
    fn html_expression_first_child_trims_leading() {
        let p = pos(false, true, false);
        assert_eq!(
            collapse_html("\n  foo\n", false, false, p),
            Some("foo\n".to_string())
        );
    }

    #[test]
    fn html_expression_last_child_trims_trailing() {
        let p = pos(false, false, true);
        assert_eq!(
            collapse_html("\n  foo\n  ", false, false, p),
            Some("\n  foo".to_string())
        );
    }

    #[test]
    fn html_expression_whitespace_only_first_removed() {
        let p = pos(false, true, false);
        assert_eq!(collapse_html("\n  \t", false, false, p), None);
    }

    // --- Jsx mode ---

    #[test]
    fn jsx_ws_only_removed() {
        assert_eq!(collapse_jsx("   ", false), None);
        assert_eq!(collapse_jsx("\n\t  ", false), None);
    }

    #[test]
    fn jsx_content_trimmed_and_collapsed() {
        assert_eq!(
            collapse_jsx("  hello world  ", false),
            Some("hello world".to_string())
        );
        assert_eq!(
            collapse_jsx("  hello   world  ", false),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn jsx_raw_element_verbatim() {
        assert_eq!(
            collapse_jsx("  hello  ", true),
            Some("  hello  ".to_string())
        );
    }
}
