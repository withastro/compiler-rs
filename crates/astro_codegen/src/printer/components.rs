//! Component element printing and hydration handling.
//!
//! Contains `impl AstroCodegen` methods for rendering Astro/framework components
//! via `$$renderComponent`, including hydration directives (`client:load`,
//! `client:visible`, `client:only`, etc.) and `set:html`/`set:text` on components.

use super::AstroCodegen;
use super::elements::ScopeId;
use super::escape::{
    decode_html_entities, escape_double_quotes, escape_double_quotes_keeping_escapes,
    escape_template_literal,
};
use super::expr_to_string;
use super::runtime;
use super::whitespace::has_is_raw_attr;
use crate::scanner::{
    get_jsx_attribute_name, is_custom_element, is_equal_jsx_attribute_name,
    jsx_attribute_value_is_empty,
};
use oxc_ast::ast::*;

/// A client hydration directive parsed from a component's attributes.
pub(super) enum HydrationDirective {
    /// `client:only="framework"` — component is not rendered server-side.
    ClientOnly,
    /// Any other `client:*` directive (e.g. `load`, `idle`, `visible`, `media`).
    Other(String),
}

impl HydrationDirective {
    /// The directive name as it appears after `client:` (e.g. `"only"`, `"load"`).
    pub fn name(&self) -> &str {
        match self {
            Self::ClientOnly => "only",
            Self::Other(name) => name,
        }
    }

    pub fn is_client_only(&self) -> bool {
        matches!(self, Self::ClientOnly)
    }
}

/// Information about component hydration directives.
pub(super) struct HydrationInfo {
    /// The parsed hydration directive.
    pub directive: HydrationDirective,
    /// Component import path (for hydration).
    pub component_path: Option<String>,
    /// Component export name (for hydration).
    pub component_export: Option<String>,
}

/// Information about a `server:defer` directive on a component.
pub(super) struct ServerDeferInfo {
    /// Resolved component import path.
    pub component_path: Option<String>,
    /// Resolved component export name.
    pub component_export: Option<String>,
}

impl<'a> AstroCodegen<'a> {
    /// Check whether a component has a `server:defer` directive.
    pub(super) fn has_server_defer(attrs: &[JSXAttributeItem<'a>]) -> bool {
        attrs.iter().any(|attr| {
            if let JSXAttributeItem::Attribute(attr) = attr {
                is_equal_jsx_attribute_name(&attr.name, "server:defer")
            } else {
                false
            }
        })
    }

    /// Extract hydration info from a component's attributes.
    ///
    /// Returns `None` if the component has no `client:*` directive.
    pub(super) fn extract_hydration_info(attrs: &[JSXAttributeItem<'a>]) -> Option<HydrationInfo> {
        let mut directive = None;

        for attr in attrs {
            if let JSXAttributeItem::Attribute(attr) = attr {
                let name = get_jsx_attribute_name(&attr.name);

                if let Some(d) = name.strip_prefix("client:") {
                    directive = Some(if d == "only" {
                        HydrationDirective::ClientOnly
                    } else {
                        HydrationDirective::Other(d.to_string())
                    });
                }
            }
        }

        Some(HydrationInfo {
            directive: directive?,
            component_path: None,
            component_export: None,
        })
    }

    /// Print a component element via `$$renderComponent`.
    pub(super) fn print_component_element(&mut self, el: &JSXElement<'a>, name: &str) {
        self.add_source_mapping_for_span(el.opening_element.span);
        // Check for client:* directives
        let mut hydration_info = Self::extract_hydration_info(&el.opening_element.attributes);

        // Check if this is a custom element (has dash in name)
        let is_custom = is_custom_element(name);

        // A component with `is:raw` has its slot children treated as raw text.
        let is_raw = has_is_raw_attr(&el.opening_element.attributes);
        if is_raw {
            self.raw_element_depth += 1;
        }

        // Check for server:defer directive
        let mut server_defer_info = if Self::has_server_defer(&el.opening_element.attributes) {
            Some(ServerDeferInfo {
                component_path: None,
                component_export: None,
            })
        } else {
            None
        };

        // Resolve component path and export for client:* hydrated components
        // This info is used for client:component-path and client:component-export attributes
        if let Some(info) = &mut hydration_info {
            // Handle member expressions like "components.A" or "defaultImport.Counter1"
            if let Some((namespace, property)) = name.split_once('.') {
                if let Some(import_info) = self.component_imports.get(namespace) {
                    info.component_path = Some(import_info.specifier.clone());
                    // For namespace imports (import * as x), the export is just the property name
                    // For default imports (import x from), the export is "default.Property"
                    if import_info.is_namespace {
                        info.component_export = Some(property.to_string());
                    } else {
                        // Default or named import - prepend the original export name
                        info.component_export =
                            Some(format!("{}.{}", import_info.export_name, property));
                    }
                }
            } else if let Some(import_info) = self.component_imports.get(name) {
                info.component_path = Some(import_info.specifier.clone());
                info.component_export = Some(import_info.export_name.clone());
            }
        }

        // Resolve component path and export for server:defer components.
        // Use resolve_specifier to match the Go compiler's ResolveIdForMatch behaviour —
        // the resolved path must match the key used in serverIslandNameMap.
        if let Some(info) = &mut server_defer_info {
            if let Some((namespace, property)) = name.split_once('.') {
                if let Some(import_info) = self.component_imports.get(namespace) {
                    info.component_path =
                        Some(self.options.resolve_specifier(&import_info.specifier));
                    info.component_export = if import_info.is_namespace {
                        Some(property.to_string())
                    } else {
                        Some(format!("{}.{}", import_info.export_name, property))
                    };
                }
            } else if let Some(import_info) = self.component_imports.get(name) {
                info.component_path = Some(self.options.resolve_specifier(&import_info.specifier));
                info.component_export = Some(import_info.export_name.clone());
            }
        }

        // Check for set:html or set:text on components (including Fragment)
        let set_directive = Self::extract_set_html_value(&el.opening_element.attributes);

        self.print("${");
        self.print(runtime::RENDER_COMPONENT);
        self.print("(");
        self.print(runtime::RESULT);
        self.print(",\"");
        self.print(name);
        self.print("\",");

        // Component reference - for client:only it's null, for custom elements it's
        // a quoted string, otherwise it's the component identifier
        if hydration_info
            .as_ref()
            .is_some_and(|h| h.directive.is_client_only())
        {
            self.print("null");
        } else if is_custom {
            // Custom elements use quoted tag name: "my-element"
            self.print("\"");
            self.print(name);
            self.print("\"");
        } else {
            self.print(name);
        }

        self.print(",{");

        // Determine if this component should receive a scope identifier.
        // Like the Go compiler, inject scope into all components (PascalCase and custom elements)
        // that are not in the NeverScopedElements list.
        let scope_id = self.scope_id_for(name);

        // Custom elements render as real DOM elements, so `define:vars` must inject
        // the CSS custom properties as an inline `style` prop. PascalCase components
        // are excluded: they control their own root element, so the style cannot be
        // attached to them here.
        let inject_define_vars = is_custom && !self.define_vars_values.is_empty();

        // Components always receive slot as a prop.
        // Only HTML elements have the slot attribute stripped when inside named slots.
        let prev_skip_slot = self.skip_slot_attribute;
        self.skip_slot_attribute = false;

        // Print attributes as object properties (skip set:html/set:text if present)
        self.print_component_attributes_filtered(
            &el.opening_element.attributes,
            hydration_info.as_ref(),
            server_defer_info.as_ref(),
            if set_directive.is_some() {
                Some(&["set:html", "set:text"])
            } else {
                None
            },
            scope_id.as_ref(),
            inject_define_vars,
        );

        self.skip_slot_attribute = prev_skip_slot;

        self.print("}");

        // For set:html or set:text, create a default slot with the content
        if let Some((value, is_html, needs_unescape, is_raw_text, set_span)) = set_directive {
            self.add_source_mapping_for_span(set_span);
            let async_prefix = self.get_async_prefix();
            let slot_params = self.get_slot_params();
            self.print_parts([",{\"default\": ", async_prefix, slot_params]);
            self.print(runtime::RENDER);
            self.print("`");
            if is_raw_text {
                // Escape template-literal syntax so a literal `${...}` isn't evaluated at render time.
                self.print(&escape_template_literal(&value));
            } else {
                self.print("${");
                if is_html && needs_unescape {
                    // set:html with expression needs $$unescapeHTML
                    self.print(runtime::UNESCAPE_HTML);
                    self.print("(");
                    self.print(&value);
                    self.print(")");
                } else {
                    // set:html with string literal or set:text with expression - just interpolate directly
                    self.print(&value);
                }
                self.print("}");
            }
            self.print("`,}");
        } else if !el.children.is_empty() {
            // Print slots if there are children
            self.print(",");
            // Custom elements use browser's Shadow DOM slots, not Astro slots
            // All children go to default slot with their slot attributes preserved
            if is_custom {
                self.print_component_default_slot_only(&el.children);
            } else {
                self.print_component_slots(&el.children);
            }
        }

        // Map the closing tag (e.g. </Card>) to the `)` that closes
        // $$renderComponent(...) — the semantic equivalent in generated code.
        if let Some(ref closing) = el.closing_element {
            self.add_source_mapping_for_span(closing.span);
        }
        self.print(")}");

        if is_raw {
            self.raw_element_depth -= 1;
        }
    }

    /// Extract `set:html` or `set:text` value from component attributes.
    ///
    /// Returns `(value_string, is_html, needs_unescape, is_raw_text, span)`:
    /// - `is_html` is `true` for `set:html`, `false` for `set:text`
    /// - `needs_unescape` is `true` for expressions (need `$$unescapeHTML`), `false` for literals
    /// - `is_raw_text` is `true` for `set:text` with string literal (should be inlined without `${}`)
    pub(super) fn extract_set_html_value(
        attrs: &[JSXAttributeItem<'a>],
    ) -> Option<(String, bool, bool, bool, oxc_span::Span)> {
        for attr in attrs {
            if let JSXAttributeItem::Attribute(attr) = attr {
                let is_html = is_equal_jsx_attribute_name(&attr.name, "set:html");
                let is_text = is_equal_jsx_attribute_name(&attr.name, "set:text");
                if is_html || is_text {
                    let (value, needs_unescape, is_raw_text) = match &attr.value {
                        Some(JSXAttributeValue::StringLiteral(lit)) => {
                            let raw_value = lit.value.as_str();
                            if is_html {
                                // set:html with string literal: needs $$unescapeHTML like any
                                // other value — the user is asking for raw HTML injection.
                                let decoded = decode_html_entities(raw_value);
                                (
                                    Some(format!("\"{}\"", escape_double_quotes(&decoded))),
                                    true,
                                    false,
                                )
                            } else {
                                // set:text with string literal - return raw value for inline
                                (Some(raw_value.to_string()), false, true)
                            }
                        }
                        Some(JSXAttributeValue::ExpressionContainer(expr)) => {
                            if let Some(e) = expr.expression.as_expression() {
                                // set:html always needs $$unescapeHTML — its purpose is to
                                // inject raw HTML, and $$render escapes by default.
                                let needs_unescape = is_html;
                                let code = expr_to_string(e);
                                return Some((code, is_html, needs_unescape, false, attr.span));
                            }
                            (None, true, false)
                        }
                        _ => (None, false, false),
                    };
                    return value.map(|v| (v, is_html, needs_unescape, is_raw_text, attr.span));
                }
            }
        }
        None
    }

    /// Print component attributes, optionally filtering out certain names.
    pub(super) fn print_component_attributes_filtered(
        &mut self,
        attrs: &[JSXAttributeItem<'a>],
        hydration: Option<&HydrationInfo>,
        server_defer: Option<&ServerDeferInfo>,
        skip_names: Option<&[&str]>,
        scope_id: Option<&ScopeId>,
        inject_define_vars: bool,
    ) {
        let mut first = true;

        // Pre-scan for transition attributes
        let mut transition_name = None;
        let mut transition_animate = None;
        let mut transition_persist = None;
        let mut transition_persist_props = None;

        for attr in attrs {
            if let JSXAttributeItem::Attribute(attr) = attr {
                let name = get_jsx_attribute_name(&attr.name);
                if name == "transition:name" {
                    transition_name = Some(attr);
                } else if name == "transition:animate" {
                    transition_animate = Some(attr);
                } else if name == "transition:persist" {
                    transition_persist = Some(attr);
                } else if name == "transition:persist-props" {
                    transition_persist_props = Some(attr);
                }
            }
        }

        // Track whether the scope class was merged into an existing class attribute
        let mut scope_injected = false;

        // Track whether `define:vars` was merged into an existing `style` attribute.
        let mut define_vars_style_injected = false;

        // Determine the scope class string (for class/where strategy only)
        let scope_class = scope_id.and_then(|sid| {
            if sid.is_attribute_strategy() {
                None
            } else {
                Some(sid.class_value())
            }
        });

        // Print regular attributes first
        for attr in attrs {
            match attr {
                JSXAttributeItem::Attribute(attr) => {
                    let name = get_jsx_attribute_name(&attr.name);

                    // Skip slot attribute when skip_slot_attribute is true
                    if name == "slot" && self.skip_slot_attribute {
                        continue;
                    }

                    // Skip filtered names
                    if let Some(names) = skip_names
                        && names.contains(&name.as_ref())
                    {
                        continue;
                    }

                    // Skip transition directives - already handled above
                    if name.starts_with("transition:") {
                        continue;
                    }

                    // Skip is:raw directive
                    if name == "is:raw" {
                        continue;
                    }

                    // Skip server:defer directive
                    if name == "server:defer" {
                        continue;
                    }

                    // Merge `define:vars` into an existing `style` prop on custom elements.
                    if inject_define_vars && name == "style" {
                        if !first {
                            self.print(",");
                        }
                        first = false;
                        self.print_component_style_prop_with_define_vars(attr);
                        define_vars_style_injected = true;
                        self.define_vars_injected = true;
                        continue;
                    }

                    if !first {
                        self.print(",");
                    }
                    first = false;

                    self.add_source_mapping_for_span(attr.span);
                    self.print("\"");
                    // Shorthand names can be arbitrary expression text, so escape as a JS string key.
                    self.print(&escape_double_quotes(&name));
                    self.print("\":");

                    // Merge scope class into class attribute value (matches Go compiler).
                    // Static:  class="foo" → "class":"foo astro-HASH"
                    // Dynamic: class={expr} → "class":(((expr) ?? "") + " astro-HASH")
                    // Boolean: class        → "class":"astro-HASH"
                    if name == "class"
                        && let Some(sc) = &scope_class
                    {
                        match &attr.value {
                            None => {
                                // Boolean class attribute → just the scope class
                                self.print_parts(["\"", sc, "\""]);
                            }
                            Some(JSXAttributeValue::StringLiteral(lit)) => {
                                let val = lit.value.as_str();
                                if val.is_empty() {
                                    self.print_parts(["\"", sc, "\""]);
                                } else {
                                    let escaped = escape_double_quotes_keeping_escapes(val);
                                    self.print_parts(["\"", &escaped, " ", sc, "\""]);
                                }
                            }
                            Some(JSXAttributeValue::ExpressionContainer(expr)) => {
                                self.print("(((");
                                self.print_jsx_expression(&expr.expression);
                                self.print_parts([") ?? \"\") + \" ", sc, "\")"]);
                            }
                            _ => {
                                self.print_parts(["\"", sc, "\""]);
                            }
                        }
                        scope_injected = true;
                        continue;
                    }

                    self.print_component_attr_value(attr);
                }
                JSXAttributeItem::SpreadAttribute(spread) => {
                    if !first {
                        self.print(",");
                    }
                    first = false;
                    self.add_source_mapping_for_span(spread.span);
                    self.print("...(");
                    self.print_expression(&spread.argument);
                    self.print(")");
                }
            }
        }

        // Print transition attributes AFTER regular attributes
        if transition_name.is_some() || transition_animate.is_some() {
            if !first {
                self.print(",");
            }
            first = false;
            // Map to whichever transition attribute comes first
            self.add_transition_source_mapping(
                transition_name.map(|a| a.span),
                transition_animate.map(|a| a.span),
            );
            let name_val = transition_name
                .map_or_else(|| "\"\"".to_string(), |a| Self::get_attr_value_string(a));
            let animate_val = transition_animate
                .map_or_else(|| "\"\"".to_string(), |a| Self::get_attr_value_string(a));
            let hash = self.generate_transition_hash();
            self.print_parts([
                "\"data-astro-transition-scope\":(",
                runtime::RENDER_TRANSITION,
                "(",
                runtime::RESULT,
                ", \"",
                &hash,
                "\", ",
                &animate_val,
                ", ",
                &name_val,
                "))",
            ]);
        }

        // Print transition:persist-props as a data attribute if present
        if let Some(props_attr) = transition_persist_props {
            if !first {
                self.print(",");
            }
            first = false;
            self.add_source_mapping_for_span(props_attr.span);
            let props_val = Self::get_attr_value_string(props_attr);
            self.print_parts(["\"data-astro-transition-persist-props\":", &props_val]);
        }

        if let Some(persist_attr) = transition_persist {
            if !first {
                self.print(",");
            }
            first = false;
            // Persist ID priority: explicit persist value, else transition:name value, else a generated hash.
            if !jsx_attribute_value_is_empty(persist_attr) {
                self.add_source_mapping_for_span(persist_attr.span);
                self.print("\"data-astro-transition-persist\":");
                self.print_component_attr_value(persist_attr);
            } else if let Some(name_attr) = transition_name {
                self.add_source_mapping_for_span(persist_attr.span);
                self.print("\"data-astro-transition-persist\":");
                self.print_component_attr_value(name_attr);
            } else {
                self.add_source_mapping_for_span(persist_attr.span);
                let hash = self.generate_transition_hash();
                self.print_parts([
                    "\"data-astro-transition-persist\":(",
                    runtime::CREATE_TRANSITION_SCOPE,
                    "(",
                    runtime::RESULT,
                    ", \"",
                    &hash,
                    "\"))",
                ]);
            }
        }

        // Add scope identifier as a prop if not already merged into an existing class attribute.
        // For attribute strategy: always add "data-astro-cid-HASH": true
        // For class/where strategy: add "class": "astro-HASH" only if no class attr existed
        if let Some(sid) = scope_id {
            if sid.is_attribute_strategy() {
                if !first {
                    self.print(",");
                }
                first = false;
                let attr_name = sid.data_attr_name();
                self.print_parts(["\"", &attr_name, "\":true"]);
            } else if !scope_injected {
                if !first {
                    self.print(",");
                }
                first = false;
                let sc = sid.class_value();
                self.print_parts(["\"class\":\"", &sc, "\""]);
            }
        }

        // Inject `define:vars` as a `style` prop when the custom element has no
        // existing `style` attribute. Appended after the scope class so the injected
        // props keep a stable `class`-then-`style` order.
        if inject_define_vars && !define_vars_style_injected {
            if !first {
                self.print(",");
            }
            first = false;
            self.print("\"style\":($$definedVars)");
            self.define_vars_injected = true;
        }

        // Add hydration attributes if present
        if let Some(hydration) = hydration {
            if !first {
                self.print(",");
            }
            self.print_parts([
                "\"client:component-hydration\":\"",
                hydration.directive.name(),
                "\"",
            ]);

            if let Some(path) = &hydration.component_path {
                if hydration.directive.is_client_only() && !self.options.has_resolve_path() {
                    self.print_parts([
                        ",\"client:component-path\":($$metadata.resolvePath(\"",
                        path,
                        "\"))",
                    ]);
                } else {
                    self.print_parts([",\"client:component-path\":(\"", path, "\")"]);
                }
            }

            if let Some(export) = &hydration.component_export {
                if hydration.directive.is_client_only() {
                    self.print_parts([",\"client:component-export\":\"", export, "\""]);
                } else {
                    self.print_parts([",\"client:component-export\":(\"", export, "\")"]);
                }
            }
        }

        // Add server:defer attributes if present — these signal the runtime to replace this
        // component with a server island placeholder instead of rendering it inline.
        if let Some(server_defer) = server_defer {
            if !first {
                self.print(",");
            }
            // Matches Go compiler: "server:component-directive": "defer"
            self.print("\"server:component-directive\":\"defer\"");

            if let Some(path) = &server_defer.component_path {
                self.print_parts([",\"server:component-path\":(\"", path, "\")"]);
            }

            if let Some(export) = &server_defer.component_export {
                self.print_parts([",\"server:component-export\":(\"", export, "\")"]);
            }
        }
    }

    /// Print an attribute's value as a component prop value (the RHS of a `"key": value` property).
    fn print_component_attr_value(&mut self, attr: &JSXAttribute<'a>) {
        match &attr.value {
            None => self.print("true"),
            Some(JSXAttributeValue::StringLiteral(lit)) => {
                self.print("\"");
                self.print(&escape_double_quotes_keeping_escapes(lit.value.as_str()));
                self.print("\"");
            }
            Some(JSXAttributeValue::ExpressionContainer(expr)) => {
                if let Some(Expression::TemplateLiteral(_) | Expression::StringLiteral(_)) =
                    expr.expression.as_expression()
                {
                    self.print_jsx_expression(&expr.expression);
                } else {
                    self.print("(");
                    self.print_jsx_expression(&expr.expression);
                    self.print(")");
                }
            }
            Some(JSXAttributeValue::Element(_)) => self.print("\"[JSX]\""),
            Some(JSXAttributeValue::Fragment(_)) => self.print("\"[Fragment]\""),
        }
    }

    /// Print a custom element's `style` prop merged with `$$definedVars`.
    ///
    /// The merged value is printed by [`AstroCodegen::print_define_vars_style_value`]
    /// (shared with the HTML-element path) and wrapped here as the object property
    /// `"style":(<value>)`.
    fn print_component_style_prop_with_define_vars(&mut self, attr: &JSXAttribute<'a>) {
        self.add_source_mapping_for_span(attr.span);
        self.print("\"style\":(");
        self.print_define_vars_style_value(attr);
        self.print(")");
    }
}
