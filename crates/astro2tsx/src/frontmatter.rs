//! Frontmatter runs in an implicit function, so its top-level `return`s are
//! valid Astro but invalid TS; they become `throw `, whose six bytes match
//! `return` and keep source mappings aligned.

use biome_js_syntax::{AnyJsRoot, JsReturnStatement, JsSyntaxKind};
use biome_rowan::{AstNode, WalkEvent};

/// `root` must be the parse of `source`.
pub(crate) fn rewrite_top_level_returns(source: &str, root: &AnyJsRoot) -> String {
    let offsets = find_top_level_returns(root);
    if offsets.is_empty() {
        return source.to_string();
    }
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for offset in offsets {
        let offset = offset as usize;
        out.push_str(&source[cursor..offset]);
        out.push_str("throw ");
        cursor = offset + "return".len();
    }
    out.push_str(&source[cursor..]);
    out
}

/// Byte offsets of every `return` keyword sitting at module scope, ascending.
fn find_top_level_returns(root: &AnyJsRoot) -> Vec<u32> {
    let mut offsets = Vec::new();
    let mut function_depth: u32 = 0;

    for event in root.syntax().preorder() {
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
