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
use oxc_ast_visit::Visit;
use oxc_span::GetSpan;
use oxc_syntax::scope::ScopeFlags;
use rustc_hash::FxHashSet;

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
    /// A `.map()`-style call whose callback yields elements with a slot name
    /// derived from the callback's own parameters (e.g. `slot={`item-${i}`}`).
    /// The name can't be hoisted to a computed key — it must be built per
    /// iteration and merged via `$$mergeSlots(...call.map(...))`.
    Mapped,
    /// Multiple different slots found — requires `$$mergeSlots`.
    Multiple,
}

/// A collected slot entry — either a static name or a dynamic expression.
///
/// `Dynamic` keeps a reference to the slot value's expression so callers can
/// inspect which identifiers it references (see [`ExpressionSlotInfo::Mapped`]).
#[derive(Debug)]
enum CollectedSlot<'a> {
    Static(&'a str),
    Dynamic(String, oxc_span::Span, &'a Expression<'a>),
}

/// Extract slot information from a JSX expression.
///
/// Recursively searches for JSX elements with `slot` attributes.
pub(super) fn extract_slots_from_expression<'a>(
    expr: &'a JSXExpression<'a>,
) -> ExpressionSlotInfo<'a> {
    // A slot name computed from a callback binding (`(_, i) => slot={`x-${i}`}`)
    // can't be hoisted to a `[expr]` object key — the binding is out of scope
    // there. Route the whole expression through the runtime collector, which
    // merges the per-iteration slot objects whatever shape it evaluates to
    // (`.map`, `.flatMap`, `Array.from`, a helper, nested, async, …). No method
    // name is assumed; non-array results just contribute nothing.
    if dynamic_slot_references_local_binding(expr) {
        return ExpressionSlotInfo::Mapped;
    }

    // An array literal of slotted elements (`{[<a slot="a"/>, <b slot="b"/>]}`)
    // also goes through the collector, so each element's slot object is merged
    // rather than `Object.assign`ed by array index.
    if is_array_slot_expression(expr) {
        return ExpressionSlotInfo::Mapped;
    }

    let mut slots: Vec<CollectedSlot<'a>> = Vec::new();
    collect_slots_from_expression(expr, &mut slots);

    match slots.len() {
        0 => ExpressionSlotInfo::None,
        1 => match slots.remove(0) {
            CollectedSlot::Static(name) => ExpressionSlotInfo::Single(name),
            CollectedSlot::Dynamic(expr_str, span, _) => {
                ExpressionSlotInfo::SingleDynamic(expr_str, span)
            }
        },
        // ≥2 slotted elements route through `$$mergeSlots` so the runtime conditional
        // decides presence (`Astro.slots.has`) — keyed off count, not names, like Go.
        _ => ExpressionSlotInfo::Multiple,
    }
}

/// Whether `expr` is an array literal (ignoring wrapping parens) with slotted
/// elements. Its per-element slot objects go through the runtime collector.
fn is_array_slot_expression(expr: &JSXExpression<'_>) -> bool {
    let Some(inner) = expr.as_expression() else {
        return false;
    };
    matches!(inner.get_inner_expression(), Expression::ArrayExpression(_))
        && expression_has_slots(inner)
}

/// Whether any dynamic slot in `expr` derives its name from a binding local to
/// `expr` — a callback parameter or local. Such a name is out of scope wherever
/// the slot object is built, so it can't be hoisted to a `[expr]` key without a
/// `ReferenceError`; the caller either merges per iteration (`.map()`) or renders
/// the content as-is.
fn dynamic_slot_references_local_binding(expr: &JSXExpression<'_>) -> bool {
    let Some(inner) = expr.as_expression() else {
        return false;
    };
    let mut bound = BoundNameCollector::default();
    bound.visit_expression(inner);
    if bound.names.is_empty() {
        return false;
    }
    let mut slots = Vec::new();
    collect_slots_from_inner_expression(inner, &mut slots);
    slots.iter().any(|slot| match slot {
        CollectedSlot::Dynamic(_, _, expr) => expression_references_any(expr, &bound.names),
        CollectedSlot::Static(_) => false,
    })
}

/// Whether any callback anywhere in the mapped expression is `async`. This is an
/// over-approximation — it also sees `async` used inside slot *content* — which
/// is safe: an extra `Promise.all`/drain is a no-op on already-resolved values.
fn mapped_has_async_callback(expr: &JSXExpression<'_>) -> bool {
    let Some(inner) = expr.as_expression() else {
        return false;
    };
    let mut detector = AsyncCallbackDetector { found: false };
    detector.visit_expression(inner);
    detector.found
}

/// Finds any `async` arrow or function anywhere within an expression.
struct AsyncCallbackDetector {
    found: bool,
}

impl<'a> Visit<'a> for AsyncCallbackDetector {
    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        self.found |= arrow.r#async;
        oxc_ast_visit::walk::walk_arrow_function_expression(self, arrow);
    }

    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        self.found |= func.r#async;
        oxc_ast_visit::walk::walk_function(self, func, flags);
    }
}

/// Collects the names bound by binding patterns (params, destructuring targets).
#[derive(Default)]
struct BoundNameCollector {
    names: FxHashSet<String>,
}

impl<'a> Visit<'a> for BoundNameCollector {
    fn visit_binding_identifier(&mut self, ident: &BindingIdentifier<'a>) {
        self.names.insert(ident.name.to_string());
    }
}

/// Checks whether an expression references any identifier in `names`.
struct ReferenceChecker<'n> {
    names: &'n FxHashSet<String>,
    found: bool,
}

impl<'a> Visit<'a> for ReferenceChecker<'_> {
    fn visit_identifier_reference(&mut self, ident: &IdentifierReference<'a>) {
        if !self.found && self.names.contains(ident.name.as_str()) {
            self.found = true;
        }
    }
}

fn expression_references_any(expr: &Expression<'_>, names: &FxHashSet<String>) -> bool {
    let mut checker = ReferenceChecker {
        names,
        found: false,
    };
    checker.visit_expression(expr);
    checker.found
}

fn collect_slot_from_attributes<'a>(
    attrs: &'a [JSXAttributeItem<'a>],
    slots: &mut Vec<CollectedSlot<'a>>,
) {
    for attr in attrs {
        if let JSXAttributeItem::Attribute(attr) = attr
            && let JSXAttributeName::Identifier(ident) = &attr.name
            && ident.name.as_str() == "slot"
        {
            match &attr.value {
                Some(JSXAttributeValue::StringLiteral(lit)) => {
                    slots.push(CollectedSlot::Static(lit.value.as_str()));
                    return;
                }
                Some(JSXAttributeValue::ExpressionContainer(container)) => {
                    if let Some(expr) = container.expression.as_expression() {
                        slots.push(CollectedSlot::Dynamic(
                            expr_to_string(expr),
                            attr.span,
                            expr,
                        ));
                        return;
                    }
                }
                _ => {}
            }
        }
    }
}

fn collect_slots_from_arrow<'a>(
    arrow: &'a oxc_ast::ast::ArrowFunctionExpression<'a>,
    slots: &mut Vec<CollectedSlot<'a>>,
) {
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

/// Recursively collect slot entries from a JSX expression.
fn collect_slots_from_expression<'a>(
    expr: &'a JSXExpression<'a>,
    slots: &mut Vec<CollectedSlot<'a>>,
) {
    match expr {
        JSXExpression::JSXElement(el) => {
            collect_slot_from_attributes(&el.opening_element.attributes, slots);
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
            collect_slots_from_arrow(arrow, slots);
        }
        JSXExpression::CallExpression(call) => {
            collect_slots_from_call(call, slots);
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
            collect_slot_from_attributes(&el.opening_element.attributes, slots);
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
            collect_slots_from_arrow(arrow, slots);
        }
        Expression::CallExpression(call) => {
            collect_slots_from_call(call, slots);
        }
        Expression::ChainExpression(chain) => {
            // e.g. items?.map(item => <div slot="item">{item}</div>)
            // Unwrap the chain to reach the inner CallExpression.
            if let oxc_ast::ast::ChainElement::CallExpression(call) = &chain.expression {
                collect_slots_from_call(call, slots);
            }
        }
        // e.g. items.map(item => [<a slot={`a-${item}`}/>, <b slot={`b-${item}`}/>])
        Expression::ArrayExpression(arr) => {
            collect_slots_from_array(arr, slots);
        }
        _ => {}
    }
}

/// Collect slots from a call: its callback arguments plus the callee's receiver,
/// so a slot-producing call behind another method (`items.map(cb).filter(…)`) is
/// still found — the producing call is the *object* of the outer call, not an
/// argument. No method name is assumed; the runtime collector sorts out whatever
/// the chain evaluates to.
fn collect_slots_from_call<'a>(call: &'a CallExpression<'a>, slots: &mut Vec<CollectedSlot<'a>>) {
    collect_slots_from_call_arguments(&call.arguments, slots);
    if let Some(member) = call.callee.as_member_expression() {
        collect_slots_from_inner_expression(member.object(), slots);
    }
}

/// Collect slots from the elements of an array literal.
fn collect_slots_from_array<'a>(arr: &'a ArrayExpression<'a>, slots: &mut Vec<CollectedSlot<'a>>) {
    for element in &arr.elements {
        match element {
            ArrayExpressionElement::SpreadElement(spread) => {
                collect_slots_from_inner_expression(&spread.argument, slots);
            }
            ArrayExpressionElement::Elision(_) => {}
            _ => {
                if let Some(expr) = element.as_expression() {
                    collect_slots_from_inner_expression(expr, slots);
                }
            }
        }
    }
}

/// Whether `expr` contains any slotted JSX.
fn expression_has_slots(expr: &Expression<'_>) -> bool {
    let mut slots = Vec::new();
    collect_slots_from_inner_expression(expr, &mut slots);
    !slots.is_empty()
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
                collect_slots_from_arrow(arrow, slots);
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
        // `.map()`-style expressions whose slot names derive from the callback's
        // parameters; merged per iteration via `$$mergeSlots(...call.map(...))`.
        let mut mapped_dynamic_slots: Vec<&JSXExpressionContainer<'a>> = Vec::new();

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
                        ExpressionSlotInfo::Mapped => {
                            mapped_dynamic_slots.push(expr);
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
        let needs_merge_slots = !conditional_slots.is_empty() || !mapped_dynamic_slots.is_empty();

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
            // The name now lives in the object key, so strip the redundant
            // `slot={…}` attribute from the element (like Go, and the static case).
            let prev = self.skip_slot_attribute;
            self.skip_slot_attribute = true;
            self.print_jsx_child(child);
            self.skip_slot_attribute = prev;
            self.print("`,");
        }

        self.print("}");

        // Print conditional slots (expressions with multiple slots) for $$mergeSlots
        for expr in &conditional_slots {
            self.print(",");
            self.print_conditional_slot_expression(expr);
        }

        // Print callback-bound dynamic slots: a runtime collector merges each
        // expression's per-iteration slot objects into one object for `$$mergeSlots`.
        for expr in &mapped_dynamic_slots {
            self.print(",");
            self.print_mapped_dynamic_slot(expr);
        }

        if needs_merge_slots {
            self.print(")");
        }
    }

    /// Print an expression whose slot names come from callback bindings, wrapped
    /// in a runtime collector that merges its per-iteration slot objects.
    ///
    /// The callback's slotted JSX is transformed into slot objects
    /// (`{[`x${i}`]: () => $$render`…`}`); the collector then walks whatever the
    /// expression evaluates to — flattening arrays at any depth, awaiting promises
    /// at any level, keeping slot objects, and ignoring everything else. That way
    /// it works for `.map`, `.flatMap`, `Array.from`, a custom helper, nesting,
    /// async — without assuming any method name; `.forEach` (`undefined`) or
    /// `.join` (a string) simply contribute no slots instead of throwing.
    fn print_mapped_dynamic_slot(&mut self, expr: &JSXExpressionContainer<'a>) {
        self.add_source_mapping_for_span(expr.span);
        let prev_wrap = self.wrap_arrow_slot_object;
        let prev_skip = self.skip_slot_attribute;
        self.wrap_arrow_slot_object = true;
        // The slot name now lives in the object key, so strip the redundant
        // `slot={…}` attribute from the element — matching the static `.map()` case.
        self.skip_slot_attribute = true;

        // An async callback yields promises the collector must await; the sync
        // collector is used otherwise (an `async` with no surviving `await` is
        // dropped as inert, so it stays on the sync path). Redundant parens are
        // stripped by the later TS-strip pass.
        let has_async = self.scan_result.has_await && mapped_has_async_callback(&expr.expression);

        let (prefix, suffix): (&str, &str) = if has_async {
            (
                "await (async function $$collectSlots($$v) { \
                 $$v = await $$v; \
                 return Array.isArray($$v) \
                 ? Object.assign({}, ...await Promise.all($$v.map($$collectSlots))) \
                 : $$v && typeof $$v === \"object\" ? $$v : {}; })(",
                ")",
            )
        } else {
            (
                "(function $$collectSlots($$v) { \
                 return Array.isArray($$v) \
                 ? $$v.reduce((a, x) => Object.assign(a, $$collectSlots(x)), {}) \
                 : $$v && typeof $$v === \"object\" ? $$v : {}; })(",
                ")",
            )
        };

        self.print(prefix);
        if let Some(inner) = expr.expression.as_expression() {
            self.print_conditional_slot_branch_expr(inner);
        }
        self.print(suffix);

        self.wrap_arrow_slot_object = prev_wrap;
        self.skip_slot_attribute = prev_skip;
    }

    /// Print an expression with multiple conditional slots for `$$mergeSlots`.
    ///
    /// Transforms: `cond ? <div slot="a"> : <div slot="b">`
    /// Into: `cond ? {"a": () => $$render`<div>`} : {"b": () => $$render`<div>`}`
    fn print_conditional_slot_expression(&mut self, expr: &JSXExpressionContainer<'a>) {
        self.print_conditional_slot_expr(&expr.expression);
    }

    fn print_conditional_slot_ternary(&mut self, cond: &oxc_ast::ast::ConditionalExpression<'a>) {
        self.add_source_mapping_for_span(cond.span);
        self.print_expression(&cond.test);
        self.print(" ? ");
        self.print_conditional_slot_branch(&cond.consequent);
        self.print(" : ");
        self.print_conditional_slot_branch(&cond.alternate);
    }

    fn print_conditional_slot_expr(&mut self, expr: &JSXExpression<'a>) {
        match expr {
            JSXExpression::ConditionalExpression(cond) => {
                self.print_conditional_slot_ternary(cond);
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
            Expression::FunctionExpression(func) => {
                self.print_slot_aware_function_expression(func);
            }
            Expression::ParenthesizedExpression(paren) => {
                self.print_conditional_slot_branch_expr(&paren.expression);
            }
            Expression::ConditionalExpression(cond) => {
                self.print_conditional_slot_ternary(cond);
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
            Expression::ArrayExpression(arr) => {
                self.print_slot_aware_array(arr);
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

    /// Print a call's callee, transforming a slot-producing receiver so a `.map()`
    /// behind another method (`items.map(cb).filter(…)`) has its elements turned
    /// into slot objects too. Purely structural — no method name is inspected.
    /// The receiver is parenthesised (the TS-strip pass drops redundant parens),
    /// so precedence-sensitive receivers like `(await x).map(…)` stay correct.
    fn print_slot_aware_callee(&mut self, callee: &Expression<'a>) {
        match callee {
            Expression::StaticMemberExpression(member) if expression_has_slots(&member.object) => {
                self.print("(");
                self.print_conditional_slot_branch_expr(&member.object);
                self.print(")");
                self.print(if member.optional { "?." } else { "." });
                self.print(member.property.name.as_str());
            }
            Expression::ComputedMemberExpression(member)
                if expression_has_slots(&member.object) =>
            {
                self.print("(");
                self.print_conditional_slot_branch_expr(&member.object);
                self.print(")");
                self.print(if member.optional { "?.[" } else { "[" });
                self.print_expression(&member.expression);
                self.print("]");
            }
            _ => self.print_callee(callee),
        }
    }

    /// Print an array literal, transforming each slotted element into a slot
    /// object (`[<a slot={x}/>, <b slot={y}/>]` → `[{[x]: …}, {[y]: …}]`). The
    /// runtime collector flattens the result, so nested arrays merge correctly.
    fn print_slot_aware_array(&mut self, arr: &oxc_ast::ast::ArrayExpression<'a>) {
        self.print("[");
        let mut first = true;
        for element in &arr.elements {
            if !first {
                self.print(", ");
            }
            first = false;
            match element {
                oxc_ast::ast::ArrayExpressionElement::SpreadElement(spread) => {
                    self.print("...");
                    self.print_conditional_slot_branch(&spread.argument);
                }
                oxc_ast::ast::ArrayExpressionElement::Elision(_) => {}
                _ => {
                    if let Some(expr) = element.as_expression() {
                        self.print_conditional_slot_branch(expr);
                    }
                }
            }
        }
        self.print("]");
    }

    /// Print a call expression where callback arguments may return slotted JSX.
    fn print_slot_aware_call_expression(&mut self, call: &oxc_ast::ast::CallExpression<'a>) {
        self.print_slot_aware_callee(&call.callee);
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
        // A mapped async callback drops its `async` when the factory is sync:
        // there's no `await` to relocate, so the keyword is inert and keeping it
        // would make `.map()` return promises that `$$mergeSlots` can't merge.
        let inert_async = self.wrap_arrow_slot_object && !self.scan_result.has_await;
        self.print_arrow_params_with_async(arrow, arrow.r#async && !inert_async);

        if arrow.expression {
            // Expression body
            if let Some(expr) = arrow.body.statements.first()
                && let oxc_ast::ast::Statement::ExpressionStatement(expr_stmt) = expr
            {
                // In a concise body, a returned slot object literal must be
                // parenthesised so its leading `{` isn't read as a block.
                let wrap = self.wrap_arrow_slot_object;
                if wrap {
                    self.print("(");
                }
                self.print_conditional_slot_branch(&expr_stmt.expression);
                if wrap {
                    self.print(")");
                }
            }
        } else {
            // Keep `wrap_arrow_slot_object` as-is: a nested map's inner arrow
            // (`g => g.items.map(i => ({…}))`) still needs to parenthesise its
            // object body even when reached through this block's `return`.
            self.print("{\n");
            for stmt in &arrow.body.statements {
                self.print_slot_aware_statement(stmt);
            }
            self.print("}");
        }
    }

    /// Print a `function` expression callback whose `return`s may contain
    /// slotted JSX (`items.map(function (item) { return <div slot={…}> })`),
    /// mirroring the block-body handling of [`Self::print_slot_aware_arrow_function`].
    fn print_slot_aware_function_expression(&mut self, func: &oxc_ast::ast::Function<'a>) {
        self.add_source_mapping_for_span(func.span);
        // Drop an inert `async` for the same reason as a mapped arrow callback.
        let inert_async = self.wrap_arrow_slot_object && !self.scan_result.has_await;
        if func.r#async && !inert_async {
            self.print("async ");
        }
        self.print("function");
        if func.generator {
            self.print("*");
        }
        if let Some(id) = &func.id {
            self.print(" ");
            self.print(id.name.as_str());
        }
        self.print("(");
        self.print_formal_parameters(&func.params);
        self.print(") ");
        if let Some(body) = &func.body {
            self.print("{\n");
            for stmt in &body.statements {
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
                    self.print_catch_param(handler);
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
                self.print_for_statement_left(&for_in.left);
                self.print(" in ");
                self.print_expression(&for_in.right);
                self.print(") ");
                self.print_slot_aware_statement(&for_in.body);
                self.print("\n");
            }
            Statement::ForOfStatement(for_of) => {
                self.add_source_mapping_for_span(for_of.span);
                self.print(if for_of.r#await { "for await(" } else { "for(" });
                self.print_for_statement_left(&for_of.left);
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
                        // Name lives in the object key now — strip the redundant
                        // `slot={…}` attribute, like the static branch above and every
                        // other regular-component case. (Custom elements and root-level
                        // slots keep theirs via separate paths, for browser slotting;
                        // Go leaves it on here too, but that's an inconsistency with its
                        // own single-slot/static handling — a sanctioned divergence.)
                        let prev = self.skip_slot_attribute;
                        self.skip_slot_attribute = true;
                        self.print_jsx_element(el);
                        self.skip_slot_attribute = prev;
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
                self.print_conditional_slot_ternary(cond);
            }
            Expression::LogicalExpression(logic) => {
                self.print_logical_slot_branch(logic);
            }
            Expression::ArrayExpression(arr) => {
                self.print_slot_aware_array(arr);
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
