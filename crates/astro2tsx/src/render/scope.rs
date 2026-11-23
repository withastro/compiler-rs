//! Splits frontmatter between module scope and a render function, as Astro's runtime does.

use std::collections::HashMap;

use biome_js_syntax::{
    AnyJsRoot, JsClassDeclaration, JsExport, JsFunctionDeclaration, JsLanguage,
    JsReferenceIdentifier, JsSyntaxKind, JsVariableStatement, TsEnumDeclaration,
    unescape_js_string,
};
use biome_rowan::{AstNode, AstSeparatedList, SyntaxKind, SyntaxNode, TextRange, TextSize};

use crate::props::{binding_names, module_items};

type JsNode = SyntaxNode<JsLanguage>;

#[derive(Debug, Default)]
pub(crate) struct ScopePlan {
    /// Lines before the first item, emitted first so top-of-file insertions land under the fence.
    pub lead: TextRange,
    pub hoisted: Vec<TextRange>,
    pub body: Vec<TextRange>,
}

#[derive(Clone, Copy, PartialEq)]
enum Placement {
    Declaration,
    Export,
    Body,
}

pub(crate) fn plan(root: &AnyJsRoot, errors: &[TextRange]) -> Option<ScopePlan> {
    let items = module_items(root);
    let mut placements: Vec<Placement> = items.iter().map(placement).collect();
    if has_unfinished_declaration(&items, &placements, errors) {
        return None;
    }

    let mut body_values: HashMap<String, usize> = HashMap::new();
    let mut body_types: HashMap<String, usize> = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        if placements[index] == Placement::Body {
            let (bound_values, bound_types) = declared_names(item);
            for name in bound_values {
                body_values.entry(name).or_insert(index);
            }
            for name in bound_types {
                body_types.entry(name).or_insert(index);
            }
        }
    }

    // Hoisted types may query render-scope values (`typeof defaults`), so those move with them.
    let mut pending: Vec<usize> = (0..items.len())
        .filter(|&index| placements[index] != Placement::Body)
        .collect();
    while let Some(index) = pending.pop() {
        let only_erased = placements[index] == Placement::Export;
        for reference in references(&items[index]) {
            if only_erased && !reference.erased {
                // Runtime reads in an export fail at runtime, so they stay type errors.
                continue;
            }
            let namespace = if reference.is_type {
                &body_types
            } else {
                &body_values
            };
            if let Some(&dependency) = namespace.get(&reference.name)
                && placements[dependency] == Placement::Body
            {
                placements[dependency] = Placement::Declaration;
                pending.push(dependency);
            }
        }
    }

    // TypeScript requires overload signatures right before their implementation.
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

/// TypeScript recovers these from the code that follows, which the render function would move.
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
    is_type: bool,
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
    use crate::{ConvertOptions, ConvertResult, convert_to_tsx};

    const OPENER: &str = ";({ async ";

    #[track_caller]
    fn assert_layout(result: &ConvertResult, module: &[&str], scope: &[&str]) {
        let code = &result.code;
        let at = code
            .find(OPENER)
            .unwrap_or_else(|| panic!("no render scope:\n{code}"));
        let (before, after) = code.split_at(at);
        for snippet in module {
            assert!(
                before.contains(snippet),
                "{snippet:?} is not at module scope:\n{code}"
            );
        }
        for snippet in scope {
            assert!(
                after.contains(snippet),
                "{snippet:?} is not in the render scope:\n{code}"
            );
        }
    }

    #[test]
    fn frontmatter_and_template_render_inside_the_scope() {
        let result = convert(
            "---\ninterface Props { value: string }\nconst { value } = Astro.props;\n---\n<p>{value}</p>",
        );
        assert_layout(
            &result,
            &["interface Props {"],
            &[
                ";({ async AstroComponent() {\n/**\n",
                "const Astro: Readonly<import('astro').AstroGlobal<Props, typeof AstroComponent>> = null!;\nconst { value } = Astro.props;",
                "<p>{value}</p>\n</Fragment>\n}});\nexport default function",
            ],
        );
    }

    #[test]
    fn generic_props_get_a_generic_render_scope() {
        let result = convert(
            "---\ninterface Props<T extends string, U = T> { value: T }\nconst { value } = Astro.props;\n---\n<p>{value}</p>",
        );
        assert_layout(
            &result,
            &["interface Props<T extends string, U = T>"],
            &[
                ";({ async AstroComponent<T extends string, U = T>() {",
                "AstroGlobal<Props<T, U>, typeof AstroComponent>> = null!;",
            ],
        );
    }

    #[test]
    fn module_items_are_hoisted_out_of_the_render_scope() {
        let result = convert(
            "---\nconst a = 1;\nimport X from './X.astro';\nexport const prerender = true;\ntype Local = string;\ninterface Props<T extends Local> { value: T }\ndeclare global { interface Window { x: 1 } }\nexport function getStaticPaths() { return []; }\nexport type { Local as Exported };\nconst b = a;\n---\n<X />",
        );
        assert_layout(
            &result,
            &[
                "import X from './X.astro';",
                "export const prerender = true;",
                "type Local = string;",
                "interface Props<T extends Local>",
                "declare global",
                "export function getStaticPaths()",
                "export type { Local as Exported };",
            ],
            &[
                "const a = 1;",
                "const b = a;",
                "ASTRO__Get<ASTRO__InferredGetStaticPath, 'params'>>> = null!;",
            ],
        );
    }

    #[test]
    fn values_queried_by_hoisted_code_are_hoisted_with_it() {
        let cases: [(&str, &[&str], &[&str]); 3] = [
            (
                "---\nconst base = { size: 1 };\nconst defaults = { ...base };\nconst { value } = Astro.props;\nenum Size { S }\nclass Model {}\ninterface Props<T> { value: T; opts: typeof defaults; size: Size; model: Model }\n---\n<p/>",
                &[
                    "const base = { size: 1 };",
                    "const defaults = { ...base };",
                    "enum Size { S }",
                    "class Model {}",
                ],
                &["const { value } = Astro.props;"],
            ),
            // `Tag` is a type parameter in `Props` and a value in the render scope.
            (
                "---\ntype Props<Tag extends string> = { as?: Tag };\nconst { as: Tag } = Astro.props;\n---\n<Tag />",
                &[],
                &["const { as: Tag } = Astro.props;"],
            ),
            // A runtime export's types may query the render scope, but its runtime reads fail.
            (
                "---\nconst queried = 1;\nconst read = 1;\nexport function f(x: typeof queried) { return read; }\n---\n<p/>",
                &["const queried = 1;"],
                &["const read = 1;"],
            ),
        ];
        for (source, module, scope) in cases {
            assert_layout(&convert(source), module, scope);
        }
    }

    #[test]
    fn overload_signatures_stay_with_their_implementation() {
        let overloads = "function pick(v: string): string;\nfunction pick(v: string) {";
        let source =
            "---\nfunction pick(v: string): string;\nfunction pick(v: string) { return v; }\n";
        assert_layout(&convert(&format!("{source}---\n<p/>")), &[], &[overloads]);
        assert_layout(
            &convert(&format!(
                "{source}type Picked = ReturnType<typeof pick>;\n---\n<p/>"
            )),
            &[overloads],
            &[],
        );
    }

    #[test]
    fn chunks_carry_leading_comments_but_not_blank_lines() {
        let result = convert(
            "---\nconst a = 1;\n\n/** Documented. */\ninterface Props { value: string }\n---\n<p/>",
        );
        assert_layout(
            &result,
            &["\n/** Documented. */\ninterface Props { value: string }\n"],
            &["const a = 1;\n\n"],
        );
    }

    #[test]
    fn only_unfinished_imports_and_exports_drop_the_render_scope() {
        let unfinished_statement = convert_to_tsx(
            "---\ninterface Props<T> { value: T }\nconst a = Astro.props.\n---\n<p/>",
            ConvertOptions::default(),
        );
        assert!(unfinished_statement.has_parse_errors);
        assert!(
            unfinished_statement.code.contains(OPENER),
            "{}",
            unfinished_statement.code
        );
        for source in [
            "---\nimport { formatTh\ninterface Props<T> { as?: T }\n---\n<p/>",
            "---\nconst { as } = Astro.props;\nimport { formatTh\n---\n<p/>",
            "---\nimport { formatTh }\nconst { as } = Astro.props;\n---\n<p/>",
            "---\nimport { formatThing } from './ut\nconst { as } = Astro.props;\n---\n<p/>",
            "---\nconst { as } = Astro.props;\nexport const\n---\n<p/>",
        ] {
            let result = convert_to_tsx(source, ConvertOptions::default());
            assert!(result.has_parse_errors, "{source}");
            assert!(!result.code.contains(OPENER), "{}", result.code);
        }
    }

    #[test]
    fn frontmatter_rewrites_survive_reordering() {
        let result = convert(
            "---\nconst v = <string>Astro.props.value\nimport X from './X.astro'\nconst id = <T>(x: T) => x\nif (!v) return Astro.redirect('/')\ninterface Props<T> { value: T }\n---\n<X />",
        );
        assert_layout(
            &result,
            &["import X from './X.astro'"],
            &[
                "const v = (Astro.props.value as string)\n",
                "const id = <T,>(x: T) => x",
                "if (!v) return Astro.redirect('/')",
            ],
        );
    }

    #[test]
    fn outline_label_names_the_opening_fence() {
        let source = "---\nconst a = 1;\n---\n<p>{a}</p>";
        let options = ConvertOptions {
            filename: Some("Card.astro".to_string()),
            ..Default::default()
        };
        let result = convert_to_tsx(source, options);
        let (label, fence) = result.outline_label.expect("no outline label");
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
        let offset = |text: &str| source.find(text).unwrap() as u32;
        let anchored: Vec<u32> = result
            .edit_anchors
            .iter()
            .map(|&(_, original)| original)
            .collect();
        assert_eq!(
            anchored,
            [offset("const a"), offset("interface Props")],
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
