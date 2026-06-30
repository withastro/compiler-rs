//! Astro code printer.
//!
//! Transforms an `AstroRoot` AST into JavaScript code compatible with the Astro runtime.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_codegen::{Codegen, Context, Gen, GenExpr};
use oxc_data_structures::code_buffer::CodeBuffer;
use oxc_span::{GetSpan, Span};
use oxc_syntax::precedence::Precedence;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::options::CompactMode;
use crate::scanner::{
    AstroScanner, ScanResult, get_jsx_attribute_name, get_jsx_element_name, is_component_name,
    is_custom_element, should_hoist_script,
};
use crate::{SourcemapOption, TransformOptions};
use whitespace::{TextPosition, collapse_html, collapse_jsx};

/// Records whether an AST subtree contains a construct that needs an `async`
/// context: an `await` expression, `for await...of`, or `await using`.
#[derive(Default)]
struct AwaitDetector {
    found: bool,
}

impl<'a> Visit<'a> for AwaitDetector {
    fn visit_await_expression(&mut self, _it: &AwaitExpression<'a>) {
        // One `await` is enough to make the function async; no need to recurse.
        self.found = true;
    }

    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        self.found |= it.r#await;
        if !self.found {
            walk::walk_for_of_statement(self, it);
        }
    }

    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        self.found |= it.kind == VariableDeclarationKind::AwaitUsing;
        if !self.found {
            walk::walk_variable_declaration(self, it);
        }
    }
}

impl AwaitDetector {
    /// Subtree checks used to decide whether a slot callback must be `async`.
    /// Scanning the whole subtree (a superset of the slot's own scope) can
    /// over-mark but never under-mark. An `await` left in a non-`async` callback
    /// would be invalid JS.
    fn found_in_child<'a>(child: &JSXChild<'a>) -> bool {
        let mut detector = Self::default();
        detector.visit_jsx_child(child);
        detector.found
    }

    fn found_in_element<'a>(el: &JSXElement<'a>) -> bool {
        let mut detector = Self::default();
        detector.visit_jsx_element(el);
        detector.found
    }

    fn found_in_children<'a>(children: &[JSXChild<'a>]) -> bool {
        children.iter().any(Self::found_in_child)
    }

    fn found_in_refs<'a>(children: &[&JSXChild<'a>]) -> bool {
        children.iter().copied().any(Self::found_in_child)
    }
}

mod components;
mod elements;
mod escape;
mod expressions;
pub mod result;
mod slots;
mod style;
mod whitespace;

#[cfg(test)]
mod printer_tests;
mod sourcemap_builder;
#[cfg(test)]
mod sourcemap_tests;
mod typescript;

// Re-export public result types at the `printer` level so that `lib.rs`
// can `pub use printer::{...}` without reaching into `result`.
pub use result::{
    HoistedScriptType, TransformResult, TransformResultHoistedScript,
    TransformResultHydratedComponent,
};

// Re-export public style types at the `printer` level.
pub use style::{StyleBlock, extract_styles};

// Bring escape helpers into scope for use inside this file.
use escape::{escape_single_quote, escape_template_literal};

// Bring element helpers into scope for use inside this file.
use elements::is_head_element;

// Bring sourcemap builder into scope.
use sourcemap_builder::AstroSourcemapBuilder;

//
// These free functions wrap `oxc_codegen::Codegen` to convert AST nodes into
// source-text strings.  They replace the repetitive
//     `Codegen::new()` → `print_expr` / `print` → `into_source_text()`
// pattern that was duplicated 10+ times across the printer submodules.

/// Convert an expression AST node to a JavaScript source string.
pub fn expr_to_string(expr: &(impl GenExpr + ?Sized)) -> String {
    let mut codegen = Codegen::new();
    expr.print_expr(
        &mut codegen,
        Precedence::Lowest,
        Context::default().with_typescript(),
    );
    codegen.into_source_text()
}

/// Convert an AST node that implements [`Gen`] (e.g. `Statement`,
/// `VariableDeclaration`, `BindingPattern`) to a JavaScript source string.
pub fn gen_to_string(node: &(impl Gen + ?Sized)) -> String {
    let mut codegen = Codegen::new();
    node.print(&mut codegen, Context::default().with_typescript());
    codegen.into_source_text()
}

/// Runtime function names used in generated code.
mod runtime {
    pub const FRAGMENT: &str = "Fragment";
    pub const RENDER: &str = "$$render";
    pub const CREATE_ASTRO: &str = "$$createAstro";
    pub const CREATE_COMPONENT: &str = "$$createComponent";
    pub const RENDER_COMPONENT: &str = "$$renderComponent";
    pub const RENDER_HEAD: &str = "$$renderHead";
    pub const MAYBE_RENDER_HEAD: &str = "$$maybeRenderHead";
    pub const UNESCAPE_HTML: &str = "$$unescapeHTML";
    pub const RENDER_SLOT: &str = "$$renderSlot";
    pub const MERGE_SLOTS: &str = "$$mergeSlots";
    pub const ADD_ATTRIBUTE: &str = "$$addAttribute";
    pub const SPREAD_ATTRIBUTES: &str = "$$spreadAttributes";
    pub const DEFINE_STYLE_VARS: &str = "$$defineStyleVars";
    pub const DEFINE_SCRIPT_VARS: &str = "$$defineScriptVars";
    pub const RENDER_TRANSITION: &str = "$$renderTransition";
    pub const CREATE_TRANSITION_SCOPE: &str = "$$createTransitionScope";
    pub const RENDER_SCRIPT: &str = "$$renderScript";
    pub const TEMPLATE_ENTER: &str = "$$templateEnter";
    pub const TEMPLATE_EXIT: &str = "$$templateExit";
    pub const CREATE_METADATA: &str = "$$createMetadata";
    pub const RESULT: &str = "$$result";
}

/// Transform an Astro AST into JavaScript code.
///
/// This is the primary entry point for code generation. It runs the scanner
/// (analysis pass) and printer (code generation pass) in sequence, then strips
/// TypeScript syntax from the output.
///
/// ```ignore
/// let allocator = Allocator::default();
/// let ret = Parser::new(&allocator, source, SourceType::astro()).parse_astro();
/// let result = transform(&allocator, source, options, &ret.root);
/// println!("{}", result.code);
/// ```
pub fn transform<'a>(
    allocator: &'a Allocator,
    source_text: &'a str,
    options: TransformOptions,
    root: &'a AstroRoot<'a>,
) -> TransformResult {
    let scan_result = AstroScanner::new(allocator).scan(root);
    let codegen = AstroCodegen::new(allocator, source_text, options, scan_result);
    codegen.build(root)
}

/// Astro code generator.
///
/// Transforms an `AstroRoot` AST into JavaScript code that can be executed
/// by the Astro runtime.
pub struct AstroCodegen<'a> {
    allocator: &'a Allocator,
    options: TransformOptions,
    /// Output buffer
    code: CodeBuffer,
    /// Source text of the original Astro file
    source_text: &'a str,
    /// Result from the scanner pass
    scan_result: ScanResult,
    /// Track if we're inside head element
    in_head: bool,
    /// Track if we've already inserted $$maybeRenderHead or $$renderHead
    render_head_inserted: bool,
    /// Track if we've seen an explicit <head> element (which uses $$renderHead)
    has_explicit_head: bool,
    /// Collected module imports for metadata
    module_imports: Vec<ModuleImport>,
    /// Map of component names to their import specifiers (for client:only resolution)
    component_imports: FxHashMap<String, ComponentImportInfo>,
    /// When true, skip printing slot="..." attributes on elements (used for conditional slots)
    skip_slot_attribute: bool,
    /// Current script index for $$renderScript URLs
    script_index: usize,
    /// Current element nesting depth (0 = root level)
    element_depth: usize,
    /// Counter for generating unique transition hashes
    transition_counter: usize,
    /// Base hash for the source file (computed once)
    source_hash: String,
    /// Sourcemap builder (present when `options.sourcemap` is enabled)
    sourcemap_builder: Option<AstroSourcemapBuilder<'a>>,
    /// Collected CSS strings from extracted `<style>` elements (scoped).
    /// Each entry corresponds to one `<style>` tag.
    extracted_css: Vec<String>,
    /// Whether any style was scoped (i.e., at least one non-global, non-inline style exists).
    has_scoped_styles: bool,
    /// Tracks whether we're inside an element that prevents style hoisting
    /// (svg, noscript, template).
    in_non_hoistable: bool,
    /// Tracks whether we're inside a `{...}` expression. Astro treats an
    /// expression as a `<template>` node, which makes nested `<style>` elements
    /// non-hoistable (rendered inline). Unlike [`Self::in_non_hoistable`], this
    /// does NOT prevent `<script>` hoisting; the Go compiler still hoists
    /// scripts inside expressions.
    in_expression: bool,
    /// Depth counter for raw elements (`<pre>`, `<textarea>`, `<script>`,
    /// `<style>`, `is:raw`, …). When > 0, whitespace collapsing is disabled
    /// for all descendant text nodes.
    raw_element_depth: usize,
    /// Collected `define:vars` expression values from `<style>` elements.
    /// Each entry is the raw JS expression string (e.g., `{color:'green'}`).
    define_vars_values: Vec<String>,
    /// Whether any element has received `$$definedVars` style injection.
    define_vars_injected: bool,
    /// Counter for the current extractable style index during prescan.
    /// Used to look up preprocessed style content from `options.preprocessed_styles`.
    style_extraction_index: usize,
}

/// Information about an imported module for metadata.
#[derive(Debug, Clone)]
struct ModuleImport {
    specifier: String,
    /// Variable name for the namespace import (e.g., `$$module1`)
    namespace_var: String,
    /// Import assertion string (e.g., `{type:"json"}`)
    assertion: String,
}

/// Information about an imported component.
#[derive(Debug, Clone)]
struct ComponentImportInfo {
    /// The import specifier (e.g., `"../components"`)
    specifier: String,
    /// The export name (`"default"` for default imports, otherwise the named export)
    export_name: String,
    /// Whether this is a namespace import (`import * as x`)
    is_namespace: bool,
}

impl<'a> AstroCodegen<'a> {
    /// Create a new Astro codegen instance.
    ///
    /// The `scan_result` must be obtained by running [`AstroScanner`] on the
    /// same `AstroRoot` that will be passed to [`build`](Self::build).
    pub fn new(
        allocator: &'a Allocator,
        source_text: &'a str,
        options: TransformOptions,
        scan_result: ScanResult,
    ) -> Self {
        // Compute base hash for the source file.
        // Use normalizedFilename (like Go's compiler) so that different files
        // with identical content get different scope hashes.  Fall back to
        // source text only when no filename is supplied (i.e. "<stdin>").
        let hash_input = options
            .normalized_filename
            .as_deref()
            .or(options.filename.as_deref())
            .filter(|s| *s != "<stdin>")
            .unwrap_or(source_text);
        let source_hash = {
            let mut hasher = DefaultHasher::new();
            hash_input.hash(&mut hasher);
            Self::to_base32_like(hasher.finish())
        };

        // Initialize sourcemap builder if requested
        let sourcemap_builder = if options.sourcemap.is_enabled() {
            let path = options.filename.as_deref().unwrap_or("<stdin>");
            Some(AstroSourcemapBuilder::new(
                std::path::Path::new(path),
                source_text,
            ))
        } else {
            None
        };

        Self {
            allocator,
            options,
            code: CodeBuffer::default(),
            source_text,
            scan_result,
            in_head: false,
            render_head_inserted: false,
            has_explicit_head: false,
            module_imports: Vec::new(),
            component_imports: FxHashMap::default(),
            skip_slot_attribute: false,
            script_index: 0,
            element_depth: 0,
            transition_counter: 0,
            source_hash,
            sourcemap_builder,
            extracted_css: Vec::new(),
            has_scoped_styles: false,
            in_non_hoistable: false,
            in_expression: false,
            raw_element_depth: 0,
            define_vars_values: Vec::new(),
            define_vars_injected: false,
            style_extraction_index: 0,
        }
    }

    /// Convert a u64 hash to a lowercase alphanumeric string (similar to base32).
    pub(super) fn to_base32_like(hash: u64) -> String {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
        let mut result = String::with_capacity(8);
        let mut h = hash;
        for _ in 0..8 {
            result.push(ALPHABET[(h & 0x1f) as usize] as char);
            h >>= 5;
        }
        result
    }

    /// Resolve a component name (possibly dot-notation like `"Two.someName"`) to
    /// its metadata, looking up the root import name in `component_imports`.
    fn resolve_component_metadata(
        &self,
        component_name: &str,
    ) -> Option<TransformResultHydratedComponent> {
        if let Some(dot_pos) = component_name.find('.') {
            let root = &component_name[..dot_pos];
            let rest = &component_name[dot_pos + 1..];

            let info = self.component_imports.get(root)?;
            let export_name = if info.is_namespace {
                rest.to_string()
            } else if info.export_name == "default" {
                format!("default.{rest}")
            } else {
                component_name.to_string()
            };

            let resolved_path = self.options.resolve_specifier(&info.specifier);
            Some(TransformResultHydratedComponent {
                export_name,
                local_name: component_name.to_string(),
                specifier: info.specifier.clone(),
                resolved_path,
            })
        } else {
            let info = self.component_imports.get(component_name)?;
            let resolved_path = self.options.resolve_specifier(&info.specifier);
            Some(TransformResultHydratedComponent {
                export_name: info.export_name.clone(),
                local_name: component_name.to_string(),
                specifier: info.specifier.clone(),
                resolved_path,
            })
        }
    }

    /// Async prefix for the component wrapper (and `set:*` slots), which enclose
    /// the whole file, so the file-level `has_await` flag is the right signal.
    fn get_async_prefix(&self) -> &'static str {
        Self::async_prefix(self.scan_result.has_await)
    }

    fn async_prefix(has_await: bool) -> &'static str {
        if has_await { "async " } else { "" }
    }

    /// Get the slot callback parameter list.
    fn get_slot_params(&self) -> &'static str {
        if self.options.result_scoped_slot {
            "($$result) => "
        } else {
            "() => "
        }
    }

    fn print(&mut self, s: &str) {
        self.code.print_str(s);
    }

    fn println(&mut self, s: &str) {
        self.code.print_str(s);
        self.code.print_char('\n');
    }

    /// Print the arrow function header: async prefix, params, ` => `.
    /// The caller prints the body.
    fn print_arrow_params(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        if arrow.r#async {
            self.print("async ");
        }
        let needs_parens = arrow.params.items.len() != 1
            || arrow.params.rest.is_some()
            || arrow
                .params
                .items
                .first()
                .map(|p| {
                    !matches!(p.pattern, BindingPattern::BindingIdentifier(_))
                        || p.initializer.is_some()
                        || p.type_annotation.is_some()
                        || p.optional
                })
                .unwrap_or(true);

        if needs_parens {
            self.print("(");
        }
        self.print_formal_parameters(&arrow.params);
        if needs_parens {
            self.print(")");
        }
        self.print(" => ");
    }

    /// Record a sourcemap mapping for a `Span` (uses `span.start`).
    fn add_source_mapping_for_span(&mut self, span: Span) {
        if let Some(ref mut sm) = self.sourcemap_builder {
            sm.add_source_mapping_for_span(self.code.as_bytes(), span);
        }
    }

    /// Print a multi-line string and record a source mapping at the start of
    /// each line so that every intermediate line has a Phase 1 token.
    ///
    /// This is needed for expressions that `oxc_codegen::Codegen` expands
    /// across multiple lines (e.g. `[1, 2, 3]` → three lines).  Without
    /// per-line mappings, Phase 2 composition's `lookup_token` (which only
    /// searches within a single line) returns `None` for those lines and the
    /// mapping is lost.
    fn print_multiline_with_mappings(&mut self, text: &str, span: Span) {
        if span.is_empty() || self.sourcemap_builder.is_none() {
            // No sourcemap — just print the text directly.
            self.print(text);
            return;
        }

        let mut first = true;
        for line in text.split('\n') {
            if !first {
                self.code.print_char('\n');
                // Record a mapping at the start of this new line, pointing
                // back to the expression's original span.  Uses `_force` to
                // bypass the consecutive-position dedup (all lines map to the
                // same original byte offset).
                if let Some(ref mut sm) = self.sourcemap_builder {
                    sm.add_source_mapping_force(self.code.as_bytes(), span.start);
                }
            }
            first = false;
            self.code.print_str(line);
        }
    }

    /// Build the JavaScript output from an Astro AST.
    ///
    /// # Panics
    ///
    /// Panics if the intermediate or final code exceeds `isize::MAX` lines
    /// (impossible in practice for source files).
    pub fn build(mut self, root: &'a AstroRoot<'a>) -> TransformResult {
        self.print_astro_root(root);

        let scripts = self.scan_result.hoisted_scripts.clone();

        // Build public hydrated components from internal representation
        let hydrated_components = self
            .scan_result
            .hydrated_components
            .iter()
            .filter_map(|h| self.resolve_component_metadata(&h.name))
            .collect();

        // Build public client-only components from scanner's full names
        let client_only_components = self
            .scan_result
            .client_only_components
            .iter()
            .filter_map(|h| self.resolve_component_metadata(&h.name))
            .collect();

        // Build public server components from internal representation
        let server_components = self
            .scan_result
            .server_deferred_components
            .iter()
            .filter_map(|h| {
                let mut meta = self.resolve_component_metadata(&h.name)?;
                meta.local_name.clone_from(&h.name);
                Some(meta)
            })
            .collect();

        // propagation is true for transition directives AND server:defer components.
        let propagation = self.scan_result.uses_transitions
            || !self.scan_result.server_deferred_components.is_empty();
        let contains_head = self.has_explicit_head;
        let scope = self.source_hash.clone();
        let intermediate_code = self.code.into_string();
        let phase1_sourcemap = self.sourcemap_builder.take();
        let source_path = self.options.filename.as_deref().unwrap_or("<stdin>");

        // Strip TypeScript from the intermediate code.  When a sourcemap is
        // requested we also ask the stripper to produce an intermediate→final
        // sourcemap (phase2) so we can compose it with the Phase 1 map below.
        let generate_sourcemap = phase1_sourcemap.is_some();
        let (mut code, phase2_map) =
            typescript::strip_typescript(self.allocator, &intermediate_code, generate_sourcemap);

        // Compose Phase 1 (astro codegen) and Phase 2 (TS stripping) sourcemaps.
        // This is independent of stripping: you can strip without a sourcemap,
        // and the compose step is a no-op when no sourcemap was requested.
        let sourcemap = sourcemap_builder::compose_sourcemaps(
            &intermediate_code,
            &code,
            phase2_map,
            phase1_sourcemap,
            source_path,
            self.source_text,
        );

        // Apply sourcemap mode: inline, both, or external.
        let sourcemap_mode = self.options.sourcemap;
        let map = match (sourcemap, sourcemap_mode) {
            (Some(sm), SourcemapOption::Inline) => {
                code.push_str("\n//# sourceMappingURL=");
                code.push_str(&sm.to_data_url());
                String::new()
            }
            (Some(sm), SourcemapOption::Both) => {
                let json = sm.to_json_string();
                code.push_str("\n//# sourceMappingURL=");
                code.push_str(&sm.to_data_url());
                json
            }
            (Some(sm), _) => sm.to_json_string(),
            (None, _) => String::new(),
        };

        let css = std::mem::take(&mut self.extracted_css);

        TransformResult {
            code,
            map,
            scope,
            style_error: Vec::new(),
            diagnostics: Vec::new(),
            css,
            scripts,
            hydrated_components,
            client_only_components,
            server_components,
            contains_head,
            propagation,
        }
    }

    fn print_astro_root(&mut self, root: &'a AstroRoot<'a>) {
        // Pre-scan: extract styles from the template so we know the CSS count
        // before printing. This walks the AST to find <style> elements,
        // extracts their CSS, applies scoping, and stores in self.extracted_css.
        self.prescan_styles(&root.body);

        // 1. Print internal imports
        self.print_internal_imports();

        // 2. Extract and print user imports from frontmatter.
        // CSS imports come AFTER user imports (matching Go compiler order) so that
        // component CSS is ordered before the page's own CSS in the final bundle.
        let (imports, exports, other_statements) =
            self.split_frontmatter(root.frontmatter.as_deref());

        // Print user imports
        for import in &imports {
            self.print_statement(import);
        }

        // Blank line after user imports
        if !imports.is_empty() {
            self.println("");
        }

        // 2b. Print CSS imports (one per extracted style) — after user imports so that
        // component stylesheets imported by user code precede this page's own styles.
        self.print_css_imports();

        // 3. Print namespace imports for modules (for metadata) - skip client:only components
        self.print_namespace_imports();

        // 4. Print metadata export
        if imports.is_empty() {
            self.println("");
        }
        self.print_metadata();

        // 5. Print top-level Astro global if needed
        if self.scan_result.uses_astro_global {
            self.print_top_level_astro();
        }

        // 6. Print hoisted exports (after $$Astro, before component)
        for export_stmt in &exports {
            self.print_statement(export_stmt);
        }

        // 7. Print the component
        let component_name = get_component_name(self.options.filename.as_deref());
        self.print_component_wrapper(&other_statements, &root.body, &component_name);

        // 8. Print default export
        self.println(&format!("export default {component_name};"));
    }

    fn print_internal_imports(&mut self) {
        let url = self.options.get_internal_url().to_string();

        self.println("import {");
        self.println(&format!("  {},", runtime::FRAGMENT));
        self.println(&format!("  render as {},", runtime::RENDER));
        self.println(&format!("  createAstro as {},", runtime::CREATE_ASTRO));
        self.println(&format!(
            "  createComponent as {},",
            runtime::CREATE_COMPONENT
        ));
        self.println(&format!(
            "  renderComponent as {},",
            runtime::RENDER_COMPONENT
        ));
        self.println(&format!("  renderHead as {},", runtime::RENDER_HEAD));
        self.println(&format!(
            "  maybeRenderHead as {},",
            runtime::MAYBE_RENDER_HEAD
        ));
        self.println(&format!("  unescapeHTML as {},", runtime::UNESCAPE_HTML));
        self.println(&format!("  renderSlot as {},", runtime::RENDER_SLOT));
        self.println(&format!("  mergeSlots as {},", runtime::MERGE_SLOTS));
        self.println(&format!("  addAttribute as {},", runtime::ADD_ATTRIBUTE));
        self.println(&format!(
            "  spreadAttributes as {},",
            runtime::SPREAD_ATTRIBUTES
        ));
        self.println(&format!(
            "  defineStyleVars as {},",
            runtime::DEFINE_STYLE_VARS
        ));
        self.println(&format!(
            "  defineScriptVars as {},",
            runtime::DEFINE_SCRIPT_VARS
        ));
        self.println(&format!(
            "  renderTransition as {},",
            runtime::RENDER_TRANSITION
        ));
        self.println(&format!(
            "  createTransitionScope as {},",
            runtime::CREATE_TRANSITION_SCOPE
        ));
        self.println(&format!("  renderScript as {},", runtime::RENDER_SCRIPT));
        if self.scan_result.has_template_element {
            self.println(&format!("  templateEnter as {},", runtime::TEMPLATE_ENTER));
            self.println(&format!("  templateExit as {},", runtime::TEMPLATE_EXIT));
        }
        if !self.options.has_resolve_path() {
            self.println(&format!("  createMetadata as {}", runtime::CREATE_METADATA));
        }
        self.println(&format!("}} from \"{url}\";"));

        if self.scan_result.uses_transitions {
            let url = self
                .options
                .transitions_animation_url
                .as_deref()
                .unwrap_or("transitions.css");
            self.println(&format!("import \"{url}\";"));
        }
    }

    fn print_namespace_imports(&mut self) {
        if self.module_imports.is_empty() || self.options.has_resolve_path() {
            return;
        }

        for i in 0..self.module_imports.len() {
            if self.module_imports[i].assertion == "{}" {
                let line = format!(
                    "import * as {} from \"{}\";",
                    self.module_imports[i].namespace_var, self.module_imports[i].specifier
                );
                self.println(&line);
            } else {
                let line = format!(
                    "import * as {} from \"{}\" assert {};",
                    self.module_imports[i].namespace_var,
                    self.module_imports[i].specifier,
                    self.module_imports[i].assertion
                );
                self.println(&line);
            }
        }
        self.println("");
    }

    fn print_metadata(&mut self) {
        if self.options.has_resolve_path() {
            return;
        }

        // Build modules array
        let modules_str = if self.module_imports.is_empty() {
            "[]".to_string()
        } else {
            let items: Vec<String> = self
                .module_imports
                .iter()
                .map(|m| {
                    format!(
                        "{{ module: {}, specifier: \"{}\", assert: {} }}",
                        m.namespace_var, m.specifier, m.assertion
                    )
                })
                .collect();
            format!("[{}]", items.join(", "))
        };

        // Build hydrated components array
        let hydrated_str = if self.scan_result.hydrated_components.is_empty() {
            "[]".to_string()
        } else {
            let custom_elements: Vec<String> = self
                .scan_result
                .hydrated_components
                .iter()
                .filter(|c| c.is_custom_element)
                .map(|c| format!("\"{}\"", c.name))
                .collect();

            let regular_components: Vec<String> = self
                .scan_result
                .hydrated_components
                .iter()
                .filter(|c| !c.is_custom_element)
                .rev()
                .map(|c| c.name.clone())
                .collect();

            let mut items = custom_elements;
            items.extend(regular_components);
            format!("[{}]", items.join(","))
        };

        // Build client-only components array
        let client_only_str = {
            let scan_client_only = &self.scan_result.client_only_components;
            if scan_client_only.is_empty() {
                "[]".to_string()
            } else {
                let mut seen = FxHashSet::default();
                let mut items = Vec::new();
                for h in scan_client_only {
                    let root = if let Some(dot_pos) = h.name.find('.') {
                        &h.name[..dot_pos]
                    } else {
                        h.name.as_str()
                    };
                    if let Some(info) = self.component_imports.get(root)
                        && seen.insert(info.specifier.clone())
                    {
                        items.push(format!("\"{}\"", info.specifier));
                    }
                }
                format!("[{}]", items.join(", "))
            }
        };

        // Build hydration directives set
        let directives_str = if self.scan_result.hydration_directives.is_empty() {
            "new Set([])".to_string()
        } else {
            let items: Vec<String> = self
                .scan_result
                .hydration_directives
                .iter()
                .map(|s| format!("\"{s}\""))
                .collect();
            format!("new Set([{}])", items.join(", "))
        };

        // Build hoisted scripts array
        let hoisted_str = if self.scan_result.hoisted_scripts.is_empty() {
            "[]".to_string()
        } else {
            let items: Vec<String> = self
                .scan_result
                .hoisted_scripts
                .iter()
                .map(|script| match script.script_type {
                    HoistedScriptType::Inline => {
                        let code = script.code.as_deref().unwrap_or("");
                        let escaped = escape_template_literal(code);
                        format!("{{ type: \"inline\", value: `{escaped}` }}")
                    }
                    HoistedScriptType::External => {
                        let src = script.src.as_deref().unwrap_or("");
                        let escaped = escape_single_quote(src);
                        format!("{{ type: \"external\", src: '{escaped}' }}")
                    }
                })
                .collect();
            if items.is_empty() {
                "[]".to_string()
            } else {
                format!("[{}]", items.join(", "))
            }
        };

        let metadata_url = match &self.options.filename {
            Some(f) => format!("\"{}\"", escape_single_quote(f)),
            None => "import.meta.url".to_string(),
        };

        self.println(&format!(
            "export const $$metadata = {}({}, {{ modules: {}, hydratedComponents: {}, clientOnlyComponents: {}, hydrationDirectives: {}, hoisted: {} }});",
            runtime::CREATE_METADATA,
            metadata_url,
            modules_str,
            hydrated_str,
            client_only_str,
            directives_str,
            hoisted_str
        ));
        self.println("");
    }

    fn print_top_level_astro(&mut self) {
        let astro_global_args = self
            .options
            .astro_global_args
            .as_deref()
            .unwrap_or("\"https://astro.build\"");

        self.println(&format!(
            "const $$Astro = {}({});",
            runtime::CREATE_ASTRO,
            astro_global_args
        ));
        self.println("const Astro = $$Astro;");
    }

    /// Split frontmatter into three categories:
    /// - imports: import declarations (hoisted to top of module)
    /// - exports: export declarations (hoisted after metadata, before component)
    /// - other: regular statements (inside component function)
    fn split_frontmatter<'b>(
        &mut self,
        frontmatter: Option<&'b AstroFrontmatter<'a>>,
    ) -> (
        Vec<&'b Statement<'a>>,
        Vec<&'b Statement<'a>>,
        Vec<&'b Statement<'a>>,
    )
    where
        'a: 'b,
    {
        let mut imports = Vec::new();
        let mut exports = Vec::new();
        let mut other = Vec::new();

        if let Some(fm) = frontmatter {
            let mut module_counter = 1;

            for stmt in &fm.program.body {
                if matches!(
                    stmt,
                    Statement::ExportNamedDeclaration(_)
                        | Statement::ExportDefaultDeclaration(_)
                        | Statement::ExportAllDeclaration(_)
                ) {
                    exports.push(stmt);
                    continue;
                }

                if let Statement::ImportDeclaration(import) = stmt {
                    let source = import.source.value.as_str();

                    if import.import_kind == ImportOrExportKind::Type {
                        imports.push(stmt);
                    } else {
                        if let Some(specifiers) = &import.specifiers {
                            for spec in specifiers {
                                match spec {
                                    ImportDeclarationSpecifier::ImportDefaultSpecifier(
                                        default_spec,
                                    ) => {
                                        let local_name =
                                            default_spec.local.name.as_str().to_string();
                                        self.component_imports.insert(
                                            local_name,
                                            ComponentImportInfo {
                                                specifier: source.to_string(),
                                                export_name: "default".to_string(),
                                                is_namespace: false,
                                            },
                                        );
                                    }
                                    ImportDeclarationSpecifier::ImportSpecifier(named_spec) => {
                                        let local_name = named_spec.local.name.as_str().to_string();
                                        let imported_name =
                                            named_spec.imported.name().as_str().to_string();
                                        self.component_imports.insert(
                                            local_name,
                                            ComponentImportInfo {
                                                specifier: source.to_string(),
                                                export_name: imported_name,
                                                is_namespace: false,
                                            },
                                        );
                                    }
                                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(
                                        ns_spec,
                                    ) => {
                                        let local_name = ns_spec.local.name.as_str().to_string();
                                        self.component_imports.insert(
                                            local_name,
                                            ComponentImportInfo {
                                                specifier: source.to_string(),
                                                export_name: "*".to_string(),
                                                is_namespace: true,
                                            },
                                        );
                                    }
                                }
                            }
                        }

                        // Only skip the import if ALL specifiers are exclusively client:only
                        // (i.e. they appear with client:only but NOT with any other directive).
                        // If a component is used with both client:only and e.g. client:load,
                        // it needs to be imported for SSR rendering.
                        let is_client_only_import = if let Some(specifiers) = &import.specifiers {
                            !specifiers.is_empty()
                                && specifiers.iter().all(|spec| {
                                    let local_name = match spec {
                                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                                            s.local.name.as_str()
                                        }
                                        ImportDeclarationSpecifier::ImportSpecifier(s) => {
                                            s.local.name.as_str()
                                        }
                                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                                            s.local.name.as_str()
                                        }
                                    };
                                    self.scan_result
                                        .client_only_component_names
                                        .contains(local_name)
                                        && !self
                                            .scan_result
                                            .referenced_bindings
                                            .contains(local_name)
                                })
                        } else {
                            false
                        };

                        let is_bare_css_import =
                            import.specifiers.is_none() && is_css_specifier(source);

                        if is_client_only_import {
                            // Client:only component imports are not needed at runtime
                        } else if is_bare_css_import {
                            imports.push(stmt);
                        } else {
                            imports.push(stmt);
                            let namespace_var = format!("$$module{module_counter}");

                            let assertion = if let Some(with_clause) = &import.with_clause {
                                let items: Vec<String> = with_clause
                                    .with_entries
                                    .iter()
                                    .map(|attr| {
                                        let key = match &attr.key {
                                            oxc_ast::ast::ImportAttributeKey::Identifier(id) => {
                                                id.name.as_str().to_string()
                                            }
                                            oxc_ast::ast::ImportAttributeKey::StringLiteral(
                                                lit,
                                            ) => format!("\"{}\"", lit.value.as_str()),
                                        };
                                        format!("{}:\"{}\"", key, attr.value.value.as_str())
                                    })
                                    .collect();
                                format!("{{{}}}", items.join(","))
                            } else {
                                "{}".to_string()
                            };

                            self.module_imports.push(ModuleImport {
                                specifier: source.to_string(),
                                namespace_var,
                                assertion,
                            });
                            module_counter += 1;
                        }
                    }
                } else {
                    other.push(stmt);
                }
            }
        }

        (imports, exports, other)
    }

    fn print_statement(&mut self, stmt: &Statement<'_>) {
        let span = stmt.span();
        let raw = &self.source_text[span.start as usize..span.end as usize];
        let raw = raw.trim_end_matches('\n');

        // First line gets the normal span mapping.
        self.add_source_mapping_for_span(span);

        let mut offset: u32 = 0;
        let mut first = true;
        for line in raw.split('\n') {
            if !first {
                self.code.print_char('\n');
                // Map this line to its actual position in the original source.
                if let Some(ref mut sm) = self.sourcemap_builder {
                    sm.add_source_mapping_force(self.code.as_bytes(), span.start + offset);
                }
            }
            first = false;
            self.code.print_str(line);
            // +1 for the '\n' delimiter between lines.
            // Line is a substring of a Span, which is bounded by u32.
            offset += u32::try_from(line.len()).expect("line length exceeds u32") + 1;
        }
        self.code.print_char('\n');
    }

    fn print_component_wrapper(
        &mut self,
        statements: &[&'a Statement<'a>],
        body: &[JSXChild<'a>],
        component_name: &str,
    ) {
        let async_prefix = self.get_async_prefix();
        self.println(&format!(
            "const {} = {}({}({}, $$props, $$slots) => {{",
            component_name,
            runtime::CREATE_COMPONENT,
            async_prefix,
            runtime::RESULT
        ));

        if self.scan_result.uses_astro_global {
            self.println(&format!(
                "const Astro = {}.createAstro($$props, $$slots);",
                runtime::RESULT
            ));
            self.println(&format!("Astro.self = {component_name};"));
        }

        self.println("");

        for stmt in statements {
            self.print_statement(stmt);
        }

        if !statements.is_empty() {
            self.println("");
        }

        // Emit $$definedVars declaration if define:vars is present on any <style>
        if !self.define_vars_values.is_empty() {
            let joined = self.define_vars_values.join(",");
            self.println(&format!(
                "const $$definedVars = {}([{}]);",
                runtime::DEFINE_STYLE_VARS,
                joined
            ));
        }

        self.print("return ");
        self.print(runtime::RENDER);
        self.print("`");

        if self.needs_maybe_render_head_at_start(body) {
            self.print(&format!(
                "${{{}({})}}",
                runtime::MAYBE_RENDER_HEAD,
                runtime::RESULT
            ));
            self.render_head_inserted = true;
        }

        self.print_jsx_body_children(body);

        self.println("`;");

        let filename_part = match &self.options.filename {
            Some(f) => format!("'{}'", escape_single_quote(f)),
            None => "undefined".to_string(),
        };
        // Head propagation is enabled for both transition directives AND server:defer components.
        // (server:defer does NOT set uses_transitions — that's transition-specific — but still
        // needs the "self" propagation arg so that <head> content is forwarded correctly.)
        let propagation = if self.scan_result.uses_transitions
            || !self.scan_result.server_deferred_components.is_empty()
        {
            "\"self\""
        } else {
            "undefined"
        };
        self.println(&format!("}}, {filename_part}, {propagation});"));
    }

    /// Print JSX children, skipping leading (and in compact mode, trailing)
    /// whitespace-only text nodes.
    fn print_jsx_body_children(&mut self, children: &[JSXChild<'a>]) {
        // Find the index of the first non-whitespace-only non-doctype child
        let first_real_idx = children.iter().position(|c| match c {
            JSXChild::Text(t) => !t.value.trim().is_empty(),
            JSXChild::AstroDoctype(_) => false,
            _ => true,
        });
        let remaining = match first_real_idx {
            Some(i) => &children[i..],
            None => return, // all whitespace / doctypes
        };

        // Also skip trailing whitespace-only text nodes at the template root
        // level (matching Go compiler's TrimTrailingSpace behaviour).
        // Extracted elements (e.g. <style>, hoisted <script>) are also
        // skipped so that whitespace between the last real element and an
        // extracted element doesn't leak into the output.
        let last_real_idx = remaining
            .iter()
            .rposition(|c| match c {
                JSXChild::Text(t) => !t.value.trim().is_empty(),
                JSXChild::AstroDoctype(_) | JSXChild::AstroScript(_) => false,
                _ => !self.is_extracted_child(c),
            })
            .map(|i| i + 1) // exclusive end
            .unwrap_or(remaining.len());
        let remaining = &remaining[..last_real_idx];

        // Print all but the last child normally, then trim trailing whitespace
        // from the last text node — matching Go compiler's TrimTrailingSpace
        // behaviour (the source file may end with a newline after real content).
        if let Some((last, rest)) = remaining.split_last() {
            self.print_jsx_children_compact(rest);
            if let JSXChild::Text(text) = last {
                let trimmed = text.value.trim_end();
                if !trimmed.is_empty() {
                    self.add_source_mapping_for_span(text.span);
                    self.print(&escape_template_literal(trimmed));
                }
            } else {
                self.print_jsx_children_compact(std::slice::from_ref(last));
            }
        }
    }

    /// Check if we need to insert `$$maybeRenderHead` at the start of the template.
    fn needs_maybe_render_head_at_start(&self, body: &[JSXChild<'a>]) -> bool {
        if self.render_head_inserted || self.has_explicit_head {
            return false;
        }

        for child in body {
            match child {
                JSXChild::Text(text) => {
                    if text.value.trim().is_empty() {
                        continue;
                    }
                    return false;
                }
                JSXChild::Element(el) => {
                    let name = get_jsx_element_name(&el.opening_element.name);
                    if name == "html" {
                        return false;
                    }
                    if name == "slot" {
                        return false;
                    }
                    if name == "script"
                        && el
                            .children
                            .iter()
                            .any(|c| matches!(c, JSXChild::AstroScript(_)))
                    {
                        continue;
                    }
                    return !is_component_name(&name)
                        && !is_custom_element(&name)
                        && !is_head_element(&name);
                }
                JSXChild::Fragment(_) | JSXChild::ExpressionContainer(_) => {
                    return false;
                }
                _ => {}
            }
        }
        false
    }

    /// Check if this element needs `$$maybeRenderHead` inserted before it.
    fn needs_render_head(&self, name: &str) -> bool {
        if self.render_head_inserted || self.in_head {
            return false;
        }
        if is_component_name(name) {
            return false;
        }
        if is_custom_element(name) {
            return false;
        }
        if is_head_element(name) {
            return false;
        }
        if name == "body" && self.has_explicit_head {
            return false;
        }
        true
    }

    /// Insert `$$maybeRenderHead` if needed before an HTML element.
    fn maybe_insert_render_head(&mut self, name: &str) {
        if self.needs_render_head(name) {
            self.print(&format!(
                "${{{}({})}}",
                runtime::MAYBE_RENDER_HEAD,
                runtime::RESULT
            ));
            self.render_head_inserted = true;
        }
    }

    /// Dispatch printing a single JSX child node.
    fn print_jsx_child(&mut self, child: &JSXChild<'a>) {
        match child {
            JSXChild::Text(text) => {
                self.add_source_mapping_for_span(text.span);
                self.print(&escape_template_literal(text.value.as_str()));
            }
            JSXChild::Element(el) => {
                self.print_jsx_element(el);
            }
            JSXChild::Fragment(frag) => {
                self.print_jsx_fragment(frag);
            }
            JSXChild::ExpressionContainer(expr) => {
                self.print_jsx_expression_container(expr);
            }
            JSXChild::Spread(spread) => {
                self.print_jsx_spread_child(spread);
            }
            JSXChild::AstroScript(_script) => {
                // AstroScript children are handled at the element level (print_html_element)
                // where we have access to the parent element's spans to derive the content range.
            }
            JSXChild::AstroDoctype(_doctype) => {
                // Doctype is typically stripped in the output
            }
            JSXChild::AstroComment(comment) => {
                self.add_source_mapping_for_span(comment.span);
                self.print("<!--");
                self.print(&escape_template_literal(comment.value.as_str()));
                self.print("-->");
            }
        }
    }

    fn print_jsx_text_with_pos(&mut self, text: &JSXText<'a>, pos: TextPosition) {
        self.add_source_mapping_for_span(text.span);
        let value = text.value.as_str();
        match self.options.compact {
            CompactMode::Disabled => {
                self.print(&escape_template_literal(value));
            }
            CompactMode::Html => {
                let in_raw = self.raw_element_depth > 0;
                let in_insensitive = self.in_head;
                if let Some(collapsed) = collapse_html(value, in_raw, in_insensitive, pos) {
                    self.print(&escape_template_literal(&collapsed));
                }
            }
            CompactMode::Jsx => {
                let in_raw = self.raw_element_depth > 0;
                if let Some(collapsed) = collapse_jsx(value, in_raw) {
                    self.print(&escape_template_literal(&collapsed));
                }
            }
        }
    }

    /// Print JSX children with compact-mode sibling awareness.
    ///
    /// When compact mode is active, this method provides each text node with
    /// context about its siblings (lone child detection, whitespace-insensitive
    /// parent).  When compact is disabled, it falls through to `print_jsx_child`.
    fn print_jsx_children_compact(&mut self, children: &[JSXChild<'a>]) {
        let refs: Vec<&JSXChild<'a>> = children.iter().collect();
        self.print_jsx_children_compact_refs(&refs);
    }

    /// Reference-slice variant of [`Self::print_jsx_children_compact`]: slot
    /// bodies hold their children as `Vec<&JSXChild>`, so they route here to get
    /// the same whitespace collapsing as regular element children.
    fn print_jsx_children_compact_refs(&mut self, children: &[&JSXChild<'a>]) {
        let compact = self.options.compact;

        // Pre-compute which children will be extracted (produce no output),
        // so we can exclude them from whitespace decisions.
        let extracted: Vec<bool> = children
            .iter()
            .map(|&c| self.is_extracted_child(c))
            .collect();

        if compact == CompactMode::Disabled {
            for (i, child) in children.iter().enumerate() {
                if let JSXChild::Text(text) = *child {
                    // In non-compact mode, skip whitespace-only text nodes
                    // that are at the edge of visible content (adjacent only
                    // to extracted elements like <style>).
                    if text.value.as_str().chars().all(|c| c.is_ascii_whitespace())
                        && is_edge_whitespace(children, &extracted, i)
                    {
                        continue;
                    }
                }

                self.print_jsx_child(child);
            }
            return;
        }

        // Count non-ignored siblings for lone-child detection.
        // A text node is "lone" if it's the only visible child.
        // Exclude extracted elements (e.g. <style>) from the count so that
        // whitespace between real content and an extracted element is not
        // preserved as a spurious space.
        let visible_count = children
            .iter()
            .enumerate()
            .filter(|(i, c)| {
                !matches!(c, JSXChild::AstroScript(_) | JSXChild::AstroDoctype(_)) && !extracted[*i]
            })
            .count();

        for (i, child) in children.iter().enumerate() {
            if let JSXChild::Text(text) = *child {
                // Skip whitespace-only text nodes at the edge of visible
                // content (e.g. between the last real element and an
                // extracted <style>).
                if text.value.as_str().chars().all(|c| c.is_ascii_whitespace())
                    && is_edge_whitespace(children, &extracted, i)
                {
                    continue;
                }
                let is_lone = visible_count == 1;
                let pos = TextPosition {
                    is_lone_child: is_lone,
                };
                self.print_jsx_text_with_pos(text, pos);
            } else {
                self.print_jsx_child(child);
            }
        }
    }

    /// Check if a JSX child will be completely removed from output (extracted).
    /// Currently this only covers `<style>` elements that will be extracted
    /// (not `is:inline`, not inside svg/noscript/template).
    /// Note: hoisted `<script>` elements still emit `$$renderScript` calls,
    /// so they are NOT considered fully extracted.
    fn is_extracted_child(&self, child: &JSXChild<'a>) -> bool {
        if let JSXChild::Element(el) = child {
            let name = get_jsx_element_name(&el.opening_element.name);
            name == "style"
                && !self.in_non_hoistable
                && !self.in_expression
                && style::should_extract_style_element(el)
        } else {
            false
        }
    }

    /// Dispatch a JSX element to either component or HTML element printing.
    fn print_jsx_element(&mut self, el: &JSXElement<'a>) {
        let name = get_jsx_element_name(&el.opening_element.name);

        // Skip styles already pulled out by prescan. Those in svg/noscript/template
        // or a `{...}` expression are never extracted, so they fall through inline.
        if name == "style"
            && !self.in_non_hoistable
            && !self.in_expression
            && self.should_extract_style(el)
        {
            // Style was already extracted during prescan — just skip it from template
            return;
        }

        // Handle <script define:vars={...}> — these are rendered inline (not bundled
        // via $$renderScript) regardless of whether is:inline is present or not.
        // - Without type="module": IIFE wrapping  <script>(function(){${$$defineScriptVars({...})}...})();</script>
        // - With type="module": no IIFE (imports are illegal inside functions)
        //   <script type="module">${$$defineScriptVars({...})}...</script>
        if name == "script" {
            let define_vars_expr = el.opening_element.attributes.iter().find_map(|attr| {
                if let JSXAttributeItem::Attribute(attr) = attr
                    && get_jsx_attribute_name(&attr.name) == "define:vars"
                {
                    return match &attr.value {
                        Some(JSXAttributeValue::StringLiteral(lit)) => {
                            Some(format!("'{}'", lit.value.as_str()))
                        }
                        Some(JSXAttributeValue::ExpressionContainer(expr)) => {
                            expr.expression.as_expression().map(expr_to_string)
                        }
                        _ => None,
                    };
                }
                None
            });

            if let Some(define_vars_expr) = define_vars_expr {
                self.add_source_mapping_for_span(el.opening_element.span);

                let is_module = el.opening_element.attributes.iter().any(|attr| {
                    if let JSXAttributeItem::Attribute(attr) = attr {
                        let attr_name = get_jsx_attribute_name(&attr.name);
                        if attr_name == "type"
                            && let Some(JSXAttributeValue::StringLiteral(lit)) = &attr.value
                        {
                            return lit.value == "module";
                        }
                    }
                    false
                });

                // Get script content — either from JSXText children (raw text) or
                // from the raw source span when the child is an AstroScript (oxc
                // parsed the content as JS).
                let has_astro_script = el
                    .children
                    .iter()
                    .any(|c| matches!(c, JSXChild::AstroScript(_)));
                let text_content: String = if has_astro_script {
                    let start = el.opening_element.span.end as usize;
                    let end = el
                        .closing_element
                        .as_ref()
                        .map(|c| c.span.start as usize)
                        .unwrap_or(start);
                    self.source_text[start..end].to_string()
                } else {
                    el.children
                        .iter()
                        .filter_map(|child| {
                            if let JSXChild::Text(t) = child {
                                Some(t.value.as_str())
                            } else {
                                None
                            }
                        })
                        .collect()
                };

                if is_module {
                    // type="module" — no IIFE, just prepend $$defineScriptVars
                    self.print("<script type=\"module\">");
                    self.print("${");
                    self.print(runtime::DEFINE_SCRIPT_VARS);
                    self.print("(");
                    self.print(&define_vars_expr);
                    self.print(")}");
                    self.print(&escape_template_literal(&text_content));
                    self.print("</script>");
                } else {
                    // No type="module" — wrap in IIFE
                    self.print("<script>");
                    self.print("(function(){${");
                    self.print(runtime::DEFINE_SCRIPT_VARS);
                    self.print("(");
                    self.print(&define_vars_expr);
                    self.print(")}");
                    self.print(&escape_template_literal(&text_content));
                    self.print("})();");
                    self.print("</script>");
                }
                return;
            }
        }

        // Handle <script> elements that should be hoisted
        let is_hoisted_script = should_hoist_script(&el.opening_element.attributes)
            && (el
                .children
                .iter()
                .any(|child| matches!(child, JSXChild::AstroScript(_)))
                || el.children.iter().any(|child| {
                    if let JSXChild::Text(text) = child {
                        !text.value.trim().is_empty()
                    } else {
                        false
                    }
                })
                || el.opening_element.attributes.iter().any(|attr| {
                    if let JSXAttributeItem::Attribute(attr) = attr {
                        get_jsx_attribute_name(&attr.name) == "src"
                    } else {
                        false
                    }
                }));

        if name == "script" && !self.in_non_hoistable && is_hoisted_script {
            self.add_source_mapping_for_span(el.opening_element.span);

            let filename = self
                .options
                .filename
                .clone()
                .unwrap_or_else(|| "/src/pages/index.astro".to_string());
            let index = self.script_index;
            self.script_index += 1;

            self.print("${");
            self.print(runtime::RENDER_SCRIPT);
            self.print("(");
            self.print(runtime::RESULT);
            self.print(",\"");
            self.print(&filename);
            self.print("?astro&type=script&index=");
            self.print(&index.to_string());
            self.print("&lang.ts\")}");
            return;
        }

        let is_component = is_component_name(&name);
        let is_custom = is_custom_element(&name);

        self.element_depth += 1;

        if is_component || is_custom {
            self.print_component_element(el, &name);
        } else {
            self.print_html_element(el, &name);
        }

        self.element_depth -= 1;
    }
}

/// Check if a whitespace-only text node at position `idx` is at the edge of
/// visible content — i.e. there are no non-extracted, non-whitespace-only
/// children between it and the start/end of the children list.
///
/// This is used to remove trailing whitespace before extracted `<style>`
/// elements (and leading whitespace after them), which would otherwise
/// render as a spurious space in the HTML output.
fn is_edge_whitespace(children: &[&JSXChild<'_>], extracted: &[bool], idx: usize) -> bool {
    let is_ignorable = |i: usize, c: &JSXChild<'_>| -> bool {
        extracted[i]
            || matches!(c, JSXChild::AstroScript(_) | JSXChild::AstroDoctype(_))
            || matches!(c, JSXChild::Text(t) if t.value.as_str().chars().all(|ch| ch.is_ascii_whitespace()))
    };

    // Check if this text node is at the leading edge: all children before it
    // are ignorable, AND at least one of them is an extracted element.
    let before = &children[..idx];
    let at_leading_edge = !before.is_empty()
        && before.iter().enumerate().all(|(i, c)| is_ignorable(i, c))
        && before.iter().enumerate().any(|(i, _)| extracted[i]);
    if at_leading_edge {
        return true;
    }

    // Check if this text node is at the trailing edge: all children after it
    // are ignorable, AND at least one of them is an extracted element.
    let after = &children[idx + 1..];
    !after.is_empty()
        && after
            .iter()
            .enumerate()
            .all(|(j, c)| is_ignorable(idx + 1 + j, c))
        && after
            .iter()
            .enumerate()
            .any(|(j, _)| extracted[idx + 1 + j])
}

/// Check if an import specifier refers to a CSS file.
fn is_css_specifier(specifier: &str) -> bool {
    matches!(
        specifier.rsplit('.').next(),
        Some("css" | "pcss" | "postcss" | "sass" | "scss" | "styl" | "stylus" | "less")
    )
}

/// Derive the component variable name from the filename.
fn get_component_name(filename: Option<&str>) -> String {
    let Some(filename) = filename else {
        return "$$Component".to_string();
    };
    if filename.is_empty() {
        return "$$Component".to_string();
    }

    let part = filename.rsplit('/').next().unwrap_or("");
    if part.is_empty() {
        return "$$Component".to_string();
    }

    let stem = part.split('.').next().unwrap_or(part);
    if stem.is_empty() {
        return "$$Component".to_string();
    }

    let pascal = stem
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    format!("{upper}{}", chars.as_str())
                }
                None => String::new(),
            }
        })
        .collect::<String>();

    if pascal.is_empty() || pascal == "Astro" {
        return "$$Component".to_string();
    }

    format!("$${pascal}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    pub(super) fn compile_astro_with_options(
        source: &str,
        options: TransformOptions,
    ) -> TransformResult {
        let allocator = Allocator::default();
        let source_type = SourceType::astro();
        let ret = Parser::new(&allocator, source, source_type).parse_astro();
        assert!(ret.errors.is_empty(), "Parse errors: {:?}", ret.errors);

        transform(&allocator, source, options, &ret.root)
    }
}
