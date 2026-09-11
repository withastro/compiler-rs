use biome_rowan::{Language, SyntaxNode};

/// Drops the tree with bounded destructor recursion. Requires exclusive ownership
/// of its nodes; other references can defer destruction until after this returns.
pub(crate) fn drop_syntax_tree<L: Language>(root: SyntaxNode<L>) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        stack.extend(node.children().map(SyntaxNode::detach));
        drop(node);
    }
}
