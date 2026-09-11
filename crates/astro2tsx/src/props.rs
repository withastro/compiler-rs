use biome_js_syntax::{
    AnyJsDeclarationClause, AnyJsExportClause, AnyJsExportNamedSpecifier, AnyJsRoot, JsExport,
    JsIdentifierBinding, JsLanguage, JsSyntaxKind, JsSyntaxToken, TsInterfaceDeclaration,
    TsTypeAliasDeclaration, TsTypeParameters, unescape_js_string,
};
use biome_rowan::{AstNode, AstNodeList, AstSeparatedList, SyntaxNode, WalkEvent};

type JsNode = SyntaxNode<JsLanguage>;

#[derive(Debug, Default, Clone)]
pub(crate) struct PropsAnalysis {
    pub has_props: bool,
    pub generics_decl: String,
    pub generics_args: String,
    pub has_get_static_paths: bool,
}

pub(crate) fn analyze(root: &AnyJsRoot) -> PropsAnalysis {
    let mut analysis = PropsAnalysis::default();

    for top in module_items(root) {
        if let Some(declaration) = type_declaration_in(&top) {
            inspect_type_declaration(&mut analysis, &declaration);
        } else if top.kind() == JsSyntaxKind::JS_IMPORT && import_binds_props(&top) {
            analysis.has_props = true;
        }
        if !analysis.has_get_static_paths && exports_get_static_paths(&top) {
            analysis.has_get_static_paths = true;
        }
    }

    analysis
}

fn type_declaration_in(item: &JsNode) -> Option<JsNode> {
    const TYPE_DECLARATIONS: [JsSyntaxKind; 2] = [
        JsSyntaxKind::TS_INTERFACE_DECLARATION,
        JsSyntaxKind::TS_TYPE_ALIAS_DECLARATION,
    ];
    if TYPE_DECLARATIONS.contains(&item.kind()) {
        return Some(item.clone());
    }
    if matches!(
        item.kind(),
        JsSyntaxKind::JS_EXPORT
            | JsSyntaxKind::TS_DECLARE_STATEMENT
            | JsSyntaxKind::TS_EXPORT_DECLARE_CLAUSE
    ) {
        return item
            .children()
            .find_map(|child| type_declaration_in(&child));
    }
    None
}

fn inspect_type_declaration(analysis: &mut PropsAnalysis, declaration: &JsNode) {
    let (name, type_parameters) = match declaration.kind() {
        JsSyntaxKind::TS_INTERFACE_DECLARATION => {
            let Some(decl) = TsInterfaceDeclaration::cast_ref(declaration) else {
                return;
            };
            let Ok(id) = decl.id() else { return };
            (
                id.syntax()
                    .first_token()
                    .map(|token| unescape_js_string(token.token_text_trimmed()).to_string()),
                decl.type_parameters(),
            )
        }
        JsSyntaxKind::TS_TYPE_ALIAS_DECLARATION => {
            let Some(decl) = TsTypeAliasDeclaration::cast_ref(declaration) else {
                return;
            };
            let Ok(id) = decl.binding_identifier() else {
                return;
            };
            (
                id.syntax()
                    .first_token()
                    .map(|token| unescape_js_string(token.token_text_trimmed()).to_string()),
                decl.type_parameters(),
            )
        }
        _ => return,
    };
    if name.as_deref() != Some("Props") {
        return;
    }
    analysis.has_props = true;
    if let Some(parameters) = type_parameters {
        fill_generics(analysis, &parameters);
    }
}

fn fill_generics(analysis: &mut PropsAnalysis, parameters: &TsTypeParameters) {
    let names: Vec<String> = parameters
        .items()
        .iter()
        .filter_map(|parameter| {
            let name = parameter.ok()?.name().ok()?;
            Some(name.ident_token().ok()?.text_trimmed().to_string())
        })
        .collect();
    if names.is_empty() {
        return;
    }
    let declarations: Vec<String> = parameters
        .items()
        .iter()
        .filter_map(Result::ok)
        .filter_map(|parameter| {
            let name = parameter.name().ok()?;
            let mut declaration = name.syntax().text_trimmed().to_string();
            if let Some(constraint) = parameter.constraint() {
                declaration.push(' ');
                declaration.push_str(&constraint.syntax().text_trimmed().to_string());
            }
            if let Some(default) = parameter.default() {
                declaration.push(' ');
                declaration.push_str(&default.syntax().text_trimmed().to_string());
            }
            Some(declaration)
        })
        .collect();
    analysis.generics_decl = format!("<{}>", declarations.join(", "));
    analysis.generics_args = format!("<{}>", names.join(", "));
}

fn import_binds_props(import: &JsNode) -> bool {
    import.descendants().any(|node| {
        JsIdentifierBinding::cast(node).is_some_and(|binding| {
            binding
                .name_token()
                .is_ok_and(|token| unescape_js_string(token.token_text_trimmed()) == "Props")
        })
    })
}

fn module_items(root: &AnyJsRoot) -> Vec<JsNode> {
    match root {
        AnyJsRoot::JsModule(module) => module.items().iter().map(|i| i.into_syntax()).collect(),
        AnyJsRoot::JsScript(script) => script
            .statements()
            .iter()
            .map(|s| s.into_syntax())
            .collect(),
        AnyJsRoot::TsDeclarationModule(decl) => {
            decl.items().iter().map(|i| i.into_syntax()).collect()
        }
        _ => Vec::new(),
    }
}

pub(crate) fn exports_name(root: &AnyJsRoot, name: &str) -> bool {
    module_items(root).iter().any(|item| {
        let Some(export) = JsExport::cast_ref(item) else {
            return false;
        };
        let Ok(clause) = export.export_clause() else {
            return false;
        };
        match clause {
            AnyJsExportClause::AnyJsDeclarationClause(declaration) => {
                declaration_exports_name(&declaration, name)
            }
            AnyJsExportClause::TsExportDeclareClause(clause) => clause
                .declaration()
                .is_ok_and(|declaration| declaration_exports_name(&declaration, name)),
            AnyJsExportClause::JsExportNamedClause(clause) => clause
                .specifiers()
                .iter()
                .filter_map(Result::ok)
                .any(|specifier| match specifier {
                    AnyJsExportNamedSpecifier::JsExportNamedShorthandSpecifier(specifier) => {
                        specifier
                            .name()
                            .and_then(|name| name.value_token())
                            .is_ok_and(|token| {
                                unescape_js_string(token.token_text_trimmed()) == name
                            })
                    }
                    AnyJsExportNamedSpecifier::JsExportNamedSpecifier(specifier) => specifier
                        .exported_name()
                        .and_then(|name| name.inner_string_text())
                        .is_ok_and(|text| unescape_js_string(text) == name),
                }),
            AnyJsExportClause::JsExportNamedFromClause(clause) => clause
                .specifiers()
                .iter()
                .filter_map(Result::ok)
                .any(|specifier| {
                    let exported = match specifier.export_as() {
                        Some(alias) => alias.exported_name(),
                        None => specifier.source_name(),
                    };
                    exported
                        .and_then(|name| name.inner_string_text())
                        .is_ok_and(|text| unescape_js_string(text) == name)
                }),
            AnyJsExportClause::JsExportFromClause(clause) => {
                clause.export_as().is_some_and(|alias| {
                    alias
                        .exported_name()
                        .and_then(|name| name.inner_string_text())
                        .is_ok_and(|text| unescape_js_string(text) == name)
                })
            }
            _ => false,
        }
    })
}

fn declaration_exports_name(declaration: &AnyJsDeclarationClause, name: &str) -> bool {
    match declaration {
        AnyJsDeclarationClause::JsVariableDeclarationClause(clause) => {
            clause.declaration().is_ok_and(|declaration| {
                declaration
                    .declarators()
                    .iter()
                    .filter_map(Result::ok)
                    .any(|declarator| {
                        declarator
                            .id()
                            .is_ok_and(|binding| binding_has_name(binding.syntax(), name))
                    })
            })
        }
        AnyJsDeclarationClause::TsModuleDeclaration(declaration) => declaration
            .name()
            .ok()
            .and_then(|name| name.syntax().first_token())
            .is_some_and(|token| unescape_js_string(token.token_text_trimmed()) == name),
        _ => declaration
            .syntax()
            .children()
            .filter(|node| {
                matches!(
                    node.kind(),
                    JsSyntaxKind::JS_IDENTIFIER_BINDING | JsSyntaxKind::TS_IDENTIFIER_BINDING
                )
            })
            .filter_map(|binding| binding.first_token())
            .any(|token| unescape_js_string(token.token_text_trimmed()) == name),
    }
}

fn exports_get_static_paths(node: &JsNode) -> bool {
    let Some(export) = JsExport::cast_ref(node) else {
        return false;
    };
    let Ok(clause) = export.export_clause() else {
        return false;
    };
    match clause {
        AnyJsExportClause::AnyJsDeclarationClause(declaration) => {
            declaration_exports_get_static_paths(&declaration)
        }
        AnyJsExportClause::JsExportNamedClause(clause) => clause
            .specifiers()
            .iter()
            .filter_map(Result::ok)
            .any(|specifier| match specifier {
                AnyJsExportNamedSpecifier::JsExportNamedShorthandSpecifier(specifier) => specifier
                    .name()
                    .and_then(|name| name.value_token())
                    .is_ok_and(identifier_token_is_get_static_paths),
                AnyJsExportNamedSpecifier::JsExportNamedSpecifier(specifier) => {
                    specifier
                        .exported_name()
                        .and_then(|name| name.inner_string_text())
                        .map(unescape_js_string)
                        .is_ok_and(|name| name == "getStaticPaths")
                        && specifier
                            .local_name()
                            .and_then(|name| name.value_token())
                            .is_ok_and(identifier_token_is_get_static_paths)
                }
            }),
        _ => false,
    }
}

fn declaration_exports_get_static_paths(declaration: &AnyJsDeclarationClause) -> bool {
    match declaration {
        AnyJsDeclarationClause::JsClassDeclaration(declaration) => declaration
            .id()
            .is_ok_and(|binding| binding_has_get_static_paths(binding.syntax())),
        AnyJsDeclarationClause::JsFunctionDeclaration(declaration) => declaration
            .id()
            .is_ok_and(|binding| binding_has_get_static_paths(binding.syntax())),
        AnyJsDeclarationClause::JsVariableDeclarationClause(clause) => {
            clause.declaration().is_ok_and(|declaration| {
                declaration
                    .declarators()
                    .iter()
                    .filter_map(Result::ok)
                    .any(|declarator| {
                        declarator
                            .id()
                            .is_ok_and(|binding| binding_has_get_static_paths(binding.syntax()))
                    })
            })
        }
        AnyJsDeclarationClause::TsDeclareFunctionDeclaration(declaration) => declaration
            .id()
            .is_ok_and(|binding| binding_has_get_static_paths(binding.syntax())),
        AnyJsDeclarationClause::TsEnumDeclaration(declaration) => declaration
            .id()
            .is_ok_and(|binding| binding_has_get_static_paths(binding.syntax())),
        _ => false,
    }
}

fn binding_has_get_static_paths(binding: &JsNode) -> bool {
    binding_has_name(binding, "getStaticPaths")
}

fn binding_has_name(binding: &JsNode, name: &str) -> bool {
    let mut walk = binding.preorder();
    while let Some(event) = walk.next() {
        let WalkEvent::Enter(node) = event else {
            continue;
        };
        match node.kind() {
            JsSyntaxKind::JS_IDENTIFIER_BINDING => {
                if JsIdentifierBinding::cast(node).is_some_and(|binding| {
                    binding
                        .name_token()
                        .is_ok_and(|token| unescape_js_string(token.token_text_trimmed()) == name)
                }) {
                    return true;
                }
            }
            JsSyntaxKind::JS_ARRAY_BINDING_PATTERN
            | JsSyntaxKind::JS_ARRAY_BINDING_PATTERN_ELEMENT_LIST
            | JsSyntaxKind::JS_ARRAY_BINDING_PATTERN_REST_ELEMENT
            | JsSyntaxKind::JS_ARRAY_BINDING_PATTERN_ELEMENT
            | JsSyntaxKind::JS_OBJECT_BINDING_PATTERN
            | JsSyntaxKind::JS_OBJECT_BINDING_PATTERN_PROPERTY_LIST
            | JsSyntaxKind::JS_OBJECT_BINDING_PATTERN_PROPERTY
            | JsSyntaxKind::JS_OBJECT_BINDING_PATTERN_SHORTHAND_PROPERTY
            | JsSyntaxKind::JS_OBJECT_BINDING_PATTERN_REST => {}
            _ => walk.skip_subtree(),
        }
    }
    false
}

fn identifier_token_is_get_static_paths(token: JsSyntaxToken) -> bool {
    unescape_js_string(token.token_text_trimmed()).text() == "getStaticPaths"
}

#[cfg(test)]
mod tests {
    use crate::test_utils::convert;

    #[test]
    fn aliases_do_not_inherit_nested_type_parameters() {
        for fields in ["", "src: Promise<{ default: string }>;"] {
            let result = convert(&format!(
                "---\ninterface LocalImageProps {{ {fields} }}\ntype Props = LocalImageProps;\n---"
            ));
            assert!(
                result
                    .code
                    .contains("function AstroComponent(_props: Props)"),
                "{}",
                result.code
            );
        }
    }

    #[test]
    fn nested_generic_constraints_are_preserved() {
        let result = convert(
            "---\ninterface Props<T extends Other<{ [key: string]: any }>> {}\n---\n<div/>",
        );
        assert!(
            result.code.contains(
                "function AstroComponent<T extends Other<{ [key: string]: any }>>(_props: Props<T>)"
            ),
            "{}",
            result.code
        );
    }

    #[test]
    fn props_names_and_export_bindings_are_scope_aware() {
        for declaration in [
            "declare interface Props { value: string }",
            "export declare interface Props { value: string }",
            "interface Pr\\u006fps { value: string }",
            "import type { Other as Pr\\u006fps } from './types';",
        ] {
            let result = convert(&format!("---\n{declaration}\n---"));
            assert!(result.code.contains("_props: Props"), "{}", result.code);
        }
        for binding in [
            "{ paths = (() => { const getStaticPaths = () => []; return getStaticPaths; })() }",
            "{ [(() => { const getStaticPaths = 'paths'; return getStaticPaths; })()]: paths }",
        ] {
            let result = convert(&format!("---\nexport const {binding} = {{}};\n---"));
            assert!(
                !result.code.contains("ReturnType<typeof getStaticPaths>"),
                "{}",
                result.code
            );
        }
        for binding in [
            "{ getStaticPaths }",
            "{ paths: getStaticPaths }",
            "[getStaticPaths = () => []]",
            "{ nested: { getStaticPaths } }",
        ] {
            let result = convert(&format!("---\nexport const {binding} = data;\n---"));
            assert!(
                result.code.contains("ReturnType<typeof getStaticPaths>"),
                "{}",
                result.code
            );
        }
    }
}
