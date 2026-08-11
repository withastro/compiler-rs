//! Frontmatter post-processing: rewrites top-level `return` statements
//! into `throw ` so the generated TSX type-checks. Astro frontmatter runs
//! inside an implicit function at runtime, so top-level returns are valid
//! Astro syntax — but TS rejects them at the module level.
//!
//! The upstream Go printer keeps the byte count stable by emitting `throw `
//! (with an extra trailing space) in place of `return` (six chars). We do
//! the same so source-mapping into the modified frontmatter remains
//! aligned with the original file.

use biome_js_parser::{JsParserOptions, parse};
use biome_js_syntax::{AnyJsRoot, JsReturnStatement, JsSyntaxKind};
use biome_languages::JsFileSource;
use biome_rowan::{AstNode, WalkEvent};

/// Returns the byte offsets (relative to `source`) of every `return`
/// keyword that sits at module scope. Offsets address the start of the
/// keyword, ready for substitution.
pub(crate) fn find_top_level_returns(source: &str) -> Vec<u32> {
    // Use the Astro embedding kind so top-level `return` keywords are
    // accepted as valid syntax (Astro frontmatter runs inside a generated
    // function at runtime).
    let parse = parse(source, JsFileSource::astro(), JsParserOptions::default());
    let root: AnyJsRoot = parse.tree();
    let syntax = root.syntax().clone();

    let mut offsets = Vec::new();
    let mut function_depth: u32 = 0;

    // Walk pre-order, incrementing on entering a function-like body and
    // decrementing on leaving it. Top-level returns are nodes encountered
    // at function_depth == 0.
    for event in syntax.preorder() {
        match event {
            WalkEvent::Enter(node) => {
                if is_function_like(node.kind()) {
                    function_depth += 1;
                } else if function_depth == 0
                    && node.kind() == JsSyntaxKind::JS_RETURN_STATEMENT
                    && let Some(stmt) = JsReturnStatement::cast(node)
                    && let Ok(token) = stmt.return_token()
                {
                    offsets.push(u32::from(token.text_trimmed_range().start()));
                }
            }
            WalkEvent::Leave(node) => {
                if is_function_like(node.kind()) {
                    function_depth = function_depth.saturating_sub(1);
                }
            }
        }
    }
    offsets
}

fn is_function_like(kind: JsSyntaxKind) -> bool {
    matches!(
        kind,
        JsSyntaxKind::JS_FUNCTION_DECLARATION
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

/// Rewrites the frontmatter source: at every offset returned by
/// `find_top_level_returns`, replaces the six-byte keyword `return` with
/// `throw ` (padded with a trailing space). Leaves the rest untouched.
pub(crate) fn rewrite_top_level_returns(source: &str) -> String {
    let offsets = find_top_level_returns(source);
    if offsets.is_empty() {
        return source.to_string();
    }
    let bytes = source.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut cursor = 0usize;
    for offset in offsets {
        let offset = offset as usize;
        if offset < cursor || offset + b"return".len() > bytes.len() {
            continue;
        }
        if &bytes[offset..offset + b"return".len()] != b"return" {
            continue;
        }
        out.extend_from_slice(&bytes[cursor..offset]);
        out.extend_from_slice(b"throw ");
        cursor = offset + b"return".len();
    }
    out.extend_from_slice(&bytes[cursor..]);
    String::from_utf8(out).expect("ASCII substitution must remain valid UTF-8")
}
