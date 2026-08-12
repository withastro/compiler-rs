//! Frontmatter analysis shaping the synthetic `export default function` signature.

use biome_js_syntax::{
    AnyJsRoot, JsLanguage, JsSyntaxKind, TsInterfaceDeclaration, TsTypeAliasDeclaration,
    TsTypeParameters,
};
use biome_rowan::{AstNode, AstNodeList, AstSeparatedList, SyntaxNode};

type JsNode = SyntaxNode<JsLanguage>;

#[derive(Debug, Default, Clone)]
pub(crate) struct PropsAnalysis {
    /// A top-level `Props` binding exists: interface, type alias, or import.
    pub has_props: bool,
    /// Full generic parameter list, e.g. `<T extends Foo>`; empty when none.
    pub generics_decl: String,
    /// Bare argument names for the application site `Props<T>`, e.g. `<T>`.
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

/// The interface or type-alias declared by `item`, unwrapping an `export`.
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

/// Local bindings are the only `JS_IDENTIFIER_BINDING`s in an import, so module
/// specifiers and the pre-`as` name in `Props as Other` can never match.
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

/// Detects an `export` binding `getStaticPaths`, including `export { x as getStaticPaths }`.
fn exports_get_static_paths(node: &JsNode) -> bool {
    if node.kind() != JsSyntaxKind::JS_EXPORT {
        return false;
    }
    node.descendants().any(|candidate| {
        if candidate.text_trimmed() != "getStaticPaths" {
            return false;
        }
        match candidate.kind() {
            JsSyntaxKind::JS_IDENTIFIER_BINDING => !is_nested_in_body(&candidate, node),
            JsSyntaxKind::JS_LITERAL_EXPORT_NAME => true,
            JsSyntaxKind::JS_REFERENCE_IDENTIFIER => candidate
                .parent()
                .is_some_and(|p| p.kind() == JsSyntaxKind::JS_EXPORT_NAMED_SHORTHAND_SPECIFIER),
            _ => false,
        }
    })
}

/// Separates an exported binding from a local one inside the exported value's body.
fn is_nested_in_body(node: &JsNode, export: &JsNode) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if &parent == export {
            return false;
        }
        if matches!(
            parent.kind(),
            JsSyntaxKind::JS_FUNCTION_BODY | JsSyntaxKind::JS_BLOCK_STATEMENT
        ) {
            return true;
        }
        current = parent.parent();
    }
    false
}
