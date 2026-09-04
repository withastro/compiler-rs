use biome_js_syntax::{
    AnyJsDeclarationClause, AnyJsExportClause, AnyJsExportNamedSpecifier, AnyJsRoot, JsExport,
    JsIdentifierBinding, JsLanguage, JsSyntaxKind, JsSyntaxToken, TsInterfaceDeclaration,
    TsTypeAliasDeclaration, TsTypeParameters, unescape_js_string,
};
use biome_rowan::{AstNode, AstNodeList, AstSeparatedList, SyntaxNode};

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
    if item.kind() == JsSyntaxKind::JS_EXPORT {
        return item
            .children()
            .find(|child| TYPE_DECLARATIONS.contains(&child.kind()));
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
                id.syntax().text_trimmed().to_string(),
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
                id.syntax().text_trimmed().to_string(),
                decl.type_parameters(),
            )
        }
        _ => return,
    };
    if name != "Props" {
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
    analysis.generics_decl = parameters.syntax().text_trimmed().to_string();
    analysis.generics_args = format!("<{}>", names.join(", "));
}

/// Import identifier bindings exclude module specifiers and pre-`as` names.
fn import_binds_props(import: &JsNode) -> bool {
    import.descendants().any(|node| {
        node.kind() == JsSyntaxKind::JS_IDENTIFIER_BINDING && node.text_trimmed() == "Props"
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
    JsIdentifierBinding::cast_ref(binding).is_some_and(|binding| {
        binding
            .name_token()
            .is_ok_and(identifier_token_is_get_static_paths)
    }) || binding.descendants().any(|candidate| {
        JsIdentifierBinding::cast_ref(&candidate).is_some_and(|binding| {
            binding
                .name_token()
                .is_ok_and(identifier_token_is_get_static_paths)
        })
    })
}

fn identifier_token_is_get_static_paths(token: JsSyntaxToken) -> bool {
    unescape_js_string(token.token_text_trimmed()).text() == "getStaticPaths"
}
