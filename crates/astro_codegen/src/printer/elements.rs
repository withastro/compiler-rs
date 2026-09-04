//! HTML element printing, attributes, and element utilities.
//!
//! Contains `impl AstroCodegen` methods for rendering plain HTML elements
//! (non-component), including attribute handling, `set:html`/`set:text`
//! directives on HTML elements, `<slot>` element rendering, transition
//! attributes, and element classification helpers (`is_void_element`,
//! `is_head_element`).

use super::escape::{escape_double_quotes, escape_html_attribute, escape_template_literal};
use super::runtime;
use super::whitespace::{has_is_raw_attr, is_raw_element_name};
use super::{AstroCodegen, expr_to_string};
use crate::css_scoping;
use crate::options::ScopedStyleStrategy;
use crate::scanner::{
    get_jsx_attribute_name, is_equal_jsx_attribute_name, jsx_attribute_value_is_empty,
};
use oxc_ast::ast::*;

/// Scope identifier for an element — either a CSS class or a data attribute,
/// depending on the `scopedStyleStrategy`.
#[derive(Clone)]
pub(super) enum ScopeId {
    /// `where` or `class` strategy: inject `class="astro-{hash}"`.
    Class(String),
    /// `attribute` strategy: inject `data-astro-cid-{hash}` as a boolean attribute.
    DataAttribute(String),
}

impl ScopeId {
    /// The value to embed in class lists / spread attributes (e.g. `"astro-{hash}"`).
    pub(super) fn class_value(&self) -> String {
        match self {
            ScopeId::Class(v) => v.clone(),
            ScopeId::DataAttribute(v) => format!("astro-{v}"),
        }
    }

    /// The attribute name for the `attribute` strategy (e.g. `"data-astro-cid-{hash}`) as a boolean attribute.
    pub(super) fn data_attr_name(&self) -> String {
        match self {
            ScopeId::DataAttribute(v) => format!("data-astro-cid-{v}"),
            ScopeId::Class(_) => unreachable!("data_attr_name called on Class variant"),
        }
    }

    pub(super) fn is_attribute_strategy(&self) -> bool {
        matches!(self, ScopeId::DataAttribute(_))
    }
}

/// Returns `true` for HTML void elements that must not have a closing tag.
pub(super) fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "selectedcontent" // HTML customizable select element
            | "source"
            | "track"
            | "wbr"
    )
}

/// Elements that can appear in `<head>` and should NOT trigger `$$maybeRenderHead`.
pub(super) fn is_head_element(name: &str) -> bool {
    matches!(
        name,
        "html"
            | "head"
            | "base"
            | "basefont"
            | "bgsound"
            | "link"
            | "meta"
            | "noframes"
            | "script"
            | "style"
            | "template"
            | "title"
    )
}

fn source_location(source_text: &str, byte_offset: u32) -> (u32, u32) {
    let offset = (byte_offset as usize).min(source_text.len());
    let bytes = source_text.as_bytes();
    let mut line = 1u32;
    let mut line_start = 0usize;
    let mut index = 0usize;

    while index < offset {
        match bytes[index] {
            b'\r' => {
                line += 1;
                if bytes.get(index + 1) == Some(&b'\n') && index + 1 < offset {
                    index += 1;
                }
                line_start = index + 1;
            }
            b'\n' => {
                line += 1;
                line_start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }

    let column = source_text[line_start..offset].encode_utf16().count() as u32;
    (line, column)
}

impl<'a> AstroCodegen<'a> {
    pub(super) fn add_transition_source_mapping(
        &mut self,
        transition_name: Option<oxc_span::Span>,
        transition_animate: Option<oxc_span::Span>,
    ) {
        if let Some(span) = transition_name.or(transition_animate) {
            self.add_source_mapping_for_span(span);
        }
    }

    pub(super) fn scope_id_for(&self, name: &str) -> Option<ScopeId> {
        if self.has_scoped_styles && css_scoping::should_scope_element(name) {
            let hash = &self.source_hash;
            match self.options.scoped_style_strategy() {
                ScopedStyleStrategy::Attribute => Some(ScopeId::DataAttribute(hash.clone())),
                _ => Some(ScopeId::Class(format!("astro-{hash}"))),
            }
        } else {
            None
        }
    }

    fn print_source_annotation(&mut self, name: &str, span: oxc_span::Span) {
        if !self.options.annotate_source_file
            || name == "html"
            || !css_scoping::should_scope_element(name)
        {
            return;
        }

        let Some(filename) = self.options.filename.clone() else {
            return;
        };
        let filename = escape_html_attribute(&filename).into_owned();
        let (line, column) = source_location(self.source_text, span.start);
        let location = format!("{line}:{column}");
        self.print_parts([
            " data-astro-source-file=\"",
            &filename,
            "\" data-astro-source-loc=\"",
            &location,
            "\"",
        ]);
    }

    /// Print an HTML (non-component) element.
    pub(super) fn print_html_element(&mut self, el: &JSXElement<'a>, name: &str) {
        self.add_source_mapping_for_span(el.opening_element.span);
        // Handle <slot> element specially — it's a slot placeholder, not an HTML element.
        // Unless it has `is:inline`, in which case render as raw HTML.
        if name == "slot" && !Self::has_is_inline_attribute(&el.opening_element.attributes) {
            self.print_slot_element(el);
            return;
        }

        let is_head = name == "head";
        let is_template = name == "template";
        let was_in_head = self.in_head;

        // Track non-hoistable context for nested style elements
        let is_non_hoistable = matches!(name, "svg" | "noscript" | "template");
        let was_in_non_hoistable = self.in_non_hoistable;
        if is_non_hoistable {
            self.in_non_hoistable = true;
        }

        if is_head {
            self.in_head = true;
            self.has_explicit_head = true;
        }

        // Raw elements are those whose text content must never be modified:
        // <pre>, <textarea>, <script>, <style>, etc., and any element with `is:raw`.
        let is_raw = is_raw_element_name(name) || has_is_raw_attr(&el.opening_element.attributes);
        if is_raw {
            self.raw_element_depth += 1;
        }

        // Insert $$maybeRenderHead before the first body HTML element
        self.maybe_insert_render_head(name);

        // Extract set:html and set:text directives
        let set_directive = Self::extract_set_directive(&el.opening_element.attributes);

        // Determine if this element should receive a scope identifier
        let scope_id = self.scope_id_for(name);

        self.print("<");
        self.print(name);

        // Determine if this element should receive $$definedVars style injection
        let inject_define_vars = self.should_inject_define_vars(name);

        // Attributes (excluding set:html and set:text), with scope injection
        self.print_html_attributes(
            &el.opening_element.attributes,
            scope_id.as_ref(),
            inject_define_vars,
        );
        self.print_source_annotation(name, el.opening_element.span);

        self.print(">");

        // Emit template depth tracking for HTML <template> elements
        if is_template {
            self.print_parts(["${", runtime::TEMPLATE_ENTER, "(", runtime::RESULT, ")}"]);
        }

        if is_head {
            self.print_jsx_children_compact(&el.children);
            self.print_parts(["${", runtime::RENDER_HEAD, "(", runtime::RESULT, ")}"]);
            // Mark that head rendering is done — prevents $$maybeRenderHead from being inserted later.
            self.render_head_inserted = true;
        } else if let Some((directive_type, value, needs_unescape, is_raw_text, set_span)) =
            set_directive
        {
            self.add_source_mapping_for_span(set_span);
            if is_raw_text {
                // Escape template-literal syntax so a literal `${...}` isn't evaluated at render time.
                self.print(&escape_template_literal(&value));
            } else if directive_type == "html" && needs_unescape {
                // Only use $$unescapeHTML for non-literal expressions
                self.print_parts(["${", runtime::UNESCAPE_HTML, "(", &value, ")}"]);
            } else {
                // For literals (string/template) or set:text expression, just interpolate
                self.print_parts(["${", &value, "}"]);
            }
        } else if name == "script"
            && el
                .children
                .iter()
                .any(|c| matches!(c, JSXChild::AstroScript(_)))
        {
            // The script content was parsed as JS by oxc (AstroScript child).
            // For non-hoisted scripts (e.g. is:inline), emit the raw source text
            // verbatim. The content lies between the opening tag end and closing
            // tag start in the original source.
            let content_start = el.opening_element.span.end as usize;
            let content_end = el
                .closing_element
                .as_ref()
                .map(|c| c.span.start as usize)
                .unwrap_or(content_start);
            if content_start < content_end {
                let raw = &self.source_text[content_start..content_end];
                self.add_source_mapping_for_span(oxc_span::Span::new(
                    content_start as u32,
                    content_end as u32,
                ));
                self.print(&escape_template_literal(raw));
            }
        } else {
            self.print_jsx_children_compact(&el.children);
        }

        if is_template {
            self.print_parts(["${", runtime::TEMPLATE_EXIT, "(", runtime::RESULT, ")}"]);
        }

        if !is_void_element(name) {
            if let Some(ref closing) = el.closing_element {
                self.add_source_mapping_for_span(closing.span);
            }
            self.print("</");
            self.print(name);
            self.print(">");
        }

        if is_raw {
            self.raw_element_depth -= 1;
        }
        if is_head {
            self.in_head = was_in_head;
        }
        if is_non_hoistable {
            self.in_non_hoistable = was_in_non_hoistable;
        }
    }

    /// Print a `<slot>` element as a `$$renderSlot` call.
    ///
    /// - `<slot />` → `$$renderSlot($$result, $$slots["default"])`
    /// - `<slot name="foo" />` → `$$renderSlot($$result, $$slots["foo"])`
    /// - `<slot><p>fallback</p></slot>` → `$$renderSlot($$result, $$slots["default"], $$render\`<p>fallback</p>\`)`
    fn print_slot_element(&mut self, el: &JSXElement<'a>) {
        let slot_name = Self::extract_slot_name(&el.opening_element.attributes);

        self.print("${");
        self.print(runtime::RENDER_SLOT);
        self.print("(");
        self.print(runtime::RESULT);
        self.print(",$$slots[\"");
        self.print(&slot_name);
        self.print("\"]");

        if !el.children.is_empty() {
            self.print(",");
            self.print(runtime::RENDER);
            self.print("`");
            self.print_jsx_children_compact(&el.children);
            self.print("`");
        }

        if let Some(ref closing) = el.closing_element {
            self.add_source_mapping_for_span(closing.span);
        }
        self.print(")}");
    }

    /// Extract the `name` attribute from a slot element, defaulting to `"default"`.
    fn extract_slot_name(attrs: &[JSXAttributeItem<'a>]) -> String {
        for attr in attrs {
            if let JSXAttributeItem::Attribute(attr) = attr
                && is_equal_jsx_attribute_name(&attr.name, "name")
            {
                if let Some(JSXAttributeValue::StringLiteral(lit)) = &attr.value {
                    return lit.value.to_string();
                }
                if let Some(JSXAttributeValue::ExpressionContainer(expr)) = &attr.value {
                    let expr_str = expr
                        .expression
                        .as_expression()
                        .map(expr_to_string)
                        .unwrap_or_default();
                    return format!("\" + {expr_str} + \"");
                }
            }
        }
        "default".to_string()
    }

    /// Check if an element has the `is:inline` attribute.
    pub(super) fn has_is_inline_attribute(attrs: &[JSXAttributeItem<'a>]) -> bool {
        for attr in attrs {
            if let JSXAttributeItem::Attribute(attr) = attr
                && is_equal_jsx_attribute_name(&attr.name, "is:inline")
            {
                return true;
            }
        }
        false
    }

    /// Extract `set:html`/`set:text` directive from HTML element attributes.
    ///
    /// Returns `(directive_type, value, needs_unescape, is_raw_text, span)`:
    /// - `is_raw_text` is `true` for `set:text` with string literal (should be inlined without `${}`)
    pub(super) fn extract_set_directive(
        attrs: &[JSXAttributeItem<'a>],
    ) -> Option<(&'static str, String, bool, bool, oxc_span::Span)> {
        for attr in attrs {
            if let JSXAttributeItem::Attribute(attr) = attr {
                let is_html = is_equal_jsx_attribute_name(&attr.name, "set:html");
                let is_text = is_equal_jsx_attribute_name(&attr.name, "set:text");
                if is_html || is_text {
                    let directive_type = if is_html { "html" } else { "text" };
                    let (value, needs_unescape, is_raw_text) = match &attr.value {
                        Some(JSXAttributeValue::StringLiteral(lit)) => {
                            if is_text {
                                // set:text with string literal: inline raw text without ${}
                                (lit.value.as_str().to_string(), false, true)
                            } else {
                                // set:html with string literal: needs $$unescapeHTML like any
                                // other value — the user is asking for raw HTML injection.
                                (
                                    format!("\"{}\"", escape_double_quotes(lit.value.as_str())),
                                    true,
                                    false,
                                )
                            }
                        }
                        Some(JSXAttributeValue::ExpressionContainer(expr)) => {
                            let mut value_str = String::new();
                            if let Some(e) = expr.expression.as_expression() {
                                value_str = expr_to_string(e);
                            }
                            // set:html always needs $$unescapeHTML — its purpose is to inject
                            // raw HTML, and $$render escapes by default.
                            let needs_unescape = is_html;
                            (value_str, needs_unescape, false)
                        }
                        _ => ("void 0".to_string(), false, false),
                    };
                    return Some((
                        directive_type,
                        value,
                        needs_unescape,
                        is_raw_text,
                        attr.span,
                    ));
                }
            }
        }
        None
    }

    /// Returns `true` if this element should receive `$$definedVars` style injection.
    fn should_inject_define_vars(&self, name: &str) -> bool {
        !self.define_vars_values.is_empty() && css_scoping::should_scope_element(name)
    }

    /// Print all HTML attributes for an element, handling transition directives,
    /// `class`/`class:list` merging, spread attributes, and optional scope injection.
    ///
    /// `scope_id` contains the scope identifier when the element should be scoped.
    pub(super) fn print_html_attributes(
        &mut self,
        attrs: &[JSXAttributeItem<'a>],
        scope_id: Option<&ScopeId>,
        inject_define_vars: bool,
    ) {
        let mut static_class: Option<&str> = None;
        let mut class_list_expr: Option<&JSXExpressionContainer<'a>> = None;

        let mut transition_name = None;
        let mut transition_animate = None;
        let mut transition_persist = None;

        for attr in attrs {
            if let JSXAttributeItem::Attribute(attr) = attr {
                let name = get_jsx_attribute_name(&attr.name);
                if name == "class" {
                    if let Some(JSXAttributeValue::StringLiteral(lit)) = &attr.value {
                        static_class = Some(lit.value.as_str());
                    }
                } else if name == "class:list" {
                    if let Some(JSXAttributeValue::ExpressionContainer(expr)) = &attr.value {
                        class_list_expr = Some(expr);
                    }
                } else if name == "transition:name" {
                    transition_name = Some(attr);
                } else if name == "transition:animate" {
                    transition_animate = Some(attr);
                } else if name == "transition:persist" {
                    transition_persist = Some(attr);
                }
            }
        }

        let has_merged_class = static_class.is_some() && class_list_expr.is_some();

        // Persist ID priority: explicit persist value, else transition:name value, else a generated hash.
        if let Some(persist_attr) = transition_persist {
            if !jsx_attribute_value_is_empty(persist_attr) {
                self.print_html_attribute_with_name(persist_attr, "data-astro-transition-persist");
            } else if let Some(name_attr) = transition_name {
                self.print_html_attribute_with_name(name_attr, "data-astro-transition-persist");
            } else {
                self.add_source_mapping_for_span(persist_attr.span);
                let hash = self.generate_transition_hash();
                self.print_parts([
                    "${",
                    runtime::ADD_ATTRIBUTE,
                    "(",
                    runtime::CREATE_TRANSITION_SCOPE,
                    "(",
                    runtime::RESULT,
                    ", \"",
                    &hash,
                    "\"), \"data-astro-transition-persist\")}",
                ]);
            }
        }

        if transition_name.is_some() || transition_animate.is_some() {
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
                "${",
                runtime::ADD_ATTRIBUTE,
                "(",
                runtime::RENDER_TRANSITION,
                "(",
                runtime::RESULT,
                ", \"",
                &hash,
                "\", ",
                &animate_val,
                ", ",
                &name_val,
                "), \"data-astro-transition-scope\")}",
            ]);
        }

        let mut scope_injected = false;
        let mut has_class_attr = false;
        let mut define_vars_style_injected = false;

        for attr in attrs {
            if let JSXAttributeItem::Attribute(attr) = attr
                && (is_equal_jsx_attribute_name(&attr.name, "class")
                    || is_equal_jsx_attribute_name(&attr.name, "class:list"))
            {
                has_class_attr = true;
                break;
            }
        }

        for attr in attrs {
            match attr {
                JSXAttributeItem::Attribute(attr) => {
                    let name = get_jsx_attribute_name(&attr.name);
                    // Skip set:html and set:text — handled separately
                    if name == "set:html" || name == "set:text" {
                        continue;
                    }
                    // Skip define:vars — Astro directive, not an HTML attribute
                    if name == "define:vars" {
                        continue;
                    }
                    // Skip is:global — Astro directive, not an HTML attribute
                    if name == "is:global" {
                        continue;
                    }
                    // Skip slot attribute if we're inside a conditional slot context
                    if self.skip_slot_attribute && name == "slot" {
                        continue;
                    }
                    // Skip is:inline and is:raw — Astro directives, not HTML attributes
                    if name == "is:inline" || name == "is:raw" {
                        continue;
                    }
                    // Skip transition directives — already handled above.
                    // Exception: transition:persist-props is a simple rename, handled below.
                    if name.starts_with("transition:") && name != "transition:persist-props" {
                        continue;
                    }
                    // Skip individual class if we're merging with class:list
                    if has_merged_class && name == "class" {
                        continue;
                    }
                    if name == "style" && inject_define_vars {
                        self.print_style_with_define_vars(attr);
                        define_vars_style_injected = true;
                        self.define_vars_injected = true;
                        continue;
                    }
                    if has_merged_class && name == "class:list" {
                        if let (Some(static_val), Some(expr)) = (static_class, class_list_expr) {
                            self.add_source_mapping_for_span(attr.span);
                            self.print_parts(["${", runtime::ADD_ATTRIBUTE, "(["]);
                            if let Some(sid) = scope_id {
                                if sid.is_attribute_strategy() {
                                    // For attribute strategy, don't merge into class
                                    let escaped = escape_double_quotes(static_val);
                                    self.print_parts(["\"", &escaped, "\""]);
                                } else {
                                    let escaped = escape_double_quotes(static_val);
                                    let class_val = sid.class_value();
                                    self.print_parts(["\"", &escaped, " ", &class_val, "\""]);
                                    scope_injected = true;
                                }
                            } else {
                                let escaped = escape_double_quotes(static_val);
                                self.print_parts(["\"", &escaped, "\""]);
                            }
                            self.print(", ");
                            self.print_jsx_expression(&expr.expression);
                            self.print("], \"class:list\")}");
                        }
                        continue;
                    }
                    if let Some(sid) = scope_id
                        && !sid.is_attribute_strategy()
                        && name == "class"
                    {
                        self.print_html_attribute_with_scope(attr, &sid.class_value());
                        scope_injected = true;
                        continue;
                    }
                    if let Some(sid) = scope_id
                        && !sid.is_attribute_strategy()
                        && name == "class:list"
                    {
                        self.print_class_list_with_scope(attr, &sid.class_value());
                        scope_injected = true;
                        continue;
                    }
                    // transition:persist-props → data-astro-transition-persist-props
                    // Simple key rename, like the Go compiler does.
                    if name == "transition:persist-props" {
                        self.print_html_attribute_with_name(
                            attr,
                            "data-astro-transition-persist-props",
                        );
                        continue;
                    }
                    self.print_html_attribute(attr);
                }
                JSXAttributeItem::SpreadAttribute(spread) => {
                    self.add_source_mapping_for_span(spread.span);
                    if let Some(sid) = scope_id
                        && !has_class_attr
                        && !scope_injected
                    {
                        self.print_parts(["${", runtime::SPREAD_ATTRIBUTES, "("]);
                        self.print_expression(&spread.argument);
                        // Always pass the class through $$spreadAttributes for runtime
                        // merging, regardless of scoped style strategy. The runtime's
                        // spreadAttributes only handles { class: ... } — it doesn't
                        // process arbitrary data attributes.
                        // For attribute strategy, the data-astro-cid-* attribute is
                        // added directly on the element by the fallback below.
                        let sc = sid.class_value();
                        self.print_parts([",undefined,{\"class\":\"", &sc, "\"})}"]);
                        // Note: do NOT set scope_injected here for attribute strategy,
                        // so the data-astro-cid-* attribute is still added directly
                        // on the element by the fallback at the end of this function.
                        if !sid.is_attribute_strategy() {
                            scope_injected = true;
                        }
                        continue;
                    }
                    self.print_parts(["${", runtime::SPREAD_ATTRIBUTES, "("]);
                    self.print_expression(&spread.argument);
                    self.print(")}");
                }
            }
        }

        if inject_define_vars && !define_vars_style_injected {
            self.print_parts(["${", runtime::ADD_ATTRIBUTE, "($$definedVars, \"style\")}"]);
            self.define_vars_injected = true;
        }

        if let Some(sid) = scope_id
            && !scope_injected
        {
            if sid.is_attribute_strategy() {
                let attr_name = sid.data_attr_name();
                self.print_parts([" ", &attr_name]);
            } else {
                let class_val = sid.class_value();
                self.print_parts([" class=\"", &class_val, "\""]);
            }
        }
    }

    /// Get attribute value as a string representation for codegen.
    pub(super) fn get_attr_value_string(attr: &JSXAttribute<'a>) -> String {
        match &attr.value {
            Some(JSXAttributeValue::StringLiteral(lit)) => {
                format!("\"{}\"", escape_double_quotes(lit.value.as_str()))
            }
            Some(JSXAttributeValue::ExpressionContainer(expr)) => {
                if let Some(e) = expr.expression.as_expression() {
                    let source = expr_to_string(e);
                    // Template literals don't need parens, but other expressions do
                    if matches!(e, Expression::TemplateLiteral(_)) {
                        source
                    } else {
                        format!("({source})")
                    }
                } else {
                    "\"\"".to_string()
                }
            }
            _ => "\"\"".to_string(),
        }
    }

    /// Generate a hash for transition scope.
    pub(super) fn generate_transition_hash(&mut self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let counter = self.transition_counter;
        self.transition_counter += 1;

        // Hash the combination of source hash + counter (like Go's "%s-%v" format)
        let mut hasher = DefaultHasher::new();
        format!("{}-{}", self.source_hash, counter).hash(&mut hasher);
        let hash = hasher.finish();

        Self::to_base32_like(hash)
    }

    /// Print a `style` attribute merged with `$$definedVars` on an HTML element.
    ///
    /// The merged value is printed by [`AstroCodegen::print_define_vars_style_value`]
    /// (shared with the custom-element props path) and wrapped here as
    /// `${$$addAttribute(<value>, "style")}`.
    fn print_style_with_define_vars(&mut self, attr: &JSXAttribute<'a>) {
        self.add_source_mapping_for_span(attr.span);
        self.print_parts(["${", runtime::ADD_ATTRIBUTE, "("]);
        self.print_define_vars_style_value(attr);
        self.print(", \"style\")}");
    }

    /// Print a class attribute with scope class appended.
    fn print_html_attribute_with_scope(&mut self, attr: &JSXAttribute<'a>, scope_class: &str) {
        self.add_source_mapping_for_span(attr.span);
        match &attr.value {
            None => {
                self.print_parts([" class=\"", scope_class, "\""]);
            }
            Some(JSXAttributeValue::StringLiteral(lit)) => {
                let val = lit.value.as_str();
                if val.is_empty() {
                    self.print_parts([" class=\"", scope_class, "\""]);
                } else {
                    let escaped = escape_html_attribute(val);
                    self.print_parts([" class=\"", &escaped, " ", scope_class, "\""]);
                }
            }
            Some(JSXAttributeValue::ExpressionContainer(expr)) => {
                // Output: ${$$addAttribute((expr ?? "") + " astro-XXXX", "class")}
                self.print_parts(["${", runtime::ADD_ATTRIBUTE, "(("]);
                self.print_jsx_expression(&expr.expression);
                self.print_parts([" ?? \"\") + \" ", scope_class, "\", \"class\")}"]);
            }
            _ => {
                self.print_parts([" class=\"", scope_class, "\""]);
            }
        }
    }

    /// Print a class:list attribute with scope class appended.
    fn print_class_list_with_scope(&mut self, attr: &JSXAttribute<'a>, scope_class: &str) {
        self.add_source_mapping_for_span(attr.span);
        match &attr.value {
            Some(JSXAttributeValue::ExpressionContainer(expr)) => {
                // class:list={expr} → ${$$addAttribute([expr, "astro-XXXX"], "class:list")}
                self.print_parts(["${", runtime::ADD_ATTRIBUTE, "([("]);
                self.print_jsx_expression(&expr.expression);
                self.print_parts([")", ", \"", scope_class, "\"], \"class:list\")}"]);
            }
            _ => {
                self.print_parts([" class=\"", scope_class, "\""]);
            }
        }
    }

    /// Print a single HTML attribute (static or dynamic).
    fn print_html_attribute(&mut self, attr: &JSXAttribute<'a>) {
        let name = get_jsx_attribute_name(&attr.name);
        self.print_html_attribute_with_name(attr, &name);
    }

    /// Print a single HTML attribute using the given output name (for key renames).
    fn print_html_attribute_with_name(&mut self, attr: &JSXAttribute<'a>, name: &str) {
        self.add_source_mapping_for_span(attr.span);
        let escaped_name = escape_template_literal(name);
        match &attr.value {
            None => {
                self.print(" ");
                self.print(&escaped_name);
            }
            Some(value) => match value {
                JSXAttributeValue::StringLiteral(lit) => {
                    self.print(" ");
                    self.print(&escaped_name);
                    self.print("=\"");
                    self.print(&escape_html_attribute(lit.value.as_str()));
                    self.print("\"");
                }
                JSXAttributeValue::ExpressionContainer(expr) => {
                    self.print_parts(["${", runtime::ADD_ATTRIBUTE, "("]);
                    self.print_jsx_expression(&expr.expression);
                    self.print(", \"");
                    // Shorthand names can be arbitrary expression text, so escape as a JS string key.
                    self.print(&escape_double_quotes(name));
                    self.print("\")}");
                }
                JSXAttributeValue::Element(el) => {
                    self.print(" ");
                    self.print(&escaped_name);
                    self.print("=\"");
                    self.print_jsx_element(el);
                    self.print("\"");
                }
                JSXAttributeValue::Fragment(frag) => {
                    self.print(" ");
                    self.print(&escaped_name);
                    self.print("=\"");
                    self.print_jsx_fragment(frag);
                    self.print("\"");
                }
            },
        }
    }
}
