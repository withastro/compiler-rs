//! Astro AST scanner.
//!
//! Pre-analyzes an `AstroRoot` AST in a single pass using the `Visit` trait
//! to collect metadata needed by the printer. This separates the analysis
//! phase from the code generation phase.

use std::borrow::Cow;

use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_codegen::Codegen;
use rustc_hash::FxHashSet;

use oxc_allocator::Allocator;

use crate::printer::result::{HoistedScriptType, TransformResultHoistedScript};

/// Information about a hydrated component.
#[derive(Debug, Clone)]
pub struct HydratedComponent {
    /// The component name (e.g., "One", "my-element")
    pub name: String,
    /// Whether this is a custom element (has a dash in the name)
    pub is_custom_element: bool,
}

/// Result of scanning an Astro AST.
///
/// Contains all metadata collected during the analysis pass.
/// The printer consumes this without doing any further analysis.
#[derive(Debug)]
pub struct ScanResult {
    /// Whether the source uses the `Astro` global (e.g., `Astro.props`)
    pub uses_astro_global: bool,
    /// Whether the source uses transition directives (`transition:*`, `server:defer`)
    pub uses_transitions: bool,
    /// Whether the source contains an `await` expression (needs async wrappers)
    pub has_await: bool,
    /// Set of component names (or namespace roots) that use `client:only`.
    /// Used by the printer to detect client:only imports during frontmatter processing.
    pub client_only_component_names: FxHashSet<String>,
    /// Roots of import bindings referenced anywhere the reference survives to the
    /// output: non-`client:only` component instances, frontmatter code, and
    /// template/attribute expressions. A `client:only` import is only safe to strip
    /// if its binding root is absent here.
    pub referenced_bindings: FxHashSet<String>,
    /// Collected hydrated components for metadata
    pub hydrated_components: Vec<HydratedComponent>,
    /// Collected client-only components with full names (including dot-notation)
    pub client_only_components: Vec<HydratedComponent>,
    /// Collected server-deferred components (with `server:defer` directive)
    pub server_deferred_components: Vec<HydratedComponent>,
    /// Collected hydration directive names (e.g., "load", "visible", "only")
    pub hydration_directives: Vec<String>,
    /// Collected hoisted scripts
    pub hoisted_scripts: Vec<TransformResultHoistedScript>,
    /// Whether the source contains an HTML `<template>` element
    pub has_template_element: bool,
}

/// Scans an Astro AST to collect metadata in a single pass.
///
/// Uses the `Visit` trait to walk the entire tree once, detecting:
/// - `Astro` global usage (in frontmatter and template expressions)
/// - `transition:*` / `server:defer` directives
/// - `client:*` hydration directives and component tracking
/// - Hoisted `<script>` elements
pub struct AstroScanner<'a> {
    allocator: &'a Allocator,
    /// Whether we've found an `Astro` identifier reference
    uses_astro_global: bool,
    /// Whether we've found transition directives
    uses_transitions: bool,
    /// Whether we've found an HTML `<template>` element
    has_template_element: bool,
    /// Whether we've found an `await` expression
    has_await: bool,
    /// Set of component names that use `client:only`
    client_only_component_names: FxHashSet<String>,
    /// Roots of import bindings referenced anywhere that survives to output
    referenced_bindings: FxHashSet<String>,
    /// Collected hydrated components
    hydrated_components: Vec<HydratedComponent>,
    /// Collected client-only components (full names including dot-notation)
    client_only_components: Vec<HydratedComponent>,
    /// Collected server-deferred components
    server_deferred_components: Vec<HydratedComponent>,
    /// Hydration directive names
    hydration_directives: Vec<String>,
    /// Collected hoisted scripts
    hoisted_scripts: Vec<TransformResultHoistedScript>,
    /// Whether we are currently inside a non-hoistable element (`<template>`, `<svg>`, `<noscript>`)
    in_non_hoistable: bool,
}

impl<'a> AstroScanner<'a> {
    pub fn new(allocator: &'a Allocator) -> Self {
        Self {
            allocator,
            uses_astro_global: false,
            uses_transitions: false,
            has_template_element: false,
            has_await: false,
            client_only_component_names: FxHashSet::default(),
            referenced_bindings: FxHashSet::default(),
            hydrated_components: Vec::new(),
            client_only_components: Vec::new(),
            server_deferred_components: Vec::new(),
            hydration_directives: Vec::new(),
            hoisted_scripts: Vec::new(),
            in_non_hoistable: false,
        }
    }

    /// Run the scanner on an Astro AST and return the collected metadata.
    pub fn scan(mut self, root: &AstroRoot<'a>) -> ScanResult {
        self.visit_astro_root(root);
        ScanResult {
            uses_astro_global: self.uses_astro_global,
            uses_transitions: self.uses_transitions,
            has_await: self.has_await,
            client_only_component_names: self.client_only_component_names,
            referenced_bindings: self.referenced_bindings,
            hydrated_components: self.hydrated_components,
            client_only_components: self.client_only_components,
            server_deferred_components: self.server_deferred_components,
            hydration_directives: self.hydration_directives,
            hoisted_scripts: self.hoisted_scripts,
            has_template_element: self.has_template_element,
        }
    }

    /// Process a JSX opening element for client:* and transition:* directives.
    fn process_element_directives(&mut self, el: &JSXOpeningElement<'a>) {
        let name = get_jsx_element_name(&el.name);
        let is_component = is_component_name(&name);
        let is_custom = is_custom_element(&name);
        let component_root = name.split('.').next().unwrap_or(&name).to_string();
        let mut node_is_client_only = false;

        for attr in &el.attributes {
            if let JSXAttributeItem::Attribute(attr) = attr {
                let attr_name = get_jsx_attribute_name(&attr.name);

                // Detect transition directives.
                // Note: server:defer does NOT set uses_transitions — it only enables
                // head propagation. Only actual transition:* attributes trigger transition
                // handling (matching Go compiler fix in #1149).
                if attr_name.starts_with("transition:") {
                    self.uses_transitions = true;
                }

                // Detect server:defer components
                if attr_name == "server:defer"
                    && (is_component || is_custom)
                    && !self
                        .server_deferred_components
                        .iter()
                        .any(|c| c.name == name)
                {
                    self.server_deferred_components.push(HydratedComponent {
                        name: name.to_string(),
                        is_custom_element: is_custom,
                    });
                }

                // Detect client:* directives
                if let Some(directive) = attr_name.strip_prefix("client:") {
                    if directive == "only" {
                        node_is_client_only = true;
                        // Store the namespace root (or simple name) for import-level checks
                        if name.contains('.') {
                            if let Some(namespace) = name.split('.').next() {
                                self.client_only_component_names
                                    .insert(namespace.to_string());
                            }
                        } else {
                            self.client_only_component_names.insert(name.to_string());
                        }
                        // Store the full component name for metadata resolution
                        if (is_component || is_custom)
                            && !self.client_only_components.iter().any(|c| c.name == name)
                        {
                            self.client_only_components.push(HydratedComponent {
                                name: name.to_string(),
                                is_custom_element: is_custom,
                            });
                        }
                        if !self.hydration_directives.contains(&"only".to_string()) {
                            self.hydration_directives.push("only".to_string());
                        }
                    } else {
                        if !self.hydration_directives.contains(&directive.to_string()) {
                            self.hydration_directives.push(directive.to_string());
                        }
                        if (is_component || is_custom)
                            && !self.hydrated_components.iter().any(|c| c.name == name)
                        {
                            self.hydrated_components.push(HydratedComponent {
                                name: name.to_string(),
                                is_custom_element: is_custom,
                            });
                        }
                    }
                    break; // Only process first client:* directive
                }
            }
        }

        // A non-client:only instance needs its import for SSR, even when the binding is also used with client:only.
        if (is_component || is_custom) && !node_is_client_only {
            self.referenced_bindings.insert(component_root);
        }
    }

    /// Check if a <script> element should be hoisted and collect it.
    fn try_collect_script(&mut self, el: &JSXElement<'a>) {
        let name = get_jsx_element_name(&el.opening_element.name);
        if name != "script" {
            return;
        }
        if !should_hoist_script(&el.opening_element.attributes) {
            return;
        }

        let attrs: Vec<_> = el.opening_element.attributes.iter().collect();

        // Try AstroScript child first (parsed JS/TS content)
        for child in &el.children {
            if let JSXChild::AstroScript(script) = child {
                self.collect_script_from_program(&script.program, &attrs);
                return;
            }
        }

        // Fall back to JSXText child (raw text content)
        self.collect_script_from_text_children(&el.children, &attrs);
    }

    fn extract_src_attribute(attrs: &[&JSXAttributeItem<'a>]) -> Option<String> {
        let mut src_value: Option<String> = None;
        for attr in attrs {
            if let JSXAttributeItem::Attribute(attr) = attr
                && is_equal_jsx_attribute_name(&attr.name, "src")
                && let Some(JSXAttributeValue::StringLiteral(lit)) = &attr.value
            {
                src_value = Some(lit.value.to_string());
            }
        }
        src_value
    }

    fn push_external_script(&mut self, src: String) {
        self.hoisted_scripts.push(TransformResultHoistedScript {
            script_type: HoistedScriptType::External,
            code: None,
            src: Some(src),
        });
    }

    /// Collect a script from its parsed program and attributes.
    fn collect_script_from_program(
        &mut self,
        program: &Program<'a>,
        attrs: &[&JSXAttributeItem<'a>],
    ) {
        if let Some(src) = Self::extract_src_attribute(attrs) {
            self.push_external_script(src);
        } else {
            let content = get_script_content(self.allocator, program);
            if !content.is_empty() {
                self.hoisted_scripts.push(TransformResultHoistedScript {
                    script_type: HoistedScriptType::Inline,
                    code: Some(content),
                    src: None,
                });
            }
        }
    }

    /// Collect a script from JSXText children (raw text content).
    fn collect_script_from_text_children(
        &mut self,
        children: &[JSXChild<'a>],
        attrs: &[&JSXAttributeItem<'a>],
    ) {
        if let Some(src) = Self::extract_src_attribute(attrs) {
            self.push_external_script(src);
        } else {
            let content: String = children
                .iter()
                .filter_map(|child| {
                    if let JSXChild::Text(text) = child {
                        Some(text.value.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("");

            let content = content.trim();
            if !content.is_empty() {
                self.hoisted_scripts.push(TransformResultHoistedScript {
                    script_type: HoistedScriptType::Inline,
                    code: Some(content.to_string()),
                    src: None,
                });
            }
        }
    }
}

impl<'a> Visit<'a> for AstroScanner<'a> {
    /// Detect `Astro` identifier references in frontmatter and template expressions.
    fn visit_identifier_reference(&mut self, ident: &IdentifierReference<'a>) {
        if ident.name == "Astro" {
            self.uses_astro_global = true;
        }
        // Every surviving reference (frontmatter, expressions, attribute values, and
        // non-client:only tag names) keeps its binding's import alive.
        self.referenced_bindings.insert(ident.name.to_string());
    }

    fn visit_jsx_opening_element(&mut self, el: &JSXOpeningElement<'a>) {
        // A client:only tag emits `null`, not the binding, so its name isn't a
        // reference; attributes still are (their expressions can reference bindings).
        if !opening_element_has_client_only(el) {
            self.visit_jsx_element_name(&el.name);
        }
        self.visit_jsx_attribute_items(&el.attributes);
    }

    // Closing tag names never emit a standalone reference.
    fn visit_jsx_closing_element(&mut self, _el: &JSXClosingElement<'a>) {}

    /// Process JSX elements for directives and script hoisting.
    fn visit_jsx_element(&mut self, el: &JSXElement<'a>) {
        // Check for directives on the opening element
        self.process_element_directives(&el.opening_element);

        let name = get_jsx_element_name(&el.opening_element.name);

        // Check for HTML <template> elements (not components)
        if name == "template" && !is_component_name(&name) {
            self.has_template_element = true;
        }

        // Check for hoistable scripts — if we collect one, skip walking
        // children to avoid visit_astro_script double-collecting the same script.
        if name == "script" {
            if !self.in_non_hoistable && should_hoist_script(&el.opening_element.attributes) {
                self.try_collect_script(el);
            }
            // Don't walk children for any <script> element — AstroScript children
            // inside non-hoistable scripts (e.g. is:inline) must not be collected.
            return;
        }

        // Track non-hoistable context (template, svg, noscript) so scripts
        // inside these elements are not extracted, matching the Go compiler.
        let is_non_hoistable = matches!(name.as_ref(), "svg" | "noscript" | "template");
        let was_in_non_hoistable = self.in_non_hoistable;
        if is_non_hoistable {
            self.in_non_hoistable = true;
        }

        // Continue walking children (the default walk handles this)
        walk::walk_jsx_element(self, el);

        self.in_non_hoistable = was_in_non_hoistable;
    }

    /// Detect `await` expressions, used to determine if async wrappers are needed.
    fn visit_await_expression(&mut self, it: &AwaitExpression<'a>) {
        self.has_await = true;
        walk::walk_await_expression(self, it);
    }

    /// Detect `for await (... of ...)`: needs async, but is not an `AwaitExpression`.
    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        self.has_await |= it.r#await;
        walk::walk_for_of_statement(self, it);
    }

    /// Detect `await using x = ...`: needs async, but is not an `AwaitExpression`.
    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        self.has_await |= it.kind == VariableDeclarationKind::AwaitUsing;
        walk::walk_variable_declaration(self, it);
    }

    /// Process standalone AstroScript nodes (direct children of the root,
    /// not inside a `<script>` JSXElement — those are handled by visit_jsx_element).
    fn visit_astro_script(&mut self, script: &AstroScript<'a>) {
        self.collect_script_from_program(&script.program, &[]);
        // Don't walk into the program — we've already handled it
    }
}

// --- Helper functions (shared with printer) ---

pub fn get_jsx_element_name<'a>(name: &JSXElementName<'a>) -> Cow<'a, str> {
    match name {
        JSXElementName::Identifier(ident) => Cow::Borrowed(ident.name.as_str()),
        JSXElementName::IdentifierReference(ident) => Cow::Borrowed(ident.name.as_str()),
        JSXElementName::NamespacedName(ns) => {
            Cow::Owned(format!("{}:{}", ns.namespace.name, ns.name.name))
        }
        JSXElementName::MemberExpression(expr) => Cow::Owned(get_jsx_member_expression_name(expr)),
        JSXElementName::ThisExpression(_) => Cow::Borrowed("this"),
    }
}

fn get_jsx_member_expression_name(expr: &JSXMemberExpression<'_>) -> String {
    let object_name = match &expr.object {
        JSXMemberExpressionObject::IdentifierReference(ident) => ident.name.to_string(),
        JSXMemberExpressionObject::MemberExpression(inner) => get_jsx_member_expression_name(inner),
        JSXMemberExpressionObject::ThisExpression(_) => "this".to_string(),
    };
    format!("{object_name}.{}", expr.property.name)
}

pub fn get_jsx_attribute_name<'a>(name: &JSXAttributeName<'a>) -> Cow<'a, str> {
    match name {
        JSXAttributeName::Identifier(ident) => Cow::Borrowed(ident.name.as_str()),
        JSXAttributeName::NamespacedName(ns) => {
            Cow::Owned(format!("{}:{}", ns.namespace.name, ns.name.name))
        }
    }
}

pub fn is_equal_jsx_attribute_name(name: &JSXAttributeName<'_>, other: &str) -> bool {
    match name {
        JSXAttributeName::Identifier(ident) => ident.name == other,
        JSXAttributeName::NamespacedName(ns) => {
            if let Some((namespace, name)) = other.split_once(":") {
                ns.namespace.name == namespace && ns.name.name == name
            } else {
                false
            }
        }
    }
}

pub fn is_component_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase()) || name.contains('.') || name.contains(':')
}

pub fn is_custom_element(name: &str) -> bool {
    name.contains('-')
}

fn opening_element_has_client_only(el: &JSXOpeningElement<'_>) -> bool {
    el.attributes.iter().any(|attr| {
        matches!(attr, JSXAttributeItem::Attribute(a) if is_equal_jsx_attribute_name(&a.name, "client:only"))
    })
}

/// Returns `true` if a `<script>` element should be hoisted (bundled via
/// `$$renderScript`).
pub fn should_hoist_script(attrs: &oxc_allocator::Vec<'_, JSXAttributeItem<'_>>) -> bool {
    let mut has_type_module = false;
    let mut has_src = false;
    let mut has_other = false;
    let mut is_inline = false;
    let mut has_define_vars = false;

    for attr in attrs {
        if let JSXAttributeItem::Attribute(attr) = attr {
            let attr_name = get_jsx_attribute_name(&attr.name);
            match attr_name.as_ref() {
                "is:inline" => is_inline = true,
                "define:vars" => has_define_vars = true,
                "src" => has_src = true,
                "type" => {
                    if let Some(JSXAttributeValue::StringLiteral(lit)) = &attr.value
                        && lit.value == "module"
                    {
                        has_type_module = true;
                    } else {
                        has_other = true;
                    }
                }
                _ => has_other = true,
            }
        }
    }

    if is_inline || has_define_vars {
        return false;
    }

    // A script with a `src` pointing to an external file is only hoistable if
    // `src` is the sole attribute (matching Go compiler behaviour). Adding any
    // other attribute — including `type="module"` — means the script is treated
    // as inline HTML and left in place rather than being bundled.
    if has_src {
        return !has_type_module && !has_other;
    }

    // Scripts with no attributes at all, or with just `type="module"`, are hoistable.
    attrs.is_empty() || (has_type_module && !has_other)
}

/// Get the script content as a string by codegen-ing the program.
fn get_script_content(_allocator: &Allocator, program: &Program<'_>) -> String {
    let codegen = Codegen::new();
    let output = codegen.build(program);
    output.code.trim_end().to_string()
}
