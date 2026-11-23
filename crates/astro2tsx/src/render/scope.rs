//! Generic render scope for components whose `Props` declare type parameters.
//!
//! A generic `Props<T>` only stays generic while the frontmatter and template are
//! checked inside a scope that declares `T`. Mirroring Astro's runtime output, imports
//! and exports (plus erased type declarations) stay at module scope while the other
//! statements and the template move into a generic render function.

use std::collections::{HashMap, HashSet};

use biome_js_parser::{JsParserOptions, parse};
use biome_js_syntax::{
    AnyJsRoot, AnyJsxElementName, JsClassDeclaration, JsExport, JsFunctionDeclaration, JsLanguage,
    JsReferenceIdentifier, JsSyntaxKind, JsVariableStatement, JsxOpeningElement,
    JsxSelfClosingElement, TsEnumDeclaration, unescape_js_string,
};
use biome_languages::JsFileSource;
use biome_rowan::{AstNode, AstSeparatedList, SyntaxNode, TextRange};

use crate::props::{PropsAnalysis, binding_names, module_items};

type JsNode = SyntaxNode<JsLanguage>;

#[derive(Debug, Default)]
pub(crate) struct ScopePlan {
    /// Module items emitted before the render function, in source order.
    pub hoisted: Vec<TextRange>,
    /// Module items emitted inside the render function, in source order.
    pub body: Vec<TextRange>,
    /// Value names bound by statements inside the render function.
    pub body_bindings: HashSet<String>,
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
/// Frontmatter with parse errors keeps the module layout: recovered items may be unfinished
/// imports or exports, and moving them into a function would break their completions.
pub(crate) fn plan(
    root: &AnyJsRoot,
    analysis: &PropsAnalysis,
    has_parse_errors: bool,
) -> Option<ScopePlan> {
    if has_parse_errors || !analysis.has_props || analysis.generics_args.is_empty() {
        return None;
    }
    if !matches!(root, AnyJsRoot::JsModule(_)) {
        return None;
    }
    let items = module_items(root);
    let mut placements: Vec<Placement> = items.iter().map(placement).collect();

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

    let mut plan = ScopePlan::default();
    for (item, placement) in items.iter().zip(placements) {
        let range = item.text_range_with_trivia();
        if placement == Placement::Body {
            plan.body_bindings.extend(declared_names(item).0);
            plan.body.push(range);
        } else {
            plan.hoisted.push(range);
        }
    }
    Some(plan)
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
        | JsSyntaxKind::TS_DECLARE_FUNCTION_DECLARATION
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

/// Render-scope bindings that the generated template opens as JSX tags, in first-use order.
///
/// A polymorphic `as` is optional, so its binding is possibly `undefined`, which TypeScript
/// rejects as a JSX tag. Only plain identifiers are returned; member tags keep their checks.
pub(crate) fn template_tag_bindings(template: &str, bindings: &HashSet<String>) -> Vec<String> {
    if bindings.is_empty() {
        return Vec::new();
    }
    let source = format!("<>{template}</>");
    let parsed = parse(&source, JsFileSource::tsx(), JsParserOptions::default());
    let mut seen = HashSet::new();
    let mut tags = Vec::new();
    for node in parsed.syntax().descendants() {
        let name = if let Some(element) = JsxOpeningElement::cast_ref(&node) {
            element.name().ok()
        } else if let Some(element) = JsxSelfClosingElement::cast_ref(&node) {
            element.name().ok()
        } else {
            None
        };
        let token = match name {
            Some(AnyJsxElementName::JsxReferenceIdentifier(name)) => name.value_token().ok(),
            Some(AnyJsxElementName::JsxName(name)) => name.value_token().ok(),
            _ => None,
        };
        let Some(token) = token else { continue };
        let name = token.text_trimmed();
        if !is_intrinsic_jsx_name(name) && bindings.contains(name) && seen.insert(name.to_string())
        {
            tags.push(name.to_string());
        }
    }
    tags
}

/// TypeScript's rule for JSX tags that name intrinsic elements rather than values.
fn is_intrinsic_jsx_name(name: &str) -> bool {
    name.starts_with(|ch: char| ch.is_ascii_lowercase()) || name.contains('-')
}

#[cfg(test)]
mod tests {
    use crate::test_utils::convert;

    const OPENER: &str = ";(async function<";

    fn split(code: &str) -> (&str, &str) {
        let at = code
            .find(OPENER)
            .unwrap_or_else(|| panic!("no render scope:\n{code}"));
        code.split_at(at)
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
            scope.starts_with(
                ";(async function<T extends string, U = T>(Astro: Readonly<import('astro').AstroGlobal<Props<T, U>, typeof AstroComponent>>) { void Astro;\n"
            ),
            "{}",
            result.code
        );
        assert!(
            scope.contains("const { value } = Astro.props;"),
            "{}",
            result.code
        );
        assert!(
            scope.contains("<p>{value}</p>\n</Fragment>\n});\nexport default function"),
            "{}",
            result.code
        );
    }

    #[test]
    fn non_generic_props_stay_at_module_scope() {
        for source in [
            "---\ninterface Props { value: string }\nconst { value } = Astro.props;\n---\n<p>{value}</p>",
            "---\nconst x = 1;\n---\n<p>{x}</p>",
        ] {
            let result = convert(source);
            assert!(!result.code.contains(OPENER), "{}", result.code);
            assert!(!result.code.contains("throw 0"), "{}", result.code);
        }
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
            scope.contains("ASTRO__Get<ASTRO__InferredGetStaticPath, 'params'>>>) {"),
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
        let result = convert(
            "---\ninterface Props<T> { value: T }\nconst a = 1;\nexport const b = a;\n---\n<p/>",
        );
        let (_, scope) = split(&result.code);
        assert!(scope.contains("const a = 1;"), "{}", result.code);
    }

    #[test]
    fn frontmatter_with_parse_errors_keeps_the_module_layout() {
        let result = crate::convert_to_tsx(
            "---\ninterface Props<T> { value: T }\nconst a = 1;\nimport {\n---\n<p/>",
            crate::ConvertOptions::default(),
        );
        assert!(result.has_parse_errors);
        assert!(!result.code.contains(OPENER), "{}", result.code);
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
        assert!(
            scope.contains("throw  Astro.redirect('/')"),
            "{}",
            result.code
        );
    }

    #[test]
    fn component_tag_bindings_are_narrowed_before_the_template() {
        let source = "---\nimport Imported from './I.astro';\ntype Props<Tag extends string> = { as?: Tag };\nconst { as: Tag } = Astro.props;\nconst Other = Tag;\nconst Member = { Item: Tag };\nconst Unused = Tag;\nconst xs = [1];\nconst _lower = Tag;\n---\n<Tag /><Imported />{xs.map(() => <Other>{xs.length <Unused}</Other>)}<Member.Item /><_lower/>\n<!-- <Unused /> --><code>{'<Unused />'}</code>";
        let result = convert(source);
        assert!(
            result
                .code
                .contains(";{};if (Tag == null) throw 0;if (Other == null) throw 0;<Fragment>"),
            "{}",
            result.code
        );
        // `<_lower/>` is text in Astro, and the others are not tags at all.
        for name in ["Imported", "Member", "Unused", "xs", "_lower"] {
            assert!(
                !result.code.contains(&format!("if ({name} == null)")),
                "{name} was narrowed:\n{}",
                result.code
            );
        }
    }
}
