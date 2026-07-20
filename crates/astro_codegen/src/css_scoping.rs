//! CSS scoping for Astro components.
//!
//! Rewrites selectors in `<style>` blocks to carry a scope identifier, matching the Go
//! compiler's behavior. lightningcss parses the stylesheet; scoping runs as a visitor pass.
//!
//! `:global()` needs special handling: with CSS modules mode off it parses as an opaque
//! `CustomFunction`, so a pre-pass re-parses its argument into a `PseudoClass::Global` and
//! hoists any leading combinator (`:global(> *)`) out before the scope pass.

use std::convert::Infallible;

use lightningcss::printer::{Printer, PrinterOptions};
use lightningcss::properties::custom::{TokenList, TokenOrValue};
use lightningcss::selector::{Combinator, Component, PseudoClass, Selector, SelectorList};
use lightningcss::stylesheet::{ParserFlags, ParserOptions, StyleSheet};
use lightningcss::traits::{IntoOwned, ParseWithOptions, ToCss};
use lightningcss::values::ident::Ident;
use lightningcss::values::string::CowArcStr;
use lightningcss::visit_types;
use lightningcss::visitor::{Visit, VisitTypes, Visitor};

use crate::ScopedStyleStrategy;

fn parser_options<'a>() -> ParserOptions<'a> {
    ParserOptions {
        flags: ParserFlags::NESTING,
        error_recovery: true,
        // Parse `:global()` as `CustomFunction` so arguments (including leading
        // combinators) are preserved in the AST. We normalize to `Global` ourselves.
        css_modules: None,
        ..ParserOptions::default()
    }
}

/// Normalize `:global()` custom functions and hoist leading combinators inside them.
struct GlobalSelectorVisitor;

impl<'i> Visitor<'i> for GlobalSelectorVisitor {
    type Error = Infallible;

    fn visit_types(&self) -> VisitTypes {
        visit_types!(SELECTORS)
    }

    fn visit_selector_list(&mut self, selectors: &mut SelectorList<'i>) -> Result<(), Self::Error> {
        // Rebuild the rule's top-level selector list, expanding any `:global()` that wraps a
        // selector list into one selector per item (`.x :global(ul, ol)` becomes the two
        // selectors `.x :global(ul)` and `.x :global(ol)`). Arguments of `:not()`/`:is()`/
        // `:has()` are not traversed, so a nested `:global(...)` stays opaque and is emitted
        // verbatim.
        let mut expanded: Vec<Selector<'i>> = Vec::with_capacity(selectors.0.len());
        for selector in selectors.0.iter() {
            // Both passes below flatten and rebuild the whole selector, so skip them unless
            // there is actually a `:global()` to handle — most selectors have none.
            if !selector_contains_global(selector) {
                expanded.push(selector.clone());
                continue;
            }
            for mut normalized in expand_global_selector_lists(selector) {
                hoist_global_leading_combinators(&mut normalized);
                expanded.push(normalized);
            }
        }
        selectors.0 = expanded.into();
        Ok(())
    }
}

/// Normalize `:global()` components in a single selector and distribute any that wrap a
/// selector list, returning one selector per combination of list items.
///
/// A selector normally yields a single result; only a `:global()` whose argument is a
/// comma-separated list (`:global(ul, ol)`) produces more than one, and multiple such
/// globals in the same selector expand as a cartesian product (`:global(a, b) :global(c, d)`
/// → `a c`, `a d`, `b c`, `b d`).
fn expand_global_selector_lists<'i>(selector: &Selector<'i>) -> Vec<Selector<'i>> {
    let flat = flatten_selector_parse_order(selector);
    let per_component: Vec<Vec<Component<'i>>> =
        flat.into_iter().map(normalize_component_options).collect();

    let mut combos: Vec<Vec<Component<'i>>> = vec![Vec::new()];
    for options in per_component {
        let mut next = Vec::with_capacity(combos.len() * options.len());
        for prefix in &combos {
            for option in &options {
                let mut combo = prefix.clone();
                combo.push(option.clone());
                next.push(combo);
            }
        }
        combos = next;
    }

    combos.into_iter().map(Selector::from).collect()
}

/// Expand a single component into its normalized options.
///
/// Every component maps to exactly one option except a `:global()` list, which maps to one
/// `Global` per list item (`:global(ul, ol)` → `[Global(ul), Global(ol)]`).
fn normalize_component_options<'i>(component: Component<'i>) -> Vec<Component<'i>> {
    let Component::NonTSPseudoClass(PseudoClass::CustomFunction { name, arguments }) = &component
    else {
        return vec![component];
    };
    if !name.eq_ignore_ascii_case("global") {
        return vec![component];
    }

    if let Some(items) = parse_global_argument_list(arguments) {
        return items
            .into_iter()
            .map(|selector| {
                Component::NonTSPseudoClass(PseudoClass::Global {
                    selector: Box::new(selector),
                })
            })
            .collect();
    }

    // Empty or unparseable argument; leave it to the single-selector path, which keeps an
    // empty `:global()` opaque.
    vec![normalize_component(component)]
}

/// Cheap, allocation-free check for a top-level `:global()` — the raw `CustomFunction`
/// form or an already-normalized `Global`.
fn selector_contains_global(selector: &Selector<'_>) -> bool {
    selector
        .iter_raw_match_order()
        .any(|component| match component {
            Component::NonTSPseudoClass(PseudoClass::CustomFunction { name, .. }) => {
                name.eq_ignore_ascii_case("global")
            }
            Component::NonTSPseudoClass(PseudoClass::Global { .. }) => true,
            _ => false,
        })
}

fn normalize_global_pseudos(selector: &mut Selector<'_>) {
    let flat = flatten_selector_parse_order(selector);
    let normalized: Vec<Component<'_>> = flat.into_iter().map(normalize_component).collect();
    *selector = normalized.into();
}

fn normalize_component(component: Component<'_>) -> Component<'_> {
    match component {
        Component::NonTSPseudoClass(PseudoClass::CustomFunction { name, arguments })
            if name.eq_ignore_ascii_case("global") =>
        {
            let mut inner = parse_global_argument(&arguments);
            if inner.len() == 0 {
                Component::NonTSPseudoClass(PseudoClass::CustomFunction { name, arguments })
            } else {
                normalize_global_pseudos(&mut inner);
                Component::NonTSPseudoClass(PseudoClass::Global {
                    selector: Box::new(inner),
                })
            }
        }
        other => other,
    }
}

fn empty_selector<'i>() -> Selector<'i> {
    Selector::from(Vec::<Component<'i>>::new())
}

/// Parse a `:global(...)` argument into a `Selector` from its preserved `CustomFunction`
/// tokens. `into_owned()` detaches the result from the temporary serialized string.
fn parse_global_argument<'i>(tokens: &TokenList<'_>) -> Selector<'i> {
    let serialized = token_list_to_css(tokens);
    let argument_text = serialized.trim();
    if argument_text.is_empty() {
        return empty_selector();
    }
    let options = parser_options();

    if let Some((combinator, rest)) = split_leading_combinator_str(argument_text) {
        let Ok(rest_selector) = Selector::parse_string_with_options(rest.trim(), options) else {
            return empty_selector();
        };
        let mut flat = vec![Component::Combinator(combinator)];
        flat.extend(flatten_selector_parse_order(&rest_selector));
        return Selector::from(flat).into_owned();
    }

    match Selector::parse_string_with_options(argument_text, options) {
        Ok(selector) => selector.into_owned(),
        Err(_) => empty_selector(),
    }
}

/// Parse a `:global(...)` argument as a selector list, so a list can be distributed across
/// the enclosing selector. Returns `None` when the argument is empty or every item fails to
/// parse.
fn parse_global_argument_list<'i>(tokens: &TokenList<'_>) -> Option<Vec<Selector<'i>>> {
    let serialized = token_list_to_css(tokens);
    let argument_text = serialized.trim();
    if argument_text.is_empty() {
        return None;
    }
    let options = parser_options();

    // Common case: a comma-separated list with no leading combinators parses directly.
    // lightningcss handles commas nested inside attribute values or `:is(a, b)`.
    if let Ok(list) = SelectorList::parse_string_with_options(argument_text, options)
        && !list.0.is_empty()
    {
        return Some(
            list.0
                .into_iter()
                .map(|selector| {
                    // Detach from the temporary serialized string, then normalize any nested
                    // `:global()` inside the item.
                    let mut selector = selector.into_owned();
                    normalize_global_pseudos(&mut selector);
                    selector
                })
                .collect(),
        );
    }

    // A leading combinator (`:global(> *)`, `:global(> .a, > .b)`) can't be parsed as a list
    // by lightningcss, so split the top-level commas manually and parse each item with its
    // own leading combinator hoisted in.
    let items: Vec<Selector<'i>> = split_top_level_commas(argument_text)
        .into_iter()
        .filter_map(parse_global_argument_item)
        .collect();
    if items.is_empty() { None } else { Some(items) }
}

/// Parse one item of a `:global(...)` argument, hoisting a leading combinator (`> .a`) into
/// the selector. Returns `None` if the item is empty or fails to parse.
fn parse_global_argument_item<'i>(item_text: &str) -> Option<Selector<'i>> {
    let item_text = item_text.trim();
    if item_text.is_empty() {
        return None;
    }
    let options = parser_options();

    let mut selector = if let Some((combinator, rest)) = split_leading_combinator_str(item_text) {
        let rest = rest.trim();
        if rest.is_empty() {
            return None;
        }
        let parsed = Selector::parse_string_with_options(rest, options).ok()?;
        let mut flat = vec![Component::Combinator(combinator)];
        flat.extend(flatten_selector_parse_order(&parsed));
        Selector::from(flat).into_owned()
    } else {
        Selector::parse_string_with_options(item_text, options)
            .ok()?
            .into_owned()
    };

    normalize_global_pseudos(&mut selector);
    Some(selector)
}

/// Split a `:global(...)` argument into its top-level comma-separated items, ignoring commas
/// nested inside `()`/`[]` or string literals.
fn split_top_level_commas(input: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut start = 0;
    let mut paren = 0i32;
    let mut bracket = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(open_quote) = quote {
            match ch {
                '\\' => escaped = true,
                _ if ch == open_quote => quote = None,
                _ => {}
            }
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' | '\'' => quote = Some(ch),
            '(' => paren += 1,
            ')' => paren -= 1,
            '[' => bracket += 1,
            ']' => bracket -= 1,
            ',' if paren == 0 && bracket == 0 => {
                items.push(&input[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    items.push(&input[start..]);
    items
}

fn split_leading_combinator_str(input: &str) -> Option<(Combinator, &str)> {
    let input = input.trim_start();
    match input.as_bytes().first()? {
        b'>' => Some((Combinator::Child, input[1..].trim_start())),
        b'+' => Some((Combinator::NextSibling, input[1..].trim_start())),
        b'~' => Some((Combinator::LaterSibling, input[1..].trim_start())),
        _ => None,
    }
}

/// Serialize `:global(...)` argument tokens back to CSS so they can be re-parsed as a
/// selector. lightningcss's own `Token` serializer handles escaping and number formatting;
/// re-parsing discards any comments.
fn token_list_to_css(tokens: &TokenList<'_>) -> String {
    let mut result = String::new();
    {
        let mut printer = Printer::new(&mut result, PrinterOptions::default());
        for item in &tokens.0 {
            if let TokenOrValue::Token(token) = item {
                let _ = token.to_css(&mut printer);
            }
        }
    }
    result
}

fn hoist_global_leading_combinators<'i>(selector: &mut Selector<'i>) {
    for component in selector.iter_mut_raw_match_order() {
        if let Component::NonTSPseudoClass(PseudoClass::Global { selector: inner }) = component {
            hoist_global_leading_combinators(inner);
        }
    }

    let mut flat = flatten_selector_parse_order(selector);

    let mut index = 0;
    while index < flat.len() {
        let Component::NonTSPseudoClass(PseudoClass::Global { selector: inner }) = &flat[index]
        else {
            index += 1;
            continue;
        };

        let Some((leading, rest)) = split_selector_leading_combinator(inner.as_ref()) else {
            index += 1;
            continue;
        };

        flat[index] = Component::NonTSPseudoClass(PseudoClass::Global {
            selector: Box::new(rest),
        });

        // Sass nesting emits a descendant space before `:global(> …)`; the hoisted inner
        // combinator replaces it (`article :global(> *)` → `article > *`).
        if index > 0 && matches!(flat[index - 1], Component::Combinator(_)) {
            flat[index - 1] = Component::Combinator(leading);
        } else {
            flat.insert(index, Component::Combinator(leading));
            index += 1;
        }

        index += 1;
    }

    if !flat.is_empty() {
        *selector = flat.into();
    }
}

fn split_selector_leading_combinator<'i>(
    selector: &Selector<'i>,
) -> Option<(Combinator, Selector<'i>)> {
    let flat = flatten_selector_parse_order(selector);
    let (first, rest) = flat.split_first()?;
    let Component::Combinator(combinator) = first else {
        return None;
    };
    if !combinator.is_tree_combinator() {
        return None;
    }
    Some((*combinator, rest.to_vec().into()))
}

/// Flatten a selector into a parse-order `Vec` with combinators interleaved.
///
/// The internal storage is right-to-left (match order) with intra-compound components in
/// parse order, so we reverse the *compound* order while preserving each compound's internal
/// order. A naive `.rev()` would also reverse intra-compound order, placing pseudo-classes
/// before type selectors (`:not(.x)html` instead of `html:not(.x)`).
fn flatten_selector_parse_order<'i>(selector: &Selector<'i>) -> Vec<Component<'i>> {
    let mut flat = Vec::with_capacity(selector.len());
    for (index, (combinator, compound)) in split_into_compounds(selector).enumerate() {
        if index > 0
            && let Some(combinator) = combinator
        {
            flat.push(Component::Combinator(combinator));
        }
        flat.extend(compound);
    }
    flat
}

fn split_into_compounds<'i>(
    selector: &Selector<'i>,
) -> impl Iterator<Item = (Option<Combinator>, Vec<Component<'i>>)> {
    let raw_slice = selector.iter_raw_match_order().as_slice();
    let mut combinators = selector
        .iter_raw_match_order()
        .rev()
        .filter_map(|component| component.as_combinator());

    let compound_slices = raw_slice.split(|component| component.is_combinator()).rev();

    compound_slices.enumerate().map(move |(index, compound)| {
        let combinator = if index == 0 { None } else { combinators.next() };
        (combinator, compound.to_vec())
    })
}

/// Stands in for a `*` during printing. Kept implausible as author CSS so the string
/// restore below can't collide with a real selector.
const GLOBAL_UNIVERSAL_MARKER_NAME: &str = "--astro-scope-global-universal";

/// lightningcss drops bare `*` before functional pseudos when printing (`*:nth-child` →
/// `:nth-child`). We emit the marker as a workaround, then restore the canonical `*`.
fn restore_printable_global_universal(css: &str) -> String {
    css.replace(&format!(":{GLOBAL_UNIVERSAL_MARKER_NAME}"), "*")
}

/// Scope CSS selectors.
///
/// Parses the CSS, identifies style rules, transforms their selectors to include
/// the scope identifier, and returns the CSS.
///
/// If parsing fails, returns the original CSS unchanged.
pub fn scope_css(css: &str, scope: &str, strategy: ScopedStyleStrategy) -> String {
    let options = parser_options();

    let mut stylesheet = match StyleSheet::parse(css, options) {
        Ok(stylesheet) => stylesheet,
        Err(_) => return css.to_string(),
    };

    let mut global_visitor = GlobalSelectorVisitor;
    let _ = stylesheet.visit(&mut global_visitor);

    let mut visitor = ScopeVisitor { scope, strategy };
    let _ = stylesheet.visit(&mut visitor);

    // Print — no minification, that's handled elsewhere in the pipeline.
    // Any remaining `PseudoClass::Global { selector }` nodes will have their `:global()`
    // wrapper stripped by the printer (it serializes the inner selector directly).
    let result = stylesheet
        .to_css(PrinterOptions::default())
        .unwrap_or_else(|_| lightningcss::stylesheet::ToCssResult {
            code: css.to_string(),
            exports: None,
            references: None,
            dependencies: None,
        });

    restore_printable_global_universal(&result.code)
}

struct ScopeVisitor<'a> {
    scope: &'a str,
    strategy: ScopedStyleStrategy,
}

impl<'i> Visitor<'i> for ScopeVisitor<'_> {
    type Error = Infallible;

    fn visit_types(&self) -> VisitTypes {
        visit_types!(SELECTORS)
    }

    fn visit_selector_list(&mut self, selectors: &mut SelectorList<'i>) -> Result<(), Self::Error> {
        let new_selectors: Vec<Selector<'i>> = selectors
            .0
            .iter()
            .flat_map(|selector| self.scope_selector(selector))
            .collect();

        selectors.0 = new_selectors.into();
        Ok(())
    }
}

impl ScopeVisitor<'_> {
    fn scope_component<'i>(&self) -> Component<'i> {
        match self.strategy {
            ScopedStyleStrategy::Where => {
                // :where(.astro-XXXX)
                let class_component =
                    Component::Class(Ident(format!("astro-{}", self.scope).into()));
                let inner_selector: Selector<'i> = vec![class_component].into();
                Component::Where(Box::new([inner_selector]))
            }
            ScopedStyleStrategy::Class => {
                // .astro-XXXX
                Component::Class(Ident(format!("astro-{}", self.scope).into()))
            }
            ScopedStyleStrategy::Attribute => {
                // [data-astro-cid-XXXX]
                let attr_name: CowArcStr<'i> = format!("data-astro-cid-{}", self.scope).into();
                Component::AttributeInNoNamespaceExists {
                    local_name: Ident(attr_name.clone()),
                    local_name_lower: Ident(attr_name),
                }
            }
        }
    }

    /// Scope a single selector, potentially returning multiple selectors.
    fn scope_selector<'i>(&self, selector: &Selector<'i>) -> Vec<Selector<'i>> {
        let compounds = split_into_compounds(selector);
        let merged = self.merge_pseudo_element_compounds(compounds);

        // Selectors that only reference `&` from inside `:is()`/`:where()`/`:not()`/`:has()`
        // have no top-level nesting to scope against — leaving them untouched avoids
        // injecting a redundant scope next to the hidden parent reference.
        let has_top_level_nesting = merged
            .iter()
            .any(|(_, compound)| compound.iter().any(|c| matches!(c, Component::Nesting)));
        let has_any_nesting = merged
            .iter()
            .any(|(_, compound)| compound.iter().any(component_contains_nesting));
        if has_any_nesting && !has_top_level_nesting {
            return vec![selector.clone()];
        }

        let mut result_components: Vec<Component<'i>> = Vec::new();

        let starts_with_bare_nesting = merged
            .first()
            .map(|(_, compound)| compound.len() == 1 && matches!(compound[0], Component::Nesting))
            .unwrap_or(false);

        for (i, (combinator, compound)) in merged.iter().enumerate() {
            if i > 0
                && let Some(combinator) = combinator
            {
                result_components.push(Component::Combinator(*combinator));
            }

            // Skip scope injection for compounds that follow the leading bare `&` via a
            // descendant combinator — they are descendant selectors, not the scoped element.
            let skip_scope = starts_with_bare_nesting
                && i > 0
                && matches!(combinator, Some(Combinator::Descendant));

            if skip_scope {
                result_components.extend(compound.iter().cloned());
            } else {
                let scoped = self.scope_compound(compound);
                result_components.extend(scoped);
            }
        }

        if result_components.is_empty() {
            return vec![];
        }

        vec![result_components.into()]
    }

    /// Merge pseudo-element compounds back into the preceding compound.
    ///
    /// `Combinator::PseudoElement`, `Combinator::Part`, and `Combinator::SlotAssignment`
    /// are all internal markers (not real CSS combinators) that parcel_selectors emits
    /// to the left of `::before`/`::after`, `::part(...)`, and `::slotted(...)`
    /// respectively. All three serialize as the empty string and must be re-attached
    /// to the preceding compound so scoping treats e.g. `.x::part(y)` as one unit.
    fn merge_pseudo_element_compounds<'i>(
        &self,
        compounds: impl Iterator<Item = (Option<Combinator>, Vec<Component<'i>>)>,
    ) -> Vec<(Option<Combinator>, Vec<Component<'i>>)> {
        let mut result: Vec<(Option<Combinator>, Vec<Component<'i>>)> = Vec::new();

        for (combinator, compound) in compounds {
            if matches!(
                combinator,
                Some(Combinator::PseudoElement | Combinator::Part | Combinator::SlotAssignment)
            ) {
                if let Some(last) = result.last_mut() {
                    last.1.extend(compound);
                } else {
                    result.push((combinator, compound));
                }
            } else {
                result.push((combinator, compound));
            }
        }

        result
    }

    /// Scope a single compound selector, returning the scoped components.
    fn scope_compound<'i>(&self, compound: &[Component<'i>]) -> Vec<Component<'i>> {
        if compound.is_empty() {
            return vec![];
        }

        if self.has_nesting(compound) {
            return self.scope_nesting_compound(compound);
        }

        if self.is_global_compound(compound) {
            return self.process_global_compound(compound);
        }

        // `:root` is never scoped
        if compound.len() == 1 && matches!(&compound[0], Component::Root) {
            return compound.to_vec();
        }

        // `body`/`html` are never scoped at the compound level
        if self.is_body_or_html(compound) {
            return compound.to_vec();
        }

        self.inject_scope_into_compound(compound)
    }

    fn has_nesting(&self, compound: &[Component<'_>]) -> bool {
        compound.iter().any(|c| matches!(c, Component::Nesting))
    }

    fn is_global_compound(&self, compound: &[Component<'_>]) -> bool {
        compound
            .iter()
            .any(|c| matches!(c, Component::NonTSPseudoClass(PseudoClass::Global { .. })))
    }

    fn is_body_or_html(&self, compound: &[Component<'_>]) -> bool {
        compound.iter().any(|c| match c {
            Component::LocalName(local) => {
                let name = local.name.0.as_ref();
                name == "body" || name == "html"
            }
            _ => false,
        })
    }

    /// Expand a `:global()` inner selector, preserving `*` when functional pseudos follow.
    ///
    /// `trailing_sibling_pseudos` covers the sibling form (`:global(*):nth-child(2n)`).
    /// Pseudos inside the `:global()` argument are handled from `selector` directly
    /// (`:global(*:nth-child(2n))`).
    fn expand_global_inner<'i>(
        selector: &Selector<'i>,
        trailing_sibling_pseudos: &[Component<'i>],
    ) -> Vec<Component<'i>> {
        let flat = flatten_selector_parse_order(selector);
        if matches!(flat.first(), Some(Component::ExplicitUniversalType))
            && (flat.len() > 1 || !trailing_sibling_pseudos.is_empty())
            && flat[1..]
                .iter()
                .all(|component| is_pseudo_class(component) || is_pseudo_element(component))
        {
            let mut result = vec![Self::printable_global_universal()];
            result.extend(flat.into_iter().skip(1));
            result.extend(trailing_sibling_pseudos.iter().cloned());
            return result;
        }

        let mut result = flat;
        result.extend(trailing_sibling_pseudos.iter().cloned());
        result
    }

    /// Emits the marker as a pseudo-class — lightningcss keeps it before a functional pseudo,
    /// whereas it would drop a bare `*`. Restored to `*` after printing.
    fn printable_global_universal<'i>() -> Component<'i> {
        Component::NonTSPseudoClass(PseudoClass::Custom {
            name: GLOBAL_UNIVERSAL_MARKER_NAME.into(),
        })
    }

    /// Process a compound that contains :global() — strip the wrapper and return
    /// inner content unscoped, while scoping a leading local part.
    ///
    /// Only a local (non-global) simple selector that appears *before* the first `:global()`
    /// makes the compound scoped (`.local:global(.g)` → `.local[scope].g`). Once a `:global()`
    /// has been seen, any following type/class/id/attribute selector belongs to the global
    /// compound and stays unscoped (`:global(a)[role="button"]` → `a[role="button"]`,
    /// `:global(a).cls` → `a.cls`).
    fn process_global_compound<'i>(&self, compound: &[Component<'i>]) -> Vec<Component<'i>> {
        let mut result = Vec::new();
        let mut has_leading_local = false;
        let mut seen_global = false;
        let mut index = 0;

        while index < compound.len() {
            let component = &compound[index];
            if let Component::NonTSPseudoClass(PseudoClass::Global { selector }) = component {
                seen_global = true;
                let trailing_end = compound[index + 1..]
                    .iter()
                    .take_while(|component| {
                        is_pseudo_class(component) || is_pseudo_element(component)
                    })
                    .count();
                let trailing = &compound[index + 1..index + 1 + trailing_end];
                result.extend(Self::expand_global_inner(selector, trailing));
                index += 1 + trailing_end;
                continue;
            }

            // Pseudo-classes (e.g. `:nth-child(2n)` in `:global(*):nth-child(2n)`) and
            // pseudo-elements (e.g. `::after` in `:global(.fallback)::after`, or
            // `::part(x)` / `::slotted(x)`) attach to the content extracted from
            // :global() and must NOT trigger scoping.
            if is_pseudo_class(component) || is_pseudo_element(component) {
                result.push(component.clone());
                index += 1;
                continue;
            }

            // A leading local part (before any `:global()`) is what gets scoped. A local part
            // after a `:global()` is a suffix on the global compound and stays unscoped.
            if !seen_global {
                has_leading_local = true;
            }
            result.push(component.clone());
            index += 1;
        }

        // After expanding :global(), check if the result contains body/html — if so, no scoping
        if self.is_body_or_html(&result) {
            return result;
        }

        // Scope only when there was a leading local part (e.g., `.class:global(.bar)`).
        if has_leading_local {
            return self.inject_scope_into_compound(&result);
        }

        result
    }

    /// `&` already carries scope via the parent rule, so a compound containing
    /// `&` is returned unchanged — re-injecting scope here would require the
    /// parent element to carry the scope class, which descendants don't always do.
    fn scope_nesting_compound<'i>(&self, compound: &[Component<'i>]) -> Vec<Component<'i>> {
        compound.to_vec()
    }

    /// Inject the scope component into a compound selector.
    fn inject_scope_into_compound<'i>(&self, compound: &[Component<'i>]) -> Vec<Component<'i>> {
        let scope_component = self.scope_component();
        let mut result = Vec::new();
        let mut scoped = false;

        let only_pseudo = compound
            .iter()
            .all(|c| is_pseudo_class(c) || is_pseudo_element(c));

        for (i, component) in compound.iter().enumerate() {
            match component {
                Component::ExplicitUniversalType => {
                    // `*` — replace with scope
                    result.push(scope_component.clone());
                    scoped = true;
                }
                Component::LocalName(_) | Component::Class(_) | Component::ID(_) => {
                    result.push(component.clone());
                    push_scope_if_unscoped(&mut result, &mut scoped, &scope_component);
                }
                Component::AttributeInNoNamespaceExists { .. }
                | Component::AttributeInNoNamespace { .. }
                | Component::AttributeOther(_)
                | Component::PseudoElement(_)
                | Component::Part(_)
                | Component::Slotted(_) => {
                    push_scope_if_unscoped(&mut result, &mut scoped, &scope_component);
                    result.push(component.clone());
                }
                Component::Root => {
                    result.push(component.clone());
                    scoped = true;
                }
                _ => {
                    if only_pseudo && i == 0 {
                        push_scope_if_unscoped(&mut result, &mut scoped, &scope_component);
                    }
                    result.push(component.clone());
                }
            }
        }

        if !scoped {
            result.push(scope_component);
        }

        result
    }
}

fn push_scope_if_unscoped<'i>(
    result: &mut Vec<Component<'i>>,
    scoped: &mut bool,
    scope_component: &Component<'i>,
) {
    if !*scoped {
        result.push(scope_component.clone());
        *scoped = true;
    }
}

/// Walks recursively into `:is(...)`, `:where(...)`, `:not(...)`, and `:has(...)`.
fn component_contains_nesting(component: &Component<'_>) -> bool {
    match component {
        Component::Nesting => true,
        Component::Is(list) | Component::Where(list) | Component::Has(list) => list
            .iter()
            .any(|s| s.iter_raw_match_order().any(component_contains_nesting)),
        Component::Negation(list) => list
            .iter()
            .any(|s| s.iter_raw_match_order().any(component_contains_nesting)),
        _ => false,
    }
}

/// Check if a component is a pseudo-element.
///
/// Includes `::part(...)` and `::slotted(...)`, which are distinct `Component`
/// variants in parcel_selectors but follow the same scoping rules as `::before`
/// and friends — attribute selectors may not be emitted after them.
fn is_pseudo_element(component: &Component<'_>) -> bool {
    matches!(
        component,
        Component::PseudoElement(_) | Component::Part(_) | Component::Slotted(_)
    )
}

/// Check if a component is a pseudo-class.
///
/// Includes `:host` / `:host(...)` (stored as its own `Component::Host` variant
/// in parcel_selectors rather than a `NonTSPseudoClass`) so a compound that is
/// only `:host` or only `:host(...)` is recognized as pseudo-only and gets the
/// scope injected before it — matching the Go compiler's output shape.
fn is_pseudo_class(component: &Component<'_>) -> bool {
    matches!(
        component,
        Component::NonTSPseudoClass(_)
            | Component::Negation(_)
            | Component::Root
            | Component::Empty
            | Component::Scope
            | Component::Nth(_)
            | Component::NthOf(_)
            | Component::Is(_)
            | Component::Where(_)
            | Component::Has(_)
            | Component::Host(_)
    )
}

/// Elements that should never receive a scope class in the HTML.
pub const NEVER_SCOPED_ELEMENTS: &[&str] = &[
    "Fragment", "base", "font", "frame", "frameset", "head", "link", "meta", "noframes",
    "noscript", "script", "style", "slot", "title",
];

/// Check if an element should receive a scope class.
pub fn should_scope_element(name: &str) -> bool {
    !NEVER_SCOPED_ELEMENTS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(source: &str) -> String {
        scope_css(source, "xxxxxx", ScopedStyleStrategy::Where)
    }

    #[allow(dead_code)]
    fn scope_class(source: &str) -> String {
        scope_css(source, "xxxxxx", ScopedStyleStrategy::Class)
    }

    #[allow(dead_code)]
    fn scope_attribute(source: &str) -> String {
        scope_css(source, "xxxxxx", ScopedStyleStrategy::Attribute)
    }

    // Note: lightningcss pretty-prints CSS (spaces around combinators, newlines,
    // indentation, trailing newline). It also normalizes:
    // - `::before`/`::after` → `:before`/`:after`
    // - attribute values get quoted
    // - media queries modernized: `min-width: 640px` → `width >= 640px`
    // - colors shortened: `blue` → `#00f`, `black` → `#000`, etc.
    // - `rotate(0deg)` → `rotate(0)`
    // - SelectorBuilder may reorder components within compounds

    #[test]
    fn test_class() {
        assert_eq!(scope(".class{}"), ".class:where(.astro-xxxxxx) {\n}\n");
    }

    #[test]
    fn test_id() {
        assert_eq!(scope("#class{}"), "#class:where(.astro-xxxxxx) {\n}\n");
    }

    #[test]
    fn test_element() {
        assert_eq!(scope("h1{}"), "h1:where(.astro-xxxxxx) {\n}\n");
    }

    #[test]
    fn test_adjacent_sibling() {
        assert_eq!(
            scope(".class+.class{}"),
            ".class:where(.astro-xxxxxx) + .class:where(.astro-xxxxxx) {\n}\n"
        );
    }

    #[test]
    fn test_and_selector() {
        assert_eq!(
            scope(".class,.class{}"),
            ".class:where(.astro-xxxxxx), .class:where(.astro-xxxxxx) {\n}\n"
        );
    }

    #[test]
    fn test_children_universal() {
        assert_eq!(
            scope(".class *{}"),
            ".class:where(.astro-xxxxxx) :where(.astro-xxxxxx) {\n}\n"
        );
    }

    #[test]
    fn test_attr() {
        assert_eq!(
            scope("a[aria-current=page]{}"),
            "a:where(.astro-xxxxxx)[aria-current=\"page\"] {\n}\n"
        );
    }

    #[test]
    fn test_attr_universal_implied() {
        assert_eq!(
            scope("[aria-visible],[aria-hidden]{}"),
            ":where(.astro-xxxxxx)[aria-visible], :where(.astro-xxxxxx)[aria-hidden] {\n}\n"
        );
    }

    #[test]
    fn test_universal_pseudo_state() {
        assert_eq!(scope("*:hover{}"), ":where(.astro-xxxxxx):hover {\n}\n");
    }

    #[test]
    fn test_immediate_child_universal() {
        assert_eq!(
            scope(".class>*{}"),
            ".class:where(.astro-xxxxxx) > :where(.astro-xxxxxx) {\n}\n"
        );
    }

    #[test]
    fn test_element_pseudo_state() {
        assert_eq!(
            scope(".class button:focus{}"),
            ".class:where(.astro-xxxxxx) button:where(.astro-xxxxxx):focus {\n}\n"
        );
    }

    #[test]
    fn test_element_pseudo_element() {
        assert_eq!(
            scope(".class h3::before{}"),
            ".class:where(.astro-xxxxxx) h3:where(.astro-xxxxxx):before {\n}\n"
        );
    }

    #[test]
    fn test_element_pseudo_state_pseudo_element() {
        assert_eq!(
            scope("button:focus::before{}"),
            "button:where(.astro-xxxxxx):focus:before {\n}\n"
        );
    }

    #[test]
    fn test_media_query() {
        assert_eq!(
            scope("@media screen and (min-width:640px){.class{}}"),
            "@media screen and (width >= 640px) {\n  .class:where(.astro-xxxxxx) {\n  }\n}\n"
        );
    }

    #[test]
    fn test_global_children() {
        assert_eq!(
            scope(".class :global(ul li){}"),
            ".class:where(.astro-xxxxxx) ul li {\n}\n"
        );
    }

    #[test]
    fn test_global_universal() {
        assert_eq!(
            scope(".class :global(*){}"),
            ".class:where(.astro-xxxxxx) * {\n}\n"
        );
    }

    #[test]
    fn test_global_with_scoped_children() {
        assert_eq!(
            scope(":global(section) .class{}"),
            "section .class:where(.astro-xxxxxx) {\n}\n"
        );
    }

    #[test]
    fn test_subsequent_siblings_global() {
        assert_eq!(
            scope(".class~:global(a){}"),
            ".class:where(.astro-xxxxxx) ~ a {\n}\n"
        );
    }

    #[test]
    fn test_global_nested_parens() {
        assert_eq!(
            scope(".class :global(.nav:not(.is-active)){}"),
            ".class:where(.astro-xxxxxx) .nav:not(.is-active) {\n}\n"
        );
    }

    #[test]
    fn test_global_chaining_global() {
        assert_eq!(scope(":global(.foo):global(.bar){}"), ".foo.bar {\n}\n");
    }

    #[test]
    fn test_class_chained_global() {
        assert_eq!(
            scope(".class:global(.bar){}"),
            ".class:where(.astro-xxxxxx).bar {\n}\n"
        );
    }

    #[test]
    fn test_body() {
        assert_eq!(scope("body h1{}"), "body h1:where(.astro-xxxxxx) {\n}\n");
    }

    #[test]
    fn test_body_class() {
        assert_eq!(scope("body.theme-dark{}"), "body.theme-dark {\n}\n");
    }

    #[test]
    fn test_html_and_body() {
        assert_eq!(scope("html,body{}"), "html, body {\n}\n");
    }

    #[test]
    fn test_root() {
        assert_eq!(scope(":root{}"), ":root {\n}\n");
    }

    #[test]
    fn test_root_with_class() {
        // :root.dark should NOT be scoped — Go: scoped=true when :root seen
        assert_eq!(scope(":root.dark{}"), ":root.dark {\n}\n");
    }

    #[test]
    fn test_root_with_not() {
        // :root:not(.theme) should NOT be scoped
        assert_eq!(scope(":root:not(.theme){}"), ":root:not(.theme) {\n}\n");
    }

    #[test]
    fn test_chained_not() {
        assert_eq!(
            scope(".class:not(.is-active):not(.is-disabled){}"),
            ".class:where(.astro-xxxxxx):not(.is-active):not(.is-disabled) {\n}\n"
        );
    }

    #[test]
    fn test_weird_chaining() {
        assert_eq!(
            scope(":hover.a:focus{}"),
            ":hover.a:where(.astro-xxxxxx):focus {\n}\n"
        );
    }

    #[test]
    fn test_more_weird_chaining() {
        assert_eq!(
            scope(":not(.is-disabled).a{}"),
            ":not(.is-disabled).a:where(.astro-xxxxxx) {\n}\n"
        );
    }

    #[test]
    fn test_keyframes() {
        assert_eq!(
            scope("@keyframes shuffle{from{transform:rotate(0deg);}to{transform:rotate(360deg);}}"),
            "@keyframes shuffle {\n  from {\n    transform: rotate(0);\n  }\n\n  to {\n    transform: rotate(360deg);\n  }\n}\n"
        );
    }

    #[test]
    fn test_variables() {
        assert_eq!(
            scope("body{--bg:red;background:var(--bg);color:black;}"),
            "body {\n  --bg: red;\n  background: var(--bg);\n  color: #000;\n}\n"
        );
    }

    #[test]
    fn test_calc() {
        assert_eq!(
            scope(":root{padding:calc(var(--space) * 2);}"),
            ":root {\n  padding: calc(var(--space) * 2);\n}\n"
        );
    }

    #[test]
    fn test_class_strategy() {
        assert_eq!(scope_class(".class{}"), ".class.astro-xxxxxx {\n}\n");
    }

    #[test]
    fn test_attribute_strategy() {
        assert_eq!(
            scope_attribute(".class{}"),
            ".class[data-astro-cid-xxxxxx] {\n}\n"
        );
    }

    #[test]
    fn test_attribute_strategy_with_part_pseudo_element() {
        // Regression: attribute selector must NOT be emitted after `::part(...)`.
        // CSS disallows attribute selectors following a pseudo-element, and
        // lightningcss minification rejects it as a SyntaxError.
        assert_eq!(
            scope_attribute(".baseline-status::part(root){}"),
            ".baseline-status[data-astro-cid-xxxxxx]::part(root) {\n}\n"
        );
    }

    #[test]
    fn test_attribute_strategy_with_before_pseudo_element() {
        assert_eq!(
            scope_attribute("h3::before{}"),
            "h3[data-astro-cid-xxxxxx]:before {\n}\n"
        );
    }

    #[test]
    fn test_attribute_strategy_with_slotted_pseudo_element() {
        // `::slotted(...)` uses `Combinator::SlotAssignment` internally, the same
        // dummy-combinator pattern as `::part(...)`.
        assert_eq!(
            scope_attribute(".host::slotted(span){}"),
            ".host[data-astro-cid-xxxxxx]::slotted(span) {\n}\n"
        );
    }

    #[test]
    fn test_where_strategy_with_part_pseudo_element() {
        assert_eq!(
            scope(".baseline-status::part(root){}"),
            ".baseline-status:where(.astro-xxxxxx)::part(root) {\n}\n"
        );
    }

    #[test]
    fn test_attribute_strategy_with_host_pseudo_class() {
        // Go compiler emits the scope attribute BEFORE `:host` because `:host`
        // acts like a type selector (the compound has no "real" selector to scope after).
        assert_eq!(
            scope_attribute(":host{}"),
            "[data-astro-cid-xxxxxx]:host {\n}\n"
        );
    }

    #[test]
    fn test_attribute_strategy_with_host_function() {
        assert_eq!(
            scope_attribute(":host(.dark){}"),
            "[data-astro-cid-xxxxxx]:host(.dark) {\n}\n"
        );
    }

    #[test]
    fn test_attribute_strategy_with_host_context() {
        assert_eq!(
            scope_attribute(":host-context(.dark){}"),
            "[data-astro-cid-xxxxxx]:host-context(.dark) {\n}\n"
        );
    }

    #[test]
    fn test_attribute_strategy_with_chained_pseudo_elements() {
        // Two pseudo-elements in the same compound parse as nested `PseudoElement`
        // combinator jumps; all three must collapse into one compound so the scope
        // attribute is injected once, before the first pseudo-element.
        assert_eq!(
            scope_attribute(".host::part(root)::before{}"),
            ".host[data-astro-cid-xxxxxx]::part(root):before {\n}\n"
        );
    }

    #[test]
    fn test_attribute_strategy_with_part_then_pseudo_class() {
        assert_eq!(
            scope_attribute(".host::part(root):hover{}"),
            ".host[data-astro-cid-xxxxxx]::part(root):hover {\n}\n"
        );
    }

    #[test]
    fn test_attribute_strategy_global_with_part() {
        // `:global(.fallback)::part(y)` — the `::part(...)` attaches to global
        // content and must stay completely unscoped.
        assert_eq!(
            scope_attribute(":global(.fallback)::part(y){}"),
            ".fallback::part(y) {\n}\n"
        );
    }

    #[test]
    fn test_nesting_combinator() {
        assert_eq!(
            scope("div{& span{color:blue}}"),
            "div:where(.astro-xxxxxx) {\n  & span {\n    color: #00f;\n  }\n}\n"
        );
    }

    #[test]
    fn test_nesting_without_ampersand() {
        assert_eq!(
            scope("nav{a{color:deeppink}}"),
            "nav:where(.astro-xxxxxx) {\n  & a {\n    color: #ff1493;\n  }\n}\n"
        );
    }

    #[test]
    fn test_nesting_modifier() {
        assert_eq!(
            scope(".header{background-color:white;&.dark{background-color:blue}}"),
            ".header:where(.astro-xxxxxx) {\n  background-color: #fff;\n\n  &.dark {\n    background-color: #00f;\n  }\n}\n"
        );
    }

    #[test]
    fn test_container() {
        assert_eq!(
            scope("@container (min-width: 200px) and (min-height: 200px){h1{font-size:30px}}"),
            "@container (width >= 200px) and (height >= 200px) {\n  h1:where(.astro-xxxxxx) {\n    font-size: 30px;\n  }\n}\n"
        );
    }

    #[test]
    fn test_layer() {
        assert_eq!(
            scope("@layer theme,layout,utilities;@layer special{.item{color:rebeccapurple}}"),
            "@layer theme, layout, utilities;\n\n@layer special {\n  .item:where(.astro-xxxxxx) {\n    color: #639;\n  }\n}\n"
        );
    }

    #[test]
    fn test_starting_style() {
        assert_eq!(
            scope("@starting-style{.class{}}"),
            "@starting-style {\n  .class:where(.astro-xxxxxx) {\n  }\n}\n"
        );
    }

    #[test]
    fn test_only_pseudo_element() {
        assert_eq!(
            scope(".class>::before{}"),
            ".class:where(.astro-xxxxxx) > :where(.astro-xxxxxx):before {\n}\n"
        );
    }

    #[test]
    fn test_only_pseudo_class_and_pseudo_element() {
        assert_eq!(
            scope(".class>:not(:first-child)::after{}"),
            ".class:where(.astro-xxxxxx) > :where(.astro-xxxxxx):not(:first-child):after {\n}\n"
        );
    }

    #[test]
    fn test_escaped_characters() {
        assert_eq!(
            scope(".class\\:class:focus{}"),
            ".class\\:class:where(.astro-xxxxxx):focus {\n}\n"
        );
    }

    #[test]
    fn test_nesting_without_ampersand_deep() {
        // Deeper descendant nesting: `nav { a { span { color: red } } }`
        // Neither `a` nor `span` should be scoped.
        assert_eq!(
            scope("nav{a{span{color:red}}}"),
            "nav:where(.astro-xxxxxx) {\n  & a {\n    & span {\n      color: red;\n    }\n  }\n}\n"
        );
    }

    #[test]
    fn test_nesting_mixed() {
        // Comprehensive real-world nesting scenario (withastro/astro#15907):
        //
        // nav { color: blue }          — top-level, scoped
        // nav { a { color: deeppink } } — `a` is a descendant, NOT scoped
        // nav { &:hover { opacity: .8 } } — `&:hover` modifies nav itself, scoped
        // nav { & > li { color: red } } — child combinator (not descendant), scoped
        // nav { a { span { color: green } } } — deeply nested descendants, neither scoped
        // nav { &::before { content: "" } } — pseudo-element on nav itself, scoped
        let input = "nav{color:blue} nav{a{color:deeppink}} nav{&:hover{opacity:.8}} nav{& > li{color:red}} nav{a{span{color:green}}} nav{&::before{content:\"\"}}";
        let output = scope(input);
        // nav itself is always scoped
        assert!(
            output.contains("nav:where(.astro-xxxxxx)"),
            "nav should be scoped: {output}"
        );
        // descendant `a` must NOT be scoped
        assert!(
            !output.contains("a:where("),
            "a should not be scoped: {output}"
        );
        // descendant `span` must NOT be scoped
        assert!(
            !output.contains("span:where("),
            "span should not be scoped: {output}"
        );
        // &:hover — `&` has a modifier in the same compound, returned as-is by scope_nesting_compound
        assert!(
            output.contains("&:hover"),
            "&:hover should be preserved: {output}"
        );
        assert!(
            !output.contains("&:where(.astro-xxxxxx):hover"),
            "&:hover should not get extra scope: {output}"
        );
        // child combinator `> li` — explicit child, NOT via descendant combinator, should be scoped
        assert!(
            output.contains("li:where(.astro-xxxxxx)"),
            "> li should be scoped: {output}"
        );
        // &::before — `&` refers to the parent (nav, already scoped); no extra scope on the &::before compound
        assert!(
            output.contains("&:before"),
            "&::before should be preserved: {output}"
        );
        assert!(
            !output.contains("&:where(.astro-xxxxxx):before"),
            "&::before should not get extra scope: {output}"
        );
    }

    #[test]
    fn test_nested_only_pseudo_element() {
        // `.class { & .other_class { &::after {} } }`:
        // - `& .other_class` — descendant, NOT scoped (withastro/astro#15907)
        // - `&::after` — `&` already refers to `.other_class`, no extra scope needed
        assert_eq!(
            scope(".class{& .other_class{&::after{}}}"),
            ".class:where(.astro-xxxxxx) {\n  & .other_class {\n    &:after {\n    }\n  }\n}\n"
        );
    }

    #[test]
    fn test_global_with_pseudo_element() {
        // :global(.fallback)::after must NOT be scoped — the ::after attaches to the
        // content inside :global() and the whole selector is intentionally unscoped.
        assert_eq!(
            scope(":global(.fallback)::after{}"),
            ".fallback:after {\n}\n"
        );
    }

    #[test]
    fn test_global_with_pseudo_element_before() {
        assert_eq!(
            scope(":global(.fallback)::before{}"),
            ".fallback:before {\n}\n"
        );
    }

    #[test]
    fn test_global_then_class_with_pseudo_element() {
        // .local:global(.global)::after — .local is non-global, so scoping applies,
        // but ::after should not cause an extra scope injection.
        assert_eq!(
            scope(".local:global(.global)::after{}"),
            ".local:where(.astro-xxxxxx).global:after {\n}\n"
        );
    }

    #[test]
    fn test_global_nested_media() {
        assert_eq!(
            scope(
                ":global(html) { @media (min-width: 640px) { color: blue } }html { background-color: lime }"
            ),
            "html {\n  @media (width >= 640px) {\n    color: #00f;\n  }\n}\n\nhtml {\n  background-color: #0f0;\n}\n"
        );
    }

    // Additional Go compiler tests

    #[test]
    fn test_global_nested_parens_chained() {
        assert_eq!(
            scope(":global(body:not(.is-light)).is-dark,:global(body:not(.is-dark)).is-light{}"),
            "body:not(.is-light).is-dark, body:not(.is-dark).is-light {\n}\n"
        );
    }

    #[test]
    fn test_global_compound_with_not() {
        // Regression: `:global(html:not(.theme-dark))` must keep the type selector
        // `html` before `:not(.theme-dark)`, not produce invalid `:not(.theme-dark)html`.
        assert_eq!(
            scope(
                ":global(.theme-dark) .icon.dark, :global(html:not(.theme-dark)) .icon.light, button[aria-pressed='false'] .icon.light { color: var(--accent-text-over); }"
            ),
            ".theme-dark .icon:where(.astro-xxxxxx).dark, html:not(.theme-dark) .icon:where(.astro-xxxxxx).light, button:where(.astro-xxxxxx)[aria-pressed=\"false\"] .icon:where(.astro-xxxxxx).light {\n  color: var(--accent-text-over);\n}\n"
        );
    }

    #[test]
    fn test_nesting_inside_is_pseudo() {
        // The literal output is `& .x` rather than `:is(&) .x` because lightningcss
        // simplifies `:is(&)` → `&` at print time — semantically equivalent.
        assert_eq!(
            scope(".box{:is(&) .x{color:red}}"),
            ".box:where(.astro-xxxxxx) {\n  & .x {\n    color: red;\n  }\n}\n"
        );
    }

    #[test]
    fn test_nesting_inside_where_pseudo() {
        // lightningcss preserves `:where(&)` (different specificity from `&`).
        assert_eq!(
            scope(".box{:where(&) .x{color:red}}"),
            ".box:where(.astro-xxxxxx) {\n  :where(&) .x {\n    color: red;\n  }\n}\n"
        );
    }

    #[test]
    fn test_nesting_inside_not_pseudo() {
        assert_eq!(
            scope(".box{:not(&){color:red}}"),
            ".box:where(.astro-xxxxxx) {\n  :not(&) {\n    color: red;\n  }\n}\n"
        );
    }

    #[test]
    fn test_nesting_inside_has_pseudo_only() {
        // `:has(&)` has no top-level `&` to scope against — leave the rule alone.
        assert_eq!(
            scope(".box{:has(&){color:red}}"),
            ".box:where(.astro-xxxxxx) {\n  :has(&) {\n    color: red;\n  }\n}\n"
        );
    }

    #[test]
    fn test_issue_39_nested_pseudo_element_under_descendant() {
        // Regression for withastro/compiler-rs#39: exact CSS reported in the bug.
        // `&::marker`, `&::before`, `&::after` nested under a descendant (`li` is a
        // descendant of `nav` and intentionally unscoped) must not have scope injected.
        let css = r#"nav {
	ul {
		row-gap: 0;
		margin: 0;
	}

	li {
		position: relative;

		&::marker {
			content: "";
		}

		&:has(> a:hover) {
			&::before, &::after {
				border-color: var(--accent);
			}
		}
	}
}"#;
        assert_eq!(
            scope(css),
            "nav:where(.astro-xxxxxx) {\n  & ul {\n    row-gap: 0;\n    margin: 0;\n  }\n\n  & li {\n    position: relative;\n\n    &::marker {\n      content: \"\";\n    }\n\n    &:has( > a:hover) {\n      &:before, &:after {\n        border-color: var(--accent);\n      }\n    }\n  }\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_child_universal() {
        assert_eq!(
            scope("article :global(> *){}"),
            "article:where(.astro-xxxxxx) > * {\n}\n"
        );
        assert_eq!(
            scope_attribute("article :global(> *){}"),
            "article[data-astro-cid-xxxxxx] > * {\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_child_with_not() {
        assert_eq!(
            scope(".panel :global(> .item:not(.hidden)){}"),
            ".panel:where(.astro-xxxxxx) > .item:not(.hidden) {\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_child_with_has() {
        assert_eq!(
            scope(".panel :global(> li:has(> a)){}"),
            ".panel:where(.astro-xxxxxx) > li:has( > a) {\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_child_with_nested_pseudos() {
        assert_eq!(
            scope(".panel :global(> .nav:not(.is-active):has(.icon)){}"),
            ".panel:where(.astro-xxxxxx) > .nav:not(.is-active):has(.icon) {\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_child_with_quoted_attribute() {
        assert_eq!(
            scope(".panel :global(> [data-state=\"(open)\"]){}"),
            ".panel:where(.astro-xxxxxx) > [data-state=\"(open)\"] {\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_sibling_combinators() {
        assert_eq!(
            scope(".a :global(+ .next){}"),
            ".a:where(.astro-xxxxxx) + .next {\n}\n"
        );
        assert_eq!(
            scope(".a :global(~ .later){}"),
            ".a:where(.astro-xxxxxx) ~ .later {\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_child_multiple_selectors_in_stylesheet() {
        let output = scope(".one :global(> *){} .two :global(+ .x){} .three :global(~ .y){}");
        assert!(
            output.contains(".one:where(.astro-xxxxxx) > *"),
            "missing child universal rule: {output}"
        );
        assert!(
            output.contains(".two:where(.astro-xxxxxx) + .x"),
            "missing adjacent sibling rule: {output}"
        );
        assert!(
            output.contains(".three:where(.astro-xxxxxx) ~ .y"),
            "missing general sibling rule: {output}"
        );
    }

    #[test]
    fn test_global_immediate_child_inside_media_query() {
        assert_eq!(
            scope("@media (min-width: 640px) { .panel :global(> *) {} }"),
            "@media (width >= 640px) {\n  .panel:where(.astro-xxxxxx) > * {\n  }\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_child_does_not_rewrite_inner_descendant_combinator() {
        // Combinator inside :global, not leading — must stay unchanged by rewrite
        // and still scope the local prefix only.
        assert_eq!(
            scope(".class :global(.foo > .bar){}"),
            ".class:where(.astro-xxxxxx) .foo > .bar {\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_child_with_functional_pseudo() {
        assert_eq!(
            scope(".list :global(> li:nth-child(odd)){}"),
            ".list:where(.astro-xxxxxx) > li:nth-child(odd) {\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_child_with_is_and_where() {
        assert_eq!(
            scope(".panel :global(> .item:is(.a, .b)){}"),
            ".panel:where(.astro-xxxxxx) > .item:is(.a, .b) {\n}\n"
        );
        assert_eq!(
            scope(".panel :global(> .item:where(.a, .b)){}"),
            ".panel:where(.astro-xxxxxx) > .item:where(.a, .b) {\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_child_deeply_nested_pseudos() {
        assert_eq!(
            scope(".panel :global(> .item:not(:has(> a))){}"),
            ".panel:where(.astro-xxxxxx) > .item:not(:has( > a)) {\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_child_inner_adjacent_sibling() {
        assert_eq!(
            scope(".panel :global(> .a + .b){}"),
            ".panel:where(.astro-xxxxxx) > .a + .b {\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_child_single_quoted_attribute() {
        assert_eq!(
            scope(".panel :global(> [data-x='(open)']){}"),
            ".panel:where(.astro-xxxxxx) > [data-x=\"(open)\"] {\n}\n"
        );
    }

    #[test]
    fn test_global_attribute_with_escaped_quote_round_trips() {
        // The argument tokens are re-serialized and re-parsed; an embedded quote must be
        // escaped back so the inner selector stays valid rather than being dropped.
        assert_eq!(
            scope(".panel :global(> [data-x=\"a\\\"b\"]){}"),
            ".panel:where(.astro-xxxxxx) > [data-x=\"a\\\"b\"] {\n}\n"
        );
    }

    #[test]
    fn test_global_immediate_child_attribute_strategy_complex() {
        assert_eq!(
            scope_attribute(".panel :global(> .nav:not(.is-active):has(.icon)){}"),
            ".panel[data-astro-cid-xxxxxx] > .nav:not(.is-active):has(.icon) {\n}\n"
        );
    }

    #[test]
    fn test_keyframes_with_selectors() {
        assert_eq!(
            scope(
                "@keyframes shuffle{0%{transform:rotate(0deg);color:blue}100%{transform:rotate(360deg)}} h1{} h2{}"
            ),
            "@keyframes shuffle {\n  0% {\n    transform: rotate(0);\n    color: #00f;\n  }\n\n  100% {\n    transform: rotate(360deg);\n  }\n}\n\nh1:where(.astro-xxxxxx) {\n}\n\nh2:where(.astro-xxxxxx) {\n}\n"
        );
    }

    #[test]
    fn test_starlight_issue() {
        assert_eq!(
            scope(
                ".stagger > :global(*):nth-child(2n) { transform: translateY(var(--stagger-height)); }"
            ),
            ".stagger:where(.astro-xxxxxx) > *:nth-child(2n) {\n  transform: translateY(var(--stagger-height));\n}\n"
        );
        assert_eq!(
            scope(":global(*):nth-child(2n){}"),
            "*:nth-child(2n) {\n}\n"
        );
        assert_eq!(scope(":global(*):not(.skip){}"), "*:not(.skip) {\n}\n");
        assert_eq!(
            scope("@layer starlight.components { .stagger > :global(*):nth-child(2n) {} }"),
            "@layer starlight.components {\n  .stagger:where(.astro-xxxxxx) > *:nth-child(2n) {\n  }\n}\n"
        );
        assert_eq!(
            scope(".stagger > :global(*:nth-child(2n)) {}"),
            ".stagger:where(.astro-xxxxxx) > *:nth-child(2n) {\n}\n"
        );
    }

    #[test]
    fn test_global_with_trailing_pseudo_stays_global() {
        assert_eq!(
            scope(":global(.host):has(> .child){}"),
            ".host:has( > .child) {\n}\n"
        );
        assert_eq!(
            scope_attribute(":global(.host):has(> .child){}"),
            ".host:has( > .child) {\n}\n"
        );
        assert_eq!(
            scope(":global(.host):has([type=\"search\"]){}"),
            ".host:has([type=\"search\"]) {\n}\n"
        );
        assert_eq!(scope(":global(.foo):hover{}"), ".foo:hover {\n}\n");
        assert_eq!(
            scope(":global(.foo):is(.bar, .baz){}"),
            ".foo:is(.bar, .baz) {\n}\n"
        );

        assert_eq!(scope(":global(main):has(.foo){}"), "main:has(.foo) {\n}\n");
        assert_eq!(
            scope(":global(a):not(.foo):not(.bar){}"),
            "a:not(.foo):not(.bar) {\n}\n"
        );
        assert_eq!(
            scope(":global(tr):nth-of-type(n + 2){}"),
            "tr:nth-of-type(n+2) {\n}\n"
        );
        assert_eq!(
            scope(":global(td):nth-of-type(1){}"),
            "td:first-of-type {\n}\n"
        );

        assert_eq!(
            scope(":global([class*=\"foo-\"]):not(.bar){}"),
            "[class*=\"foo-\"]:not(.bar) {\n}\n"
        );
    }

    #[test]
    fn test_combinator_between_globals_stays_global() {
        assert_eq!(
            scope(":global(br) + :global(strong){}"),
            "br + strong {\n}\n"
        );
    }

    #[test]
    fn test_global_does_not_overglobalize_local_parts() {
        // A class suffixed onto a leading `:global()` attaches to the global compound and
        // stays unscoped; only a local part BEFORE the `:global()` gets scoped.
        assert_eq!(scope(":global(.foo).is-active{}"), ".foo.is-active {\n}\n");
        assert_eq!(
            scope_attribute(":global(.foo).is-active{}"),
            ".foo.is-active {\n}\n"
        );
        assert_eq!(
            scope(":global(.foo):hover .child{}"),
            ".foo:hover .child:where(.astro-xxxxxx) {\n}\n"
        );
        assert_eq!(
            scope_attribute(":global(.foo):hover .child{}"),
            ".foo:hover .child[data-astro-cid-xxxxxx] {\n}\n"
        );
    }

    #[test]
    fn test_global_nested_in_pseudo_class_stays_literal() {
        // Like Go: `:global()` inside another pseudo-class stays literal; the subject is scoped.
        assert_eq!(
            scope(".a:not(:global(.foo)){}"),
            ".a:where(.astro-xxxxxx):not(:global(.foo)) {\n}\n"
        );
        assert_eq!(
            scope(".a:is(:global(.foo), .bar){}"),
            ".a:where(.astro-xxxxxx):is(:global(.foo), .bar) {\n}\n"
        );
        assert_eq!(
            scope(".a:has(:global(.child)){}"),
            ".a:where(.astro-xxxxxx):has(:global(.child)) {\n}\n"
        );
        assert_eq!(
            scope_attribute(".button:not(:global(.disabled)){}"),
            ".button[data-astro-cid-xxxxxx]:not(:global(.disabled)) {\n}\n"
        );
    }

    #[test]
    fn test_global_preserves_author_is_universal() {
        // The `*` workaround must not clobber an author-written `:is(*)`.
        assert_eq!(scope(":global(.x):is(*):hover{}"), ".x:is(*):hover {\n}\n");
        assert_eq!(
            scope(".x:is(*):hover{}"),
            ".x:where(.astro-xxxxxx):is(*):hover {\n}\n"
        );
    }

    #[test]
    fn test_global_with_comment_in_argument() {
        assert_eq!(
            scope(".panel :global(> /* hi */ .item){}"),
            ".panel:where(.astro-xxxxxx) > .item {\n}\n"
        );
    }

    #[test]
    fn test_global_inside_has_stays_literal() {
        // Even a leading combinator inside `:global()` here stays verbatim (not hoisted).
        assert_eq!(
            scope(".a:has(:global(> .child)){}"),
            ".a:where(.astro-xxxxxx):has(:global(> .child)) {\n}\n"
        );
        assert_eq!(
            scope(".a:has(> :global(.child)){}"),
            ".a:where(.astro-xxxxxx):has( > :global(.child)) {\n}\n"
        );
    }

    #[test]
    fn test_nested_global_strips_both_wrappers() {
        // The inner `:global()` survives the outer expansion and is stripped by the printer;
        // the trailing local part still gets scoped, like the non-nested `:global(.x).foo`.
        assert_eq!(scope(":global(:global(.x)){}"), ".x {\n}\n");
        // `.foo` follows the (nested) `:global()`, so like `:global(.foo).is-active` it
        // attaches to the global compound and stays unscoped.
        assert_eq!(scope(":global(:global(.x)).foo{}"), ".x.foo {\n}\n");
    }

    // An attribute or class suffixed onto a leading `:global()` must NOT be scoped — the
    // whole compound stays global.

    #[test]
    fn test_global_attribute_suffix_stays_global() {
        assert_eq!(
            scope(":global(a)[role=\"button\"]{}"),
            "a[role=\"button\"] {\n}\n"
        );
        assert_eq!(
            scope_attribute(":global(a)[role=\"button\"]{}"),
            "a[role=\"button\"] {\n}\n"
        );
    }

    #[test]
    fn test_global_attribute_suffix_with_scoped_ancestor() {
        assert_eq!(
            scope("section > :global(a)[role=\"button\"]{}"),
            "section:where(.astro-xxxxxx) > a[role=\"button\"] {\n}\n"
        );
        assert_eq!(
            scope_attribute("section > :global(a)[role=\"button\"]{}"),
            "section[data-astro-cid-xxxxxx] > a[role=\"button\"] {\n}\n"
        );
    }

    #[test]
    fn test_global_class_suffix_stays_global() {
        assert_eq!(scope(":global(a).cls{}"), "a.cls {\n}\n");
        assert_eq!(scope_attribute(":global(a).cls{}"), "a.cls {\n}\n");
    }

    #[test]
    fn test_global_id_suffix_stays_global() {
        assert_eq!(scope(":global(a)#id{}"), "a#id {\n}\n");
    }

    #[test]
    fn test_global_multiple_suffixes_stay_global() {
        assert_eq!(scope(":global(a).b.c{}"), "a.b.c {\n}\n");
    }

    #[test]
    fn test_leading_local_before_global_with_suffix_scopes() {
        // A local part BEFORE the `:global()` still scopes, and the trailing suffix
        // rides along on the already-scoped compound.
        assert_eq!(
            scope(".local:global(.global).bar{}"),
            ".local:where(.astro-xxxxxx).global.bar {\n}\n"
        );
        assert_eq!(
            scope_attribute(".local:global(.global)[data-x]{}"),
            ".local[data-astro-cid-xxxxxx].global[data-x] {\n}\n"
        );
    }

    // A selector list inside `:global()` must keep every item, with the scope prefix
    // distributed across each.

    #[test]
    fn test_global_selector_list_bare() {
        assert_eq!(scope(":global(ul, ol){}"), "ul, ol {\n}\n");
    }

    #[test]
    fn test_global_selector_list_with_scoped_ancestor() {
        assert_eq!(
            scope(".x :global(ul, ol){}"),
            ".x:where(.astro-xxxxxx) ul, .x:where(.astro-xxxxxx) ol {\n}\n"
        );
        assert_eq!(
            scope_attribute(".x :global(ul, ol){}"),
            ".x[data-astro-cid-xxxxxx] ul, .x[data-astro-cid-xxxxxx] ol {\n}\n"
        );
    }

    #[test]
    fn test_global_selector_list_with_scoped_descendant() {
        assert_eq!(
            scope(".x :global(ul, ol) .y{}"),
            ".x:where(.astro-xxxxxx) ul .y:where(.astro-xxxxxx), .x:where(.astro-xxxxxx) ol .y:where(.astro-xxxxxx) {\n}\n"
        );
    }

    #[test]
    fn test_global_selector_list_with_suffix() {
        // Each distributed item carries the trailing suffix, still unscoped.
        assert_eq!(scope(":global(ul, ol).cls{}"), "ul.cls, ol.cls {\n}\n");
        assert_eq!(
            scope(":global(div, span):hover{}"),
            "div:hover, span:hover {\n}\n"
        );
    }

    #[test]
    fn test_global_selector_list_within_larger_list() {
        assert_eq!(
            scope(".a :global(ul, ol), .b :global(p){}"),
            ".a:where(.astro-xxxxxx) ul, .a:where(.astro-xxxxxx) ol, .b:where(.astro-xxxxxx) p {\n}\n"
        );
    }

    #[test]
    fn test_global_selector_list_cartesian() {
        // Two list globals in one selector expand as a cartesian product.
        assert_eq!(
            scope(":global(a, b) :global(c, d){}"),
            "a c, a d, b c, b d {\n}\n"
        );
    }

    #[test]
    fn test_global_selector_list_is_workaround_unchanged() {
        // `:global(:is(ul, ol))` is a single component, not a selector list, so it is
        // emitted intact with no distribution.
        assert_eq!(
            scope(".x :global(:is(ul, ol)){}"),
            ".x:where(.astro-xxxxxx) :is(ul, ol) {\n}\n"
        );
    }

    #[test]
    fn test_global_selector_list_with_leading_child_combinators() {
        // Each item keeps its own hoisted combinator and the scope prefix is distributed.
        assert_eq!(
            scope(".panel :global(> .a, > .b){}"),
            ".panel:where(.astro-xxxxxx) > .a, .panel:where(.astro-xxxxxx) > .b {\n}\n"
        );
        assert_eq!(
            scope_attribute(".panel :global(> .a, > .b){}"),
            ".panel[data-astro-cid-xxxxxx] > .a, .panel[data-astro-cid-xxxxxx] > .b {\n}\n"
        );
    }

    #[test]
    fn test_global_selector_list_with_leading_sibling_combinators() {
        assert_eq!(
            scope(".panel :global(+ .a, ~ .b){}"),
            ".panel:where(.astro-xxxxxx) + .a, .panel:where(.astro-xxxxxx) ~ .b {\n}\n"
        );
    }

    #[test]
    fn test_global_selector_list_mixed_leading_combinator() {
        // A combinator on only some items still distributes; items without one stay
        // descendants of the scoped ancestor.
        assert_eq!(
            scope(".panel :global(> .a, .b){}"),
            ".panel:where(.astro-xxxxxx) > .a, .panel:where(.astro-xxxxxx) .b {\n}\n"
        );
    }

    #[test]
    fn test_global_selector_list_comma_in_attribute_value() {
        // The top-level comma split must ignore commas inside an attribute value, so a
        // leading-combinator list with a comma in `[data-x="a,b"]` stays a single item.
        assert_eq!(
            scope(".panel :global(> [data-x=\"a,b\"], > .c){}"),
            ".panel:where(.astro-xxxxxx) > [data-x=\"a,b\"], .panel:where(.astro-xxxxxx) > .c {\n}\n"
        );
        assert_eq!(
            scope_attribute(".panel :global(> [data-x=\"a,b\"], > .c){}"),
            ".panel[data-astro-cid-xxxxxx] > [data-x=\"a,b\"], .panel[data-astro-cid-xxxxxx] > .c {\n}\n"
        );
    }

    #[test]
    fn test_global_selector_list_comma_in_nested_pseudo() {
        // The top-level comma split must ignore commas inside `:is(...)` parentheses, so a
        // leading-combinator list keeps `> :is(a, b)` as a single item.
        assert_eq!(
            scope(".panel :global(> :is(a, b), > .c){}"),
            ".panel:where(.astro-xxxxxx) > :is(a, b), .panel:where(.astro-xxxxxx) > .c {\n}\n"
        );
    }

    #[test]
    fn test_global_selector_list_with_leading_local() {
        // A leading local part scopes the compound AND the `:global()` list distributes,
        // so each item carries the scoped leading local.
        assert_eq!(
            scope(".a:global(.x, .y){}"),
            ".a:where(.astro-xxxxxx).x, .a:where(.astro-xxxxxx).y {\n}\n"
        );
        assert_eq!(
            scope_attribute(".a:global(.x, .y){}"),
            ".a[data-astro-cid-xxxxxx].x, .a[data-astro-cid-xxxxxx].y {\n}\n"
        );
    }

    #[test]
    fn test_global_selector_list_trailing_comma() {
        // An empty item from a trailing comma is dropped, leaving just the real item.
        assert_eq!(scope(":global(ul,){}"), "ul {\n}\n");
    }
}
