//! Render scope that the frontmatter body and template are checked in.
//!
//! Mirroring Astro's runtime output, imports and exports (plus erased type declarations)
//! stay at module scope while the other statements and the template move into an async
//! render function. That function declares the type parameters of a generic `Props<T>`,
//! which only stays generic inside a scope that declares `T`.

use std::collections::HashMap;

use biome_js_syntax::{
    AnyJsRoot, JsClassDeclaration, JsExport, JsFunctionDeclaration, JsLanguage,
    JsReferenceIdentifier, JsSyntaxKind, JsVariableStatement, TsEnumDeclaration,
    unescape_js_string,
};
use biome_rowan::{AstNode, AstSeparatedList, SyntaxKind, SyntaxNode, TextRange, TextSize};

use crate::props::{binding_names, module_items};

type JsNode = SyntaxNode<JsLanguage>;

/// Line-aligned chunks of the frontmatter that tile it from `lead` to its end.
#[derive(Debug, Default)]
pub(crate) struct ScopePlan {
    /// Lines before the first item, emitted first so top-of-file insertions land under the fence.
    pub lead: TextRange,
    /// Chunks emitted before the render function, in source order.
    pub hoisted: Vec<TextRange>,
    /// Chunks emitted inside the render function, in source order.
    pub body: Vec<TextRange>,
}

/// Whether a module item must stay at module scope.
#[derive(Clone, Copy, PartialEq)]
enum Placement {
    /// Imports and erased declarations: hoisted, and may reference render-scope values in types.
    Declaration,
    /// Runtime exports: hoisted by Astro too, so only their types may see render-scope values.
    Export,
    Body,
}

/// Plans the render scope, or `None` to keep the module layout.
///
/// Unfinished imports, exports, and declarations keep it, since TypeScript recovers them from
/// the code that follows; errors elsewhere keep the render scope so diagnostics stay stable.
pub(crate) fn plan(root: &AnyJsRoot, errors: &[TextRange]) -> Option<ScopePlan> {
    let items = module_items(root);
    let mut placements: Vec<Placement> = items.iter().map(placement).collect();
    if has_unfinished_declaration(&items, &placements, errors) {
        return None;
    }

    // Body declarations that hoisted code can name, by namespace.
    let mut values: HashMap<String, usize> = HashMap::new();
    let mut types: HashMap<String, usize> = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        if placements[index] == Placement::Body {
            let (bound_values, bound_types) = declared_names(item);
            for name in bound_values {
                values.entry(name).or_insert(index);
            }
            for name in bound_types {
                types.entry(name).or_insert(index);
            }
        }
    }

    // Types are erased, so hoisted types may query render-scope values (`typeof defaults`).
    // Hoist those declarations and their dependencies instead of leaving the names unbound.
    let mut pending: Vec<usize> = (0..items.len())
        .filter(|&index| placements[index] != Placement::Body)
        .collect();
    while let Some(index) = pending.pop() {
        let only_erased = placements[index] == Placement::Export;
        for reference in references(&items[index]) {
            if only_erased && !reference.erased {
                // A runtime export that reads a render-scope value fails at runtime too.
                continue;
            }
            let namespace = if reference.is_type { &types } else { &values };
            if let Some(&dependency) = namespace.get(&reference.name)
                && placements[dependency] == Placement::Body
            {
                placements[dependency] = Placement::Declaration;
                pending.push(dependency);
            }
        }
    }

    // Overload signatures stay with the implementation that follows them.
    for index in (0..items.len().saturating_sub(1)).rev() {
        if items[index].kind() == JsSyntaxKind::TS_DECLARE_FUNCTION_DECLARATION {
            placements[index] = placements[index + 1];
        }
    }

    let root_range = root.syntax().text_range_with_trivia();
    let starts: Vec<TextSize> = items.iter().map(chunk_start).collect();
    let mut plan = ScopePlan {
        lead: TextRange::new(
            root_range.start(),
            starts.first().copied().unwrap_or(root_range.end()),
        ),
        ..ScopePlan::default()
    };
    for (index, placement) in placements.into_iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(root_range.end());
        let chunk = TextRange::new(starts[index], end);
        if placement == Placement::Body {
            plan.body.push(chunk);
        } else {
            plan.hoisted.push(chunk);
        }
    }
    Some(plan)
}

fn has_unfinished_declaration(
    items: &[JsNode],
    placements: &[Placement],
    errors: &[TextRange],
) -> bool {
    errors.iter().any(|error| {
        let Some(index) = items
            .iter()
            .rposition(|item| item.text_trimmed_range().start() <= error.start())
        else {
            return false;
        };
        // An error at an item's first token usually reports the unfinished item before it.
        let previous = (items[index].text_trimmed_range().start() == error.start())
            .then(|| index.checked_sub(1))
            .flatten();
        [Some(index), previous]
            .into_iter()
            .flatten()
            .any(|index| placements[index] != Placement::Body || items[index].kind().is_bogus())
    })
}

/// Start of the line holding an item's first comment or token, so comments move with their
/// item while blank lines stay with the chunk before it.
fn chunk_start(item: &JsNode) -> TextSize {
    let mut start = item.text_range_with_trivia().start();
    if let Some(token) = item.first_token() {
        for piece in token.leading_trivia().pieces() {
            if piece.is_comments() {
                break;
            }
            if piece.is_newline() {
                start = piece.text_range().end();
            }
        }
    }
    start
}

fn placement(item: &JsNode) -> Placement {
    match item.kind() {
        JsSyntaxKind::JS_EXPORT => {
            if JsExport::cast_ref(item).is_some_and(|export| is_type_only_export(&export)) {
                Placement::Declaration
            } else {
                Placement::Export
            }
        }
        JsSyntaxKind::JS_IMPORT
        | JsSyntaxKind::TS_IMPORT_EQUALS_DECLARATION
        | JsSyntaxKind::TS_INTERFACE_DECLARATION
        | JsSyntaxKind::TS_TYPE_ALIAS_DECLARATION
        | JsSyntaxKind::TS_DECLARE_STATEMENT
        | JsSyntaxKind::TS_MODULE_DECLARATION
        | JsSyntaxKind::TS_GLOBAL_DECLARATION
        | JsSyntaxKind::TS_EXTERNAL_MODULE_DECLARATION => Placement::Declaration,
        _ => Placement::Body,
    }
}

fn is_type_only_export(export: &JsExport) -> bool {
    let Ok(clause) = export.export_clause() else {
        return false;
    };
    let clause = clause.into_syntax();
    match clause.kind() {
        JsSyntaxKind::TS_EXPORT_DECLARE_CLAUSE => true,
        JsSyntaxKind::JS_EXPORT_NAMED_CLAUSE | JsSyntaxKind::JS_EXPORT_NAMED_FROM_CLAUSE => clause
            .first_token()
            .is_some_and(|token| token.kind() == JsSyntaxKind::TYPE_KW),
        _ => clause.first_child().is_some_and(|declaration| {
            matches!(
                declaration.kind(),
                JsSyntaxKind::TS_INTERFACE_DECLARATION
                    | JsSyntaxKind::TS_TYPE_ALIAS_DECLARATION
                    | JsSyntaxKind::TS_DECLARE_FUNCTION_DECLARATION
            )
        }),
    }
}

/// Names a body item binds as `(values, types)`.
fn declared_names(item: &JsNode) -> (Vec<String>, Vec<String>) {
    if let Some(statement) = JsVariableStatement::cast_ref(item) {
        let mut names = Vec::new();
        if let Ok(declaration) = statement.declaration() {
            for declarator in declaration.declarators().iter().flatten() {
                if let Ok(id) = declarator.id() {
                    names.extend(binding_names(id.syntax()));
                }
            }
        }
        (names, Vec::new())
    } else if let Some(id) = JsFunctionDeclaration::cast_ref(item).and_then(|f| f.id().ok()) {
        (binding_names(id.syntax()), Vec::new())
    } else if let Some(id) = JsClassDeclaration::cast_ref(item).and_then(|c| c.id().ok()) {
        let names = binding_names(id.syntax());
        (names.clone(), names)
    } else if let Some(id) = TsEnumDeclaration::cast_ref(item).and_then(|e| e.id().ok()) {
        let names = id
            .syntax()
            .first_token()
            .map(|token| unescape_js_string(token.token_text_trimmed()).to_string())
            .into_iter()
            .collect::<Vec<_>>();
        (names.clone(), names)
    } else {
        (Vec::new(), Vec::new())
    }
}

struct Reference {
    name: String,
    /// Resolved in the type namespace rather than the value namespace.
    is_type: bool,
    /// Inside a type, so it disappears at runtime.
    erased: bool,
}

fn references(item: &JsNode) -> Vec<Reference> {
    item.descendants()
        .filter_map(JsReferenceIdentifier::cast)
        .filter_map(|reference| {
            let token = reference.value_token().ok()?;
            // The innermost of the two decides the namespace: `Foo<typeof x>` reads value `x`.
            let is_type =
                reference
                    .syntax()
                    .ancestors()
                    .find_map(|ancestor| match ancestor.kind() {
                        JsSyntaxKind::TS_TYPEOF_TYPE => Some(false),
                        JsSyntaxKind::TS_REFERENCE_TYPE => Some(true),
                        _ => None,
                    });
            Some(Reference {
                name: unescape_js_string(token.token_text_trimmed()).to_string(),
                is_type: is_type == Some(true),
                erased: is_type.is_some(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::test_utils::convert;

    const OPENER: &str = ";({ async ";

    fn split(code: &str) -> (&str, &str) {
        let at = code
            .find(OPENER)
            .unwrap_or_else(|| panic!("no render scope:\n{code}"));
        code.split_at(at)
    }

    #[test]
    fn frontmatter_and_template_render_inside_the_scope() {
        let result = convert(
            "---\ninterface Props { value: string }\nconst { value } = Astro.props;\n---\n<p>{value}</p>",
        );
        let (module, scope) = split(&result.code);
        assert!(module.contains("interface Props {"), "{}", result.code);
        assert!(
            scope.starts_with(";({ async AstroComponent() {\n/**\n"),
            "{}",
            result.code
        );
        assert!(
            scope.contains(
                "const Astro: Readonly<import('astro').AstroGlobal<Props, typeof AstroComponent>> = null!;\nconst { value } = Astro.props;"
            ),
            "{}",
            result.code
        );
        assert!(
            scope.contains("<p>{value}</p>\n</Fragment>\n}});\nexport default function"),
            "{}",
            result.code
        );
    }

    #[test]
    fn generic_props_get_a_generic_render_scope() {
        let result = convert(
            "---\ninterface Props<T extends string, U = T> { value: T }\nconst { value } = Astro.props;\n---\n<p>{value}</p>",
        );
        let (module, scope) = split(&result.code);
        assert!(
            module.contains("interface Props<T extends string, U = T>"),
            "{}",
            result.code
        );
        assert!(
            scope.starts_with(";({ async AstroComponent<T extends string, U = T>() {"),
            "{}",
            result.code
        );
        assert!(
            scope.contains(
                "const Astro: Readonly<import('astro').AstroGlobal<Props<T, U>, typeof AstroComponent>> = null!;"
            ),
            "{}",
            result.code
        );
    }

    #[test]
    fn module_items_are_hoisted_out_of_the_render_scope() {
        let source = "---\nconst a = 1;\nimport X from './X.astro';\nexport const prerender = true;\ntype Local = string;\ninterface Props<T extends Local> { value: T }\ndeclare global { interface Window { x: 1 } }\nexport function getStaticPaths() { return []; }\nexport type { Local as Exported };\nconst b = a;\n---\n<X />";
        let result = convert(source);
        let (module, scope) = split(&result.code);
        for hoisted in [
            "import X from './X.astro';",
            "export const prerender = true;",
            "type Local = string;",
            "interface Props<T extends Local>",
            "declare global",
            "export function getStaticPaths()",
            "export type { Local as Exported };",
        ] {
            assert!(
                module.contains(hoisted),
                "{hoisted} was not hoisted:\n{}",
                result.code
            );
        }
        assert!(scope.contains("const a = 1;"), "{}", result.code);
        assert!(scope.contains("const b = a;"), "{}", result.code);
        assert!(
            scope.contains("ASTRO__Get<ASTRO__InferredGetStaticPath, 'params'>>> = null!;"),
            "{}",
            result.code
        );
    }

    #[test]
    fn values_queried_by_hoisted_types_are_hoisted_with_them() {
        let result = convert(
            "---\nconst base = { size: 1 };\nconst defaults = { ...base };\nconst { value } = Astro.props;\nenum Size { S }\nclass Model {}\ninterface Props<T> { value: T; opts: typeof defaults; size: Size; model: Model }\n---\n<p/>",
        );
        let (module, scope) = split(&result.code);
        for hoisted in [
            "const base = { size: 1 };",
            "const defaults = { ...base };",
            "enum Size { S }",
            "class Model {}",
        ] {
            assert!(
                module.contains(hoisted),
                "{hoisted} was not hoisted:\n{}",
                result.code
            );
        }
        assert!(
            scope.contains("const { value } = Astro.props;"),
            "{}",
            result.code
        );
    }

    #[test]
    fn type_and_value_namespaces_are_kept_apart() {
        // `Tag` is a type parameter in `Props` and a value in the render scope.
        let result = convert(
            "---\ntype Props<Tag extends string> = { as?: Tag };\nconst { as: Tag } = Astro.props;\nconst queried = 1;\nexport function f(x: typeof queried) {}\n---\n<Tag />",
        );
        let (module, scope) = split(&result.code);
        assert!(
            scope.contains("const { as: Tag } = Astro.props;"),
            "{}",
            result.code
        );
        // Erased reads from a runtime export still need the declaration at module scope.
        assert!(module.contains("const queried = 1;"), "{}", result.code);
    }

    #[test]
    fn runtime_exports_do_not_hoist_render_scope_values() {
        let result = convert("---\nconst a = 1;\nexport const b = a;\n---\n<p/>");
        let (_, scope) = split(&result.code);
        assert!(scope.contains("const a = 1;"), "{}", result.code);
    }

    #[test]
    fn overload_signatures_stay_with_their_implementation() {
        for (source, hoisted) in [
            (
                "---\nfunction pick(v: string): string;\nfunction pick(v: string) { return v; }\n---\n<p/>",
                false,
            ),
            (
                "---\nfunction pick(v: string): string;\nfunction pick(v: string) { return v; }\ntype Picked = ReturnType<typeof pick>;\n---\n<p/>",
                true,
            ),
        ] {
            let result = convert(source);
            let (module, scope) = split(&result.code);
            let side = if hoisted { module } else { scope };
            assert!(
                side.contains("function pick(v: string): string;\nfunction pick(v: string) {"),
                "{}",
                result.code
            );
        }
    }

    #[test]
    fn chunks_carry_leading_comments_but_not_blank_lines() {
        let result = convert(
            "---\nconst a = 1;\n\n/** Documented. */\ninterface Props { value: string }\n---\n<p/>",
        );
        let (module, scope) = split(&result.code);
        assert!(
            module.contains("\n/** Documented. */\ninterface Props { value: string }\n"),
            "{}",
            result.code
        );
        assert!(scope.contains("const a = 1;\n\n"), "{}", result.code);
    }

    #[test]
    fn frontmatter_with_parse_errors_keeps_the_render_scope() {
        let result = crate::convert_to_tsx(
            "---\ninterface Props<T> { value: T }\nconst a = Astro.props.\n---\n<p/>",
            crate::ConvertOptions::default(),
        );
        assert!(result.has_parse_errors);
        assert!(result.code.contains(OPENER), "{}", result.code);
    }

    #[test]
    fn unfinished_imports_and_exports_keep_the_module_layout() {
        for source in [
            "---\nimport { formatTh\ninterface Props<T> { as?: T }\n---\n<p/>",
            "---\nconst { as } = Astro.props;\nimport { formatTh\n---\n<p/>",
            "---\nimport { formatTh }\nconst { as } = Astro.props;\n---\n<p/>",
            "---\nimport { formatThing } from './ut\nconst { as } = Astro.props;\n---\n<p/>",
            "---\nconst { as } = Astro.props;\nexport const\n---\n<p/>",
        ] {
            let result = crate::convert_to_tsx(source, crate::ConvertOptions::default());
            assert!(result.has_parse_errors, "{source}");
            assert!(!result.code.contains(OPENER), "{}", result.code);
        }
    }

    #[test]
    fn frontmatter_rewrites_survive_reordering() {
        let source = "---\nconst v = <string>Astro.props.value\nimport X from './X.astro'\nconst id = <T>(x: T) => x\nif (!v) return Astro.redirect('/')\ninterface Props<T> { value: T }\n---\n<X />";
        let result = convert(source);
        let (module, scope) = split(&result.code);
        assert!(
            module.contains("import X from './X.astro'"),
            "{}",
            result.code
        );
        assert!(
            scope.contains("const v = (Astro.props.value as string)\n"),
            "{}",
            result.code
        );
        assert!(
            scope.contains("const id = <T,>(x: T) => x"),
            "{}",
            result.code
        );
        // Top-level returns end the render function, as they do at runtime.
        assert!(
            scope.contains("if (!v) return Astro.redirect('/')"),
            "{}",
            result.code
        );
    }

    #[test]
    fn render_scope_label_names_the_opening_fence() {
        let source = "---\nconst a = 1;\n---\n<p>{a}</p>";
        let result = crate::convert_to_tsx(
            source,
            crate::ConvertOptions {
                filename: Some("Card.astro".to_string()),
                ..Default::default()
            },
        );
        let (label, fence) = result.scope_label.expect("no render scope label");
        assert_eq!(
            &result.code[label.start as usize..label.end as usize],
            "async Card"
        );
        assert_eq!(&source[fence.start as usize..fence.end as usize], "---");
    }

    #[test]
    fn detached_chunks_hand_their_shared_offset_to_an_anchor() {
        let source = "---\nimport X from './X.astro';\nconst a = 1;\ninterface Props { value: string }\n---\n<X />";
        let result = convert(source);
        let starts = |text: &str| source.find(text).unwrap() as u32;
        let anchored: Vec<u32> = result
            .edit_anchors
            .iter()
            .map(|&(_, original)| original)
            .collect();
        assert_eq!(
            anchored,
            [starts("const a"), starts("interface Props")],
            "{}",
            result.code
        );
        let quiet: Vec<&str> = result
            .featureless
            .iter()
            .map(|range| &result.code[range.start as usize..range.end as usize])
            .collect();
        // In emission order: the hoisted `interface` comes first.
        assert_eq!(quiet, ["i", "c"], "{}", result.code);
        for &(generated, _) in &result.edit_anchors {
            assert!(
                !result
                    .mappings
                    .iter()
                    .any(|mapping| mapping.generated == generated && mapping.original.is_some()),
                "anchor at {generated} shares its offset with a mapped run:\n{}",
                result.code
            );
        }
    }
}
