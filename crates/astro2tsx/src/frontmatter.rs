//! Top-level frontmatter returns become throws to preserve terminal control flow in TSX.

use biome_js_syntax::{AnyJsRoot, JsReturnStatement, JsSyntaxKind};
use biome_rowan::{AstNode, WalkEvent};

pub(crate) struct RewrittenFrontmatter {
    pub(crate) text: String,
    pub(crate) replaced: Vec<Replacement>,
}

pub(crate) struct Replacement {
    pub(crate) text_offset: u32,
    pub(crate) text_len: u32,
    pub(crate) source_len: u32,
}

const RETURN_LEN: usize = "return".len();

/// `root` must be the parse of `source`.
pub(crate) fn rewrite_top_level_returns(source: &str, root: &AnyJsRoot) -> RewrittenFrontmatter {
    let returns = find_top_level_returns(root);
    let mut text = String::with_capacity(source.len());
    let mut replaced = Vec::with_capacity(returns.len());
    let mut cursor = 0usize;
    for (offset, has_argument) in returns {
        let offset = offset as usize;
        text.push_str(&source[cursor..offset]);
        let replacement = if has_argument {
            "throw "
        } else {
            "throw undefined"
        };
        replaced.push(Replacement {
            text_offset: text.len() as u32,
            text_len: replacement.len() as u32,
            source_len: RETURN_LEN as u32,
        });
        text.push_str(replacement);
        cursor = offset + RETURN_LEN;
    }
    text.push_str(&source[cursor..]);
    RewrittenFrontmatter { text, replaced }
}

fn find_top_level_returns(root: &AnyJsRoot) -> Vec<(u32, bool)> {
    let mut returns = Vec::new();
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
                    returns.push((
                        u32::from(token.text_trimmed_range().start()),
                        stmt.argument().is_some(),
                    ));
                }
            }
            WalkEvent::Leave(node) => {
                if is_function_like(node.kind()) {
                    function_depth = function_depth.saturating_sub(1);
                }
            }
        }
    }
    returns
}

fn is_function_like(kind: JsSyntaxKind) -> bool {
    matches!(
        kind,
        JsSyntaxKind::JS_FUNCTION_DECLARATION
            | JsSyntaxKind::JS_FUNCTION_EXPORT_DEFAULT_DECLARATION
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
