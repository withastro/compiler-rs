//! Slot analysis and printing.
//!
//! This module handles two concerns:
//!
//! 1. **Slot analysis** — pure functions that inspect JSX children and expressions
//!    to determine which slots they belong to (`SlotValue`, `ExpressionSlotInfo`,
//!    `extract_slots_from_expression`, etc.).
//!
//! 2. **Slot printing** — `impl AstroCodegen` methods that emit the slot objects
//!    passed to `$$renderComponent` (`print_component_slots`,
//!    `print_conditional_slot_*`, etc.).

use oxc_ast::ast::*;
use oxc_span::GetSpan;

use super::AstroCodegen;
use super::AwaitDetector;
use super::escape::escape_double_quotes;
use super::runtime;
use super::{expr_to_string, gen_to_string};
use crate::options::CompactMode;

/// Represents a slot attribute value — either a static string or a dynamic expression.
#[derive(Debug, Clone)]
pub(super) enum SlotValue {
    /// Static slot name like `slot="header"`.
    Static(String),
    /// Dynamic slot name like `slot={name}` — stores the expression as a string and the attribute span.
    Dynamic(String, oxc_span::Span),
}

/// Extract the static slot name from a JSX element's attributes.
///
/// Returns `Some("name")` if the element has `slot="name"`, otherwise `None`.
/// Does **not** handle dynamic `slot={expr}` — use [`get_slot_attribute_value`] for that.
pub(super) fn get_slot_attribute<'a>(attrs: &'a [JSXAttributeItem<'a>]) -> Option<&'a str> {
    for attr in attrs {
        if let JSXAttributeItem::Attribute(attr) = attr
            && let JSXAttributeName::Identifier(ident) = &attr.name
            && ident.name.as_str() == "slot"
            && let Some(JSXAttributeValue::StringLiteral(lit)) = &attr.value
        {
            return Some(lit.value.as_str());
        }
    }
    None
}

/// Extract the slot attribute value (static or dynamic) from a JSX element's attributes.
pub(super) fn get_slot_attribute_value(attrs: &[JSXAttributeItem<'_>]) -> Option<SlotValue> {
    for attr in attrs {
        if let JSXAttributeItem::Attribute(attr) = attr
            && let JSXAttributeName::Identifier(ident) = &attr.name
            && ident.name.as_str() == "slot"
        {
            match &attr.value {
                Some(JSXAttributeValue::StringLiteral(lit)) => {
                    return Some(SlotValue::Static(lit.value.to_string()));
                }
                Some(JSXAttributeValue::ExpressionContainer(expr)) => {
                    if let Some(e) = expr.expression.as_expression() {
                        return Some(SlotValue::Dynamic(expr_to_string(e), attr.span));
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Information about slots found within an expression container.
#[derive(Debug)]
pub(super) enum ExpressionSlotInfo<'a> {
    /// No slotted elements found — treat as default slot.
    None,
    /// Single static slot found — use that slot name for the entire expression.
    Single(&'a str),
    /// Single dynamic slot found — use computed `[expr]` key for the entire expression.
    SingleDynamic(String, oxc_span::Span),
    /// Multiple different slots found — requires `$$mergeSlots`.
    Multiple,
}

/// A collected slot entry — either a static name or a dynamic expression.
#[derive(Debug)]
enum CollectedSlot<'a> {
    Static(&'a str),
    Dynamic(String, oxc_span::Span),
}

/// Extract slot information from a JSX expression.
///
/// Recursively searches for JSX elements with `slot` attributes.
pub(super) fn extract_slots_from_expression<'a>(
    expr: &'a JSXExpression<'a>,
) -> ExpressionSlotInfo<'a> {
    let mut slots: Vec<CollectedSlot<'a>> = Vec::new();
    collect_slots_from_expression(expr, &mut slots);

    match slots.len() {
        0 => ExpressionSlotInfo::None,
        1 => match slots.remove(0) {
            CollectedSlot::Static(name) => ExpressionSlotInfo::Single(name),
            CollectedSlot::Dynamic(expr_str, span) => {
                ExpressionSlotInfo::SingleDynamic(expr_str, span)
            }
        },
        // ≥2 slotted elements route through `$$mergeSlots` so the runtime conditional
        // decides presence (`Astro.slots.has`) — keyed off count, not names, like Go.
        _ => ExpressionSlotInfo::Multiple,
    }
}

/// Recursively collect slot entries from a JSX expression.
fn collect_slots_from_expression<'a>(
    expr: &'a JSXExpression<'a>,
    slots: &mut Vec<CollectedSlot<'a>>,
) {
    match expr {
        JSXExpression::JSXElement(el) => {
            match get_slot_attribute_value(&el.opening_element.attributes) {
                Some(SlotValue::Static(name)) => {
                    if let Some(name_ref) = get_slot_attribute(&el.opening_element.attributes) {
                        slots.push(CollectedSlot::Static(name_ref));
                    } else {
                        drop(name);
                    }
                }
                Some(SlotValue::Dynamic(expr_str, span)) => {
                    slots.push(CollectedSlot::Dynamic(expr_str, span));
                }
                None => {}
            }
        }
        // A bare `<>` is opaque: its `slot=` children belong to the fragment, not the
        // parent, which receives the whole fragment as default content. Matches Go.
        JSXExpression::JSXFragment(_) => {}
        JSXExpression::ConditionalExpression(cond) => {
            collect_slots_from_inner_expression(&cond.consequent, slots);
            collect_slots_from_inner_expression(&cond.alternate, slots);
        }
        JSXExpression::LogicalExpression(logic) => {
            // For &&/|| expressions, the JSX is typically on the right side
            collect_slots_from_inner_expression(&logic.right, slots);
        }
        JSXExpression::ParenthesizedExpression(paren) => {
            collect_slots_from_inner_expression(&paren.expression, slots);
        }
        JSXExpression::ArrowFunctionExpression(arrow) => {
            if arrow.expression {
                // Expression-body arrow: `() => <span slot="c">C</span>`
                // The body has one ExpressionStatement wrapping the JSX.
                if let Some(oxc_ast::ast::Statement::ExpressionStatement(expr_stmt)) =
                    arrow.body.statements.first()
                {
                    collect_slots_from_inner_expression(&expr_stmt.expression, slots);
                }
            } else {
                // Block-body arrow: look inside return/switch/if statements
                collect_slots_from_function_body(&arrow.body, slots);
            }
        }
        JSXExpression::CallExpression(call) => {
            // e.g. items.map(item => <div slot="content">{item}</div>)
            // Walk into callback arguments to find slotted JSX.
            collect_slots_from_call_arguments(&call.arguments, slots);
        }
        _ => {
            if let Some(inner) = expr.as_expression() {
                collect_slots_from_inner_expression(inner, slots);
            }
        }
    }
}

/// Collect slot entries from an inner `Expression` (not `JSXExpression`).
fn collect_slots_from_inner_expression<'a>(
    expr: &'a Expression<'a>,
    slots: &mut Vec<CollectedSlot<'a>>,
) {
    match expr {
        Expression::JSXElement(el) => {
            match get_slot_attribute_value(&el.opening_element.attributes) {
                Some(SlotValue::Static(_)) => {
                    if let Some(name_ref) = get_slot_attribute(&el.opening_element.attributes) {
                        slots.push(CollectedSlot::Static(name_ref));
                    }
                }
                Some(SlotValue::Dynamic(expr_str, span)) => {
                    slots.push(CollectedSlot::Dynamic(expr_str, span));
                }
                None => {}
            }
        }
        // Bare `<>` is opaque — see `collect_slots_from_expression`.
        Expression::JSXFragment(_) => {}
        Expression::ConditionalExpression(cond) => {
            collect_slots_from_inner_expression(&cond.consequent, slots);
            collect_slots_from_inner_expression(&cond.alternate, slots);
        }
        Expression::LogicalExpression(logic) => {
            collect_slots_from_inner_expression(&logic.right, slots);
        }
        Expression::ParenthesizedExpression(paren) => {
            collect_slots_from_inner_expression(&paren.expression, slots);
        }
        Expression::ArrowFunctionExpression(arrow) => {
            if arrow.expression {
                if let Some(oxc_ast::ast::Statement::ExpressionStatement(expr_stmt)) =
                    arrow.body.statements.first()
                {
                    collect_slots_from_inner_expression(&expr_stmt.expression, slots);
                }
            } else {
                collect_slots_from_function_body(&arrow.body, slots);
            }
        }
        Expression::CallExpression(call) => {
            // e.g. items.map(item => <div slot="content">{item}</div>)
            // Walk into callback arguments to find slotted JSX.
            collect_slots_from_call_arguments(&call.arguments, slots);
        }
        Expression::ChainExpression(chain) => {
            // e.g. items?.map(item => <div slot="item">{item}</div>)
            // Unwrap the chain to reach the inner CallExpression.
            if let oxc_ast::ast::ChainElement::CallExpression(call) = &chain.expression {
                collect_slots_from_call_arguments(&call.arguments, slots);
            }
        }
        _ => {}
    }
}

/// Walk into the arguments of a call expression looking for arrow functions
/// or other expressions that may contain slotted JSX elements.
/// This handles patterns like `items.map(item => <div slot="name">...</div>)`.
fn collect_slots_from_call_arguments<'a>(
    arguments: &'a [oxc_ast::ast::Argument<'a>],
    slots: &mut Vec<CollectedSlot<'a>>,
) {
    for arg in arguments {
        match arg {
            oxc_ast::ast::Argument::ArrowFunctionExpression(arrow) => {
                if arrow.expression {
                    if let Some(oxc_ast::ast::Statement::ExpressionStatement(expr_stmt)) =
                        arrow.body.statements.first()
                    {
                        collect_slots_from_inner_expression(&expr_stmt.expression, slots);
                    }
                } else {
                    collect_slots_from_function_body(&arrow.body, slots);
                }
            }
            oxc_ast::ast::Argument::FunctionExpression(func) => {
                if let Some(body) = &func.body {
                    collect_slots_from_function_body(body, slots);
                }
            }
            _ => {
                // For other argument types, try to recurse as an expression
                if let Some(expr) = arg.as_expression() {
                    collect_slots_from_inner_expression(expr, slots);
                }
            }
        }
    }
}

/// Collect slot entries from return statements inside a function body.
fn collect_slots_from_function_body<'a>(
    body: &'a oxc_ast::ast::FunctionBody<'a>,
    slots: &mut Vec<CollectedSlot<'a>>,
) {
    for stmt in &body.statements {
        collect_slots_from_statement(stmt, slots);
    }
}

/// Recursively collect slot entries from statements (looking into return, switch, if, block).
fn collect_slots_from_statement<'a>(
    stmt: &'a oxc_ast::ast::Statement<'a>,
    slots: &mut Vec<CollectedSlot<'a>>,
) {
    use oxc_ast::ast::Statement;
    match stmt {
        Statement::ReturnStatement(ret) => {
            if let Some(arg) = &ret.argument {
                collect_slots_from_inner_expression(arg, slots);
            }
        }
        Statement::SwitchStatement(switch_stmt) => {
            for case in &switch_stmt.cases {
                for s in &case.consequent {
                    collect_slots_from_statement(s, slots);
                }
            }
        }
        Statement::BlockStatement(block) => {
            for s in &block.body {
                collect_slots_from_statement(s, slots);
            }
        }
        Statement::IfStatement(if_stmt) => {
            collect_slots_from_statement(&if_stmt.consequent, slots);
            if let Some(alt) = &if_stmt.alternate {
                collect_slots_from_statement(alt, slots);
            }
        }
        Statement::TryStatement(try_stmt) => {
            for s in &try_stmt.block.body {
                collect_slots_from_statement(s, slots);
            }
            if let Some(handler) = &try_stmt.handler {
                for s in &handler.body.body {
                    collect_slots_from_statement(s, slots);
                }
            }
            if let Some(finalizer) = &try_stmt.finalizer {
                for s in &finalizer.body {
                    collect_slots_from_statement(s, slots);
                }
            }
        }
        Statement::ForStatement(for_stmt) => {
            collect_slots_from_statement(&for_stmt.body, slots);
        }
        Statement::ForInStatement(for_in) => {
            collect_slots_from_statement(&for_in.body, slots);
        }
        Statement::ForOfStatement(for_of) => {
            collect_slots_from_statement(&for_of.body, slots);
        }
        Statement::WhileStatement(while_stmt) => {
            collect_slots_from_statement(&while_stmt.body, slots);
        }
        Statement::DoWhileStatement(do_while) => {
            collect_slots_from_statement(&do_while.body, slots);
        }
        Statement::LabeledStatement(labeled) => {
            collect_slots_from_statement(&labeled.body, slots);
        }
        _ => {}
    }
}

impl<'a> AstroCodegen<'a> {
    /// Check if a JSX child has meaningful content (not just whitespace or empty expressions).
    pub(super) fn jsx_child_has_content(child: &JSXChild<'a>) -> bool {
        match child {
            JSXChild::Text(text) => !text.value.trim().is_empty(),
            JSXChild::ExpressionContainer(expr) => {
                !matches!(expr.expression, JSXExpression::EmptyExpression(_))
            }
            JSXChild::Element(_)
            | JSXChild::Fragment(_)
            | JSXChild::Spread(_)
            | JSXChild::AstroComment(_) => true,
            JSXChild::AstroScript(_) | JSXChild::AstroDoctype(_) => false,
        }
    }

    /// Print the opening of a slot render function: ` async? () => $$render\``.
    ///
    /// This is the common suffix after a slot key. After calling this, the
    /// caller should print the slot body and then close with `` \` ``.
    fn print_slot_fn_open(&mut self, body_has_await: bool) {
        let async_prefix = Self::async_prefix(body_has_await);
        let slot_params = self.get_slot_params();
        self.print(async_prefix);
        self.print(slot_params);
        self.print(runtime::RENDER);
        self.print("`");
    }

    /// Emit slot body children. Under compact mode they route through the shared
    /// whitespace-collapsing path so slotted content is trimmed like regular
    /// template children; with compact disabled they are emitted verbatim, which
    /// is what the Go compiler does for slot whitespace.
    fn print_slot_children(&mut self, children: &[&JSXChild<'a>]) {
        if self.options.compact == CompactMode::Disabled {
            for child in children {
                self.print_jsx_child(child);
            }
        } else {
            self.print_jsx_children_compact_refs(children);
        }
    }

    /// Print all children as a single default slot, preserving slot attributes.
    /// Used for custom elements (web components) where the browser handles slots.
    pub(super) fn print_component_default_slot_only(&mut self, children: &[JSXChild<'a>]) {
        self.print("{\"default\": ");
        self.print_slot_fn_open(AwaitDetector::found_in_children(children));

        // DO NOT set skip_slot_attribute — we want to preserve slot="..." for custom elements
        let rendered: Vec<&JSXChild<'a>> = children
            .iter()
            .filter(|child| {
                !(self.options.strip_slot_comments && matches!(child, JSXChild::AstroComment(_)))
            })
            .collect();
        self.print_slot_children(&rendered);

        self.print("`,}");
    }

    /// Categorize children into named/default/dynamic/conditional slots and print them.
    pub(super) fn print_component_slots(&mut self, children: &[JSXChild<'a>]) {
        // Categorize children into:
        // 1. default_children — children without slot attribute
        // 2. named_slots — single static slot="name", whether a direct element or
        //    expression-wrapped (`{cond && <div slot="name">}`)
        // 3. conditional_slots — expressions with multiple different slots (need $$mergeSlots)
        let mut default_children: Vec<&JSXChild<'a>> = Vec::new();
        // Direct and expression-wrapped elements share this bucket, keyed by name, so a
        // plain `slot="x"` and a `{cond && <_ slot="x">}` sibling don't emit a duplicate
        // (last-wins) object key. Matches the Go compiler.
        let mut named_slots: Vec<(String, Vec<&JSXChild<'a>>)> = Vec::new();
        let mut conditional_slots: Vec<&JSXExpressionContainer<'a>> = Vec::new();

        // Direct elements with slot={expr} (e.g. <Comp slot={name} />)
        let mut dynamic_slots: Vec<(String, oxc_span::Span, Vec<&JSXChild<'a>>)> = Vec::new();
        // Expression containers whose single slotted element has a dynamic slot={expr}
        // e.g. {cond ? <Comp slot={cond ? "meta" : ""} /> : null}
        let mut dynamic_expression_slots: Vec<(String, oxc_span::Span, &JSXChild<'a>)> = Vec::new();

        for (i, child) in children.iter().enumerate() {
            // Skip HTML comments in slots if configured
            if self.options.strip_slot_comments && matches!(child, JSXChild::AstroComment(_)) {
                continue;
            }

            match child {
                JSXChild::Element(el) => {
                    // Check for slot attribute on direct element children
                    match get_slot_attribute_value(&el.opening_element.attributes) {
                        Some(SlotValue::Static(slot_name)) => {
                            // Static slot: slot="name"
                            if let Some((_, slot_children)) =
                                named_slots.iter_mut().find(|(name, _)| name == &slot_name)
                            {
                                slot_children.push(child);
                            } else {
                                named_slots.push((slot_name, vec![child]));
                            }
                        }
                        Some(SlotValue::Dynamic(expr, span)) => {
                            // Dynamic slot: slot={expr}
                            dynamic_slots.push((expr, span, vec![child]));
                        }
                        None => {
                            default_children.push(child);
                        }
                    }
                }
                JSXChild::ExpressionContainer(expr) => {
                    // Check if expression contains slotted elements
                    match extract_slots_from_expression(&expr.expression) {
                        ExpressionSlotInfo::None => {
                            default_children.push(child);
                        }
                        ExpressionSlotInfo::Single(slot_name) => {
                            // Group with direct-element slots of the same name so siblings
                            // don't emit a duplicate (last-wins) object key.
                            if let Some((_, slot_children)) =
                                named_slots.iter_mut().find(|(name, _)| name == slot_name)
                            {
                                slot_children.push(child);
                            } else {
                                named_slots.push((slot_name.to_string(), vec![child]));
                            }
                        }
                        ExpressionSlotInfo::SingleDynamic(expr_str, span) => {
                            // Expression containing a single element with a dynamic slot={expr}
                            // Use computed property syntax: [expr]: () => $$render`...`
                            dynamic_expression_slots.push((expr_str, span, child));
                        }
                        ExpressionSlotInfo::Multiple => {
                            conditional_slots.push(expr);
                        }
                    }
                }
                JSXChild::Text(text) => {
                    // A whitespace-only text node is dropped when it sits
                    // immediately adjacent to an expression container —
                    // specifically when it is:
                    //   • the first child and its next sibling is an expression, OR
                    //   • directly after an expression sibling.
                    // This matches the Go compiler's slot child filtering exactly
                    // and avoids passing leading/trailing whitespace through to
                    // framework slot renderers (e.g. Vue's <slot />).
                    if text.value.trim().is_empty() {
                        let prev_is_expr =
                            i > 0 && matches!(children[i - 1], JSXChild::ExpressionContainer(_));
                        let next_is_expr = i + 1 < children.len()
                            && matches!(children[i + 1], JSXChild::ExpressionContainer(_));
                        let is_first = i == 0;
                        // Drop if: leading node whose next sibling is an expression,
                        //       or: node immediately following an expression.
                        if (is_first && next_is_expr) || prev_is_expr {
                            continue;
                        }
                    }
                    default_children.push(child);
                }
                _ => {
                    default_children.push(child);
                }
            }
        }

        // Determine if we need $$mergeSlots wrapper
        let needs_merge_slots = !conditional_slots.is_empty();

        if needs_merge_slots {
            self.print(runtime::MERGE_SLOTS);
            self.print("(");
        }

        self.print("{");

        // Print default slot only if there are children with actual content
        let has_meaningful_content = default_children
            .iter()
            .any(|c| Self::jsx_child_has_content(c));
        if has_meaningful_content {
            self.print("\"default\": ");
            self.print_slot_fn_open(AwaitDetector::found_in_refs(&default_children));
            self.print_slot_children(&default_children);
            self.print("`,");
        }

        // Print named slots — direct and expression-wrapped elements sharing a name
        // render under one key, bodies concatenated, like the default slot's children.
        for (name, slot_children) in &named_slots {
            self.print("\"");
            self.print(&escape_double_quotes(name));
            self.print("\": ");
            self.print_slot_fn_open(AwaitDetector::found_in_refs(slot_children));
            // Skip slot attribute when printing these children
            let prev = self.skip_slot_attribute;
            self.skip_slot_attribute = true;
            self.print_slot_children(slot_children);
            self.skip_slot_attribute = prev;
            self.print("`,");
        }

        // Print dynamic slots (elements with slot={expr}) using computed property syntax
        for (expr, span, slot_children) in &dynamic_slots {
            self.add_source_mapping_for_span(*span);
            self.print("[");
            self.print(expr);
            self.print("]: ");
            self.print_slot_fn_open(AwaitDetector::found_in_refs(slot_children));
            let prev = self.skip_slot_attribute;
            self.skip_slot_attribute = true;
            self.print_slot_children(slot_children);
            self.skip_slot_attribute = prev;
            self.print("`,");
        }

        // Print dynamic expression slots — expressions that contain a single element with
        // a dynamic slot={expr}. Use a computed property key matching the Go compiler:
        //   [expr]: () => $$render`${cond ? $$render`${$$renderComponent(...)}` : null}`
        for (expr_str, span, child) in &dynamic_expression_slots {
            self.add_source_mapping_for_span(*span);
            self.print("[");
            self.print(expr_str);
            self.print("]: ");
            self.print_slot_fn_open(AwaitDetector::found_in_child(child));
            // Do NOT set skip_slot_attribute — Go compiler keeps the slot prop on the component
            self.print_jsx_child(child);
            self.print("`,");
        }

        self.print("}");

        // Print conditional slots (expressions with multiple slots) for $$mergeSlots
        for expr in &conditional_slots {
            self.print(",");
            self.print_conditional_slot_expression(expr);
        }

        if needs_merge_slots {
            self.print(")");
        }
    }

    /// Print an expression with multiple conditional slots for `$$mergeSlots`.
    ///
    /// Transforms: `cond ? <div slot="a"> : <div slot="b">`
    /// Into: `cond ? {"a": () => $$render`<div>`} : {"b": () => $$render`<div>`}`
    fn print_conditional_slot_expression(&mut self, expr: &JSXExpressionContainer<'a>) {
        self.print_conditional_slot_expr(&expr.expression);
    }

    fn print_conditional_slot_expr(&mut self, expr: &JSXExpression<'a>) {
        match expr {
            JSXExpression::ConditionalExpression(cond) => {
                self.add_source_mapping_for_span(cond.span);
                self.print_expression(&cond.test);
                self.print(" ? ");
                self.print_conditional_slot_branch(&cond.consequent);
                self.print(" : ");
                self.print_conditional_slot_branch(&cond.alternate);
            }
            JSXExpression::ArrowFunctionExpression(arrow) => {
                self.add_source_mapping_for_span(arrow.span);
                self.print_slot_aware_arrow_function(arrow);
            }
            _ => {
                if let Some(inner) = expr.as_expression() {
                    self.print_conditional_slot_branch_expr(inner);
                } else {
                    self.print_jsx_expression(expr);
                }
            }
        }
    }

    /// Print an expression that might contain conditional slot returns.
    fn print_conditional_slot_branch_expr(&mut self, expr: &Expression<'a>) {
        match expr {
            Expression::ArrowFunctionExpression(arrow) => {
                self.add_source_mapping_for_span(arrow.span);
                self.print_slot_aware_arrow_function(arrow);
            }
            Expression::ParenthesizedExpression(paren) => {
                self.print_conditional_slot_branch_expr(&paren.expression);
            }
            Expression::ConditionalExpression(cond) => {
                self.add_source_mapping_for_span(cond.span);
                self.print_expression(&cond.test);
                self.print(" ? ");
                self.print_conditional_slot_branch(&cond.consequent);
                self.print(" : ");
                self.print_conditional_slot_branch(&cond.alternate);
            }
            Expression::CallExpression(call) => {
                self.print_slot_aware_call_expression(call);
            }
            Expression::ChainExpression(chain) => {
                if let oxc_ast::ast::ChainElement::CallExpression(call) = &chain.expression {
                    self.print_slot_aware_call_expression(call);
                } else {
                    self.print_expression(expr);
                }
            }
            Expression::LogicalExpression(logic) => {
                self.print_logical_slot_branch(logic);
            }
            _ => {
                self.print_expression(expr);
            }
        }
    }

    /// Recurse into the right operand so its slotted elements get wrapped in slot
    /// objects, preserving parens so `a && (b ? X : Y)` does not re-bind as `(a && b) ? X : Y`.
    fn print_logical_slot_branch(&mut self, logic: &oxc_ast::ast::LogicalExpression<'a>) {
        self.add_source_mapping_for_span(logic.span);
        self.print_expression(&logic.left);
        self.print(match logic.operator {
            oxc_ast::ast::LogicalOperator::And => " && ",
            oxc_ast::ast::LogicalOperator::Or => " || ",
            oxc_ast::ast::LogicalOperator::Coalesce => " ?? ",
        });
        if let Expression::ParenthesizedExpression(paren) = &logic.right {
            self.print("(");
            self.print_conditional_slot_branch(&paren.expression);
            self.print(")");
        } else {
            self.print_conditional_slot_branch(&logic.right);
        }
    }

    /// Print a call expression where callback arguments may return slotted JSX.
    fn print_slot_aware_call_expression(&mut self, call: &oxc_ast::ast::CallExpression<'a>) {
        // Print callee (same as regular call expression printing)
        match &call.callee {
            Expression::StaticMemberExpression(member) => {
                self.print_expression(&member.object);
                self.print(if member.optional { "?." } else { "." });
                self.print(member.property.name.as_str());
            }
            Expression::ComputedMemberExpression(member) => {
                self.print_expression(&member.object);
                self.print(if member.optional { "?.[" } else { "[" });
                self.print_expression(&member.expression);
                self.print("]");
            }
            other => {
                self.print_expression(other);
            }
        }
        if call.optional {
            self.print("?.");
        }
        // Print arguments — route arrow/function args through slot-aware printing
        self.print("(");
        let mut first = true;
        for arg in &call.arguments {
            if !first {
                self.print(", ");
            }
            first = false;
            match arg {
                oxc_ast::ast::Argument::SpreadElement(spread) => {
                    self.print("...");
                    self.print_conditional_slot_branch_expr(&spread.argument);
                }
                _ => {
                    if let Some(expr) = arg.as_expression() {
                        self.print_conditional_slot_branch_expr(expr);
                    }
                }
            }
        }
        self.print(")");
    }

    /// Print an arrow function where return statements may contain slotted JSX.
    ///
    /// Transforms `return <div slot="a">A</div>` into
    /// `return {"a": () => $$render\`<div>A</div>\`}`
    fn print_slot_aware_arrow_function(
        &mut self,
        arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>,
    ) {
        self.add_source_mapping_for_span(arrow.span);
        self.print_arrow_params(arrow);

        if arrow.expression {
            // Expression body
            if let Some(expr) = arrow.body.statements.first()
                && let oxc_ast::ast::Statement::ExpressionStatement(expr_stmt) = expr
            {
                self.print_conditional_slot_branch(&expr_stmt.expression);
            }
        } else {
            // Block body with slot-aware statements
            self.print("{\n");
            for stmt in &arrow.body.statements {
                self.print_slot_aware_statement(stmt);
            }
            self.print("}");
        }
    }

    /// Print a statement where return values may contain slotted JSX.
    fn print_slot_aware_statement(&mut self, stmt: &oxc_ast::ast::Statement<'a>) {
        use oxc_ast::ast::Statement;
        match stmt {
            Statement::ReturnStatement(ret) => {
                self.add_source_mapping_for_span(ret.span);
                self.print("return ");
                if let Some(arg) = &ret.argument {
                    self.print_conditional_slot_branch(arg);
                }
                self.print(";\n");
            }
            Statement::SwitchStatement(switch_stmt) => {
                self.add_source_mapping_for_span(switch_stmt.span);
                self.print("switch (");
                self.print_expression(&switch_stmt.discriminant);
                self.print(") {\n");
                for case in &switch_stmt.cases {
                    self.add_source_mapping_for_span(case.span);
                    if let Some(test) = &case.test {
                        self.print("case ");
                        self.print_expression(test);
                        self.print(":");
                    } else {
                        self.print("default:");
                    }
                    for s in &case.consequent {
                        self.print_slot_aware_statement(s);
                    }
                    self.print("\n");
                }
                self.print("}");
            }
            Statement::BlockStatement(block) => {
                self.add_source_mapping_for_span(block.span);
                self.print("{\n");
                for s in &block.body {
                    self.print_slot_aware_statement(s);
                }
                self.print("}");
            }
            Statement::IfStatement(if_stmt) => {
                self.add_source_mapping_for_span(if_stmt.span);
                self.print("if (");
                self.print_expression(&if_stmt.test);
                self.print(") ");
                self.print_slot_aware_statement(&if_stmt.consequent);
                if let Some(alt) = &if_stmt.alternate {
                    self.print(" else ");
                    self.print_slot_aware_statement(alt);
                }
            }
            Statement::TryStatement(try_stmt) => {
                self.add_source_mapping_for_span(try_stmt.span);
                self.print("try {\n");
                for s in &try_stmt.block.body {
                    self.print_slot_aware_statement(s);
                }
                self.print("}");
                if let Some(handler) = &try_stmt.handler {
                    self.print(" catch");
                    if let Some(param) = &handler.param {
                        self.print("(");
                        let code = gen_to_string(&param.pattern);
                        self.print(&code);
                        self.print(")");
                    }
                    self.print(" {\n");
                    for s in &handler.body.body {
                        self.print_slot_aware_statement(s);
                    }
                    self.print("}");
                }
                if let Some(finalizer) = &try_stmt.finalizer {
                    self.print(" finally {\n");
                    for s in &finalizer.body {
                        self.print_slot_aware_statement(s);
                    }
                    self.print("}");
                }
                self.print("\n");
            }
            Statement::ForStatement(for_stmt) => {
                self.add_source_mapping_for_span(for_stmt.span);
                self.print("for(");
                if let Some(init) = &for_stmt.init {
                    let code = gen_to_string(init);
                    self.print(&code);
                }
                self.print(";");
                if let Some(test) = &for_stmt.test {
                    self.print_expression(test);
                }
                self.print(";");
                if let Some(update) = &for_stmt.update {
                    self.print_expression(update);
                }
                self.print(") ");
                self.print_slot_aware_statement(&for_stmt.body);
                self.print("\n");
            }
            Statement::ForInStatement(for_in) => {
                self.add_source_mapping_for_span(for_in.span);
                self.print("for(");
                match &for_in.left {
                    oxc_ast::ast::ForStatementLeft::VariableDeclaration(decl) => {
                        let code = gen_to_string(decl.as_ref());
                        self.print(&code);
                    }
                    other => {
                        let start = other.span().start as usize;
                        let end = other.span().end as usize;
                        if start < self.source_text.len() && end <= self.source_text.len() {
                            self.print(&self.source_text[start..end]);
                        }
                    }
                }
                self.print(" in ");
                self.print_expression(&for_in.right);
                self.print(") ");
                self.print_slot_aware_statement(&for_in.body);
                self.print("\n");
            }
            Statement::ForOfStatement(for_of) => {
                self.add_source_mapping_for_span(for_of.span);
                self.print(if for_of.r#await { "for await(" } else { "for(" });
                match &for_of.left {
                    oxc_ast::ast::ForStatementLeft::VariableDeclaration(decl) => {
                        let code = gen_to_string(decl.as_ref());
                        self.print(&code);
                    }
                    other => {
                        let start = other.span().start as usize;
                        let end = other.span().end as usize;
                        if start < self.source_text.len() && end <= self.source_text.len() {
                            self.print(&self.source_text[start..end]);
                        }
                    }
                }
                self.print(" of ");
                self.print_expression(&for_of.right);
                self.print(") ");
                self.print_slot_aware_statement(&for_of.body);
                self.print("\n");
            }
            Statement::WhileStatement(while_stmt) => {
                self.add_source_mapping_for_span(while_stmt.span);
                self.print("while (");
                self.print_expression(&while_stmt.test);
                self.print(") ");
                self.print_slot_aware_statement(&while_stmt.body);
                self.print("\n");
            }
            Statement::DoWhileStatement(do_while) => {
                self.add_source_mapping_for_span(do_while.span);
                self.print("do ");
                self.print_slot_aware_statement(&do_while.body);
                self.print(" while (");
                self.print_expression(&do_while.test);
                self.print(");\n");
            }
            Statement::LabeledStatement(labeled) => {
                self.add_source_mapping_for_span(labeled.span);
                self.print(&labeled.label.name);
                self.print(": ");
                self.print_slot_aware_statement(&labeled.body);
            }
            _ => {
                self.add_source_mapping_for_span(stmt.span());
                let code = gen_to_string(stmt);
                self.print(&code);
                self.print("\n");
            }
        }
    }

    pub(super) fn print_conditional_slot_branch(&mut self, expr: &Expression<'a>) {
        match expr {
            Expression::JSXElement(el) => {
                self.add_source_mapping_for_span(el.span);
                // Extract slot attribute — static or dynamic
                match get_slot_attribute_value(&el.opening_element.attributes) {
                    Some(SlotValue::Static(slot_name)) => {
                        self.print("{\"");
                        self.print(&escape_double_quotes(&slot_name));
                        self.print("\": ");
                        self.print_slot_fn_open(AwaitDetector::found_in_element(el));
                        let prev = self.skip_slot_attribute;
                        self.skip_slot_attribute = true;
                        self.print_jsx_element(el);
                        self.skip_slot_attribute = prev;
                        self.print("`}");
                    }
                    Some(SlotValue::Dynamic(expr_str, span)) => {
                        // Dynamic slot: use computed property key [expr]
                        self.add_source_mapping_for_span(span);
                        self.print("{[");
                        self.print(&expr_str);
                        self.print("]: ");
                        self.print_slot_fn_open(AwaitDetector::found_in_element(el));
                        // Keep the slot prop on the component (matches Go compiler behavior)
                        self.print_jsx_element(el);
                        self.print("`}");
                    }
                    None => {
                        // No slot attribute — print as default
                        self.print("{\"default\": ");
                        self.print_slot_fn_open(AwaitDetector::found_in_element(el));
                        self.print_jsx_element(el);
                        self.print("`}");
                    }
                }
            }
            Expression::ParenthesizedExpression(paren) => {
                self.print_conditional_slot_branch(&paren.expression);
            }
            Expression::ConditionalExpression(cond) => {
                // Nested ternary
                self.add_source_mapping_for_span(cond.span);
                self.print_expression(&cond.test);
                self.print(" ? ");
                self.print_conditional_slot_branch(&cond.consequent);
                self.print(" : ");
                self.print_conditional_slot_branch(&cond.alternate);
            }
            Expression::LogicalExpression(logic) => {
                self.print_logical_slot_branch(logic);
            }
            Expression::CallExpression(_)
            | Expression::ChainExpression(_)
            | Expression::ArrowFunctionExpression(_) => {
                self.print_conditional_slot_branch_expr(expr);
            }
            _ => {
                // Other expression types — use default codegen
                self.print_expression(expr);
            }
        }
    }
}
