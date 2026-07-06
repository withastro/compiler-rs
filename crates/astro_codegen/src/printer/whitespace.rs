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
//! Applies the React/JSX whitespace rules (Babel's `cleanJSXElementLiteralChild`):
//! whitespace is trimmed only where it touches a line break. Whitespace within a
//! single line is significant and preserved (leading or trailing space on a
//! one-line text node, or a lone space between two elements). Whitespace-only
//! lines are dropped, and the surviving lines are joined with a single space.

use oxc_ast::ast::*;

use crate::scanner::is_equal_jsx_attribute_name;

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
        if let JSXAttributeItem::Attribute(attr) = attr
            && is_equal_jsx_attribute_name(&attr.name, "is:raw")
        {
            return true;
        }
    }
    false
}

/// Position of a text node relative to its siblings, for Html mode trimming.
#[derive(Clone, Copy)]
pub(super) struct TextPosition {
    /// Whether this text node is the only child (no siblings).
    pub is_lone_child: bool,
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
/// Mirrors React/JSX text handling (Babel's `cleanJSXElementLiteralChild`):
/// whitespace is only trimmed where it borders a line break. Within a single
/// line, leading, trailing, and interior whitespace is significant and kept, so
/// `Page ` and `: ` and a lone space between elements all survive. Tabs collapse
/// to a single space, whitespace-only lines are dropped, and the surviving lines
/// are joined with a single space. Returns `None` when nothing remains.
pub(super) fn collapse_jsx(text: &str, in_raw_element: bool) -> Option<String> {
    if in_raw_element {
        return Some(text.to_string());
    }

    let lines = split_jsx_lines(text);
    let line_count = lines.len();

    // Index of the last line carrying a non-space/tab character; defaults to 0
    // (matching Babel) so a single whitespace-only line is preserved verbatim.
    let last_non_empty = lines
        .iter()
        .rposition(|line| line.bytes().any(|b| b != b' ' && b != b'\t'))
        .unwrap_or(0);

    let mut result = String::with_capacity(text.len());
    for (i, line) in lines.iter().enumerate() {
        let mut trimmed = line.replace('\t', " ");
        if i != 0 {
            trimmed = trimmed.trim_start_matches(' ').to_string();
        }
        if i + 1 != line_count {
            trimmed = trimmed.trim_end_matches(' ').to_string();
        }

        if trimmed.is_empty() {
            continue;
        }

        result.push_str(&trimmed);
        if i != last_non_empty {
            result.push(' ');
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Split text on JSX line boundaries (`\r\n`, `\n`, or `\r`), keeping empty
/// segments (including a trailing one after a final line break) so the line
/// indices line up with React's `String.prototype.split(/\r\n|\n|\r/)`.
fn split_jsx_lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(&text[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                lines.push(&text[start..i]);
                i += 1;
                if i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
                start = i;
            }
            _ => i += 1,
        }
    }
    lines.push(&text[start..]);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(lone: bool) -> TextPosition {
        TextPosition {
            is_lone_child: lone,
        }
    }

    fn html(text: &str) -> Option<String> {
        collapse_html(text, false, false, pos(false))
    }

    fn html_lone(text: &str) -> Option<String> {
        collapse_html(text, false, false, pos(true))
    }

    fn html_insensitive(text: &str) -> Option<String> {
        collapse_html(text, false, true, pos(false))
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
        let p = pos(false);
        assert_eq!(
            collapse_html("  hello  ", true, false, p),
            Some("  hello  ".to_string())
        );
    }

    // --- Jsx mode ---

    fn jsx(text: &str) -> Option<String> {
        collapse_jsx(text, false)
    }

    // Same-line whitespace is significant and preserved (the bug these rules fix).

    #[test]
    fn jsx_preserves_trailing_space_before_expression() {
        // `<h1>Page {n}</h1>` → text node "Page "
        assert_eq!(jsx("Page "), Some("Page ".to_string()));
    }

    #[test]
    fn jsx_preserves_space_between_expressions() {
        // `<li>{id}: {title}</li>` → text node ": "
        assert_eq!(jsx(": "), Some(": ".to_string()));
    }

    #[test]
    fn jsx_preserves_leading_space() {
        // `<h1 slot="named"> / Named</h1>` → text node " / Named"
        assert_eq!(jsx(" / Named"), Some(" / Named".to_string()));
    }

    #[test]
    fn jsx_preserves_lone_inter_element_space() {
        // `<span>hello</span> <em>world</em>` → text node " " between elements
        assert_eq!(jsx(" "), Some(" ".to_string()));
        assert_eq!(jsx("   "), Some("   ".to_string()));
    }

    #[test]
    fn jsx_preserves_interior_and_edge_spaces_on_single_line() {
        assert_eq!(jsx("  hello world  "), Some("  hello world  ".to_string()));
        assert_eq!(jsx("hello   world"), Some("hello   world".to_string()));
    }

    // Newline-adjacent whitespace is trimmed and collapsed (matches React).

    #[test]
    fn jsx_newline_only_text_removed() {
        assert_eq!(jsx("\n"), None);
        assert_eq!(jsx("\n\t  \n"), None);
        assert_eq!(jsx("  \n  "), None);
    }

    #[test]
    fn jsx_multiline_indented_text_collapses_to_single_spaces() {
        assert_eq!(jsx("\n  Hello\n  world\n"), Some("Hello world".to_string()));
    }

    #[test]
    fn jsx_trims_whitespace_touching_newline_but_keeps_inline_edge() {
        // Leading space sits on the first line (kept); trailing newline is trimmed.
        assert_eq!(jsx(" Hello\n"), Some(" Hello".to_string()));
        // Leading newline is trimmed; trailing space sits on the last line (kept).
        assert_eq!(jsx("\nHello "), Some("Hello ".to_string()));
    }

    #[test]
    fn jsx_tabs_collapse_to_single_space() {
        assert_eq!(jsx("a\tb"), Some("a b".to_string()));
    }

    #[test]
    fn jsx_raw_element_verbatim() {
        assert_eq!(
            collapse_jsx("  hello  ", true),
            Some("  hello  ".to_string())
        );
        assert_eq!(
            collapse_jsx("\n  keep me\n", true),
            Some("\n  keep me\n".to_string())
        );
    }

    /// Differential parity check: every `(input, expected)` pair below is the
    /// verbatim output of Babel 7.29.0's `cleanJSXElementLiteralChild` run on the
    /// same input. `collapse_jsx` must reproduce it exactly.
    #[test]
    fn jsx_matches_babel_clean_jsx_element_literal_child() {
        let cases: &[(&str, Option<&str>)] = &[
            ("Page ", Some("Page ")),
            (": ", Some(": ")),
            (" ", Some(" ")),
            ("   ", Some("   ")),
            (" / Named", Some(" / Named")),
            ("Form result: ", Some("Form result: ")),
            ("  hello world  ", Some("  hello world  ")),
            ("hello   world", Some("hello   world")),
            ("\n", None),
            ("\n\t  \n", None),
            ("  \n  ", None),
            ("\n  Hello\n  world\n", Some("Hello world")),
            (" Hello\n", Some(" Hello")),
            ("\nHello ", Some("Hello ")),
            ("a\tb", Some("a b")),
            ("x\t\ty", Some("x  y")),
            ("a \n b", Some("a b")),
            ("a\r\nb", Some("a b")),
            ("a\rb", Some("a b")),
            ("\r\r", None),
            ("  \n\thello\n  ", Some("hello")),
            ("line1\n\n\nline2", Some("line1 line2")),
            ("\t\thello\t\t", Some("  hello  ")),
            ("end with spaces   \n", Some("end with spaces")),
            ("中文 test", Some("中文 test")),
            ("", None),
            ("a\u{000b}b", Some("a\u{000b}b")),
            ("one\ntwo\nthree", Some("one two three")),
            ("  \n  x  \n  ", Some("x")),
        ];

        for (input, expected) in cases {
            assert_eq!(
                collapse_jsx(input, false).as_deref(),
                *expected,
                "collapse_jsx({input:?}) diverged from Babel"
            );
        }
    }
}
