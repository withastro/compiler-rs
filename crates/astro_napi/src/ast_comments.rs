//! One flat root-level comment list, because that is the shape Prettier attaches from.

use oxc_ast::{
    Comment, CommentKind,
    ast::{AstroRoot, Program},
};
use oxc_ast_visit::{Visit, walk};
use oxc_estree::{ESTree, JsonSafeString, Serializer, StructSerializer};
use oxc_span::Span;

pub struct SerializedComment<'s> {
    kind: CommentKind,
    value: &'s str,
    span: Span,
}

impl SerializedComment<'_> {
    pub fn span_mut(&mut self) -> &mut Span {
        &mut self.span
    }
}

impl ESTree for SerializedComment<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) {
        let mut state = serializer.serialize_struct();
        state.serialize_field(
            "type",
            &JsonSafeString(match self.kind {
                CommentKind::Line => "Line",
                CommentKind::SingleLineBlock | CommentKind::MultiLineBlock => "Block",
            }),
        );
        state.serialize_field("value", &self.value);
        state.serialize_span(self.span);
        state.end();
    }
}

/// `AstroRoot` plus the flat comment list, which has no home on the AST itself.
pub struct AstroRootWithComments<'a, 'b> {
    pub root: &'b AstroRoot<'a>,
    pub comments: &'b [SerializedComment<'b>],
}

impl ESTree for AstroRootWithComments<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) {
        let mut state = serializer.serialize_struct();
        state.serialize_field("type", &JsonSafeString("AstroRoot"));
        state.serialize_field("frontmatter", &self.root.frontmatter);
        state.serialize_field("body", &self.root.body);
        state.serialize_field("comments", &self.comments);
        state.serialize_span(self.root.span);
        state.end();
    }
}

struct ProgramCommentCollector<'s> {
    source_text: &'s str,
    comments: Vec<SerializedComment<'s>>,
}

impl<'a, 's> Visit<'a> for ProgramCommentCollector<'s> {
    fn visit_program(&mut self, program: &Program<'a>) {
        for comment in &program.comments {
            self.comments.push(to_serialized(comment, self.source_text));
        }
        walk::walk_program(self, program);
    }
}

fn to_serialized<'s>(comment: &Comment, source_text: &'s str) -> SerializedComment<'s> {
    let content = comment.content_span();
    SerializedComment {
        kind: comment.kind,
        value: &source_text[content.start as usize..content.end as usize],
        span: comment.span,
    }
}

/// Template comments never reach the AST, so they arrive separately as `body_comments`.
pub fn collect<'s>(
    root: &AstroRoot<'_>,
    body_comments: &[Comment],
    source_text: &'s str,
) -> Vec<SerializedComment<'s>> {
    let mut collector = ProgramCommentCollector { source_text, comments: Vec::new() };
    collector.visit_astro_root(root);

    let mut comments = collector.comments;
    comments.extend(body_comments.iter().map(|comment| to_serialized(comment, source_text)));
    comments.sort_unstable_by_key(|comment| (comment.span.start, comment.span.end));
    // `<script>` contents are lexed twice — once as body trivia, once by the script re-parse.
    comments.dedup_by_key(|comment| comment.span);
    comments
}
