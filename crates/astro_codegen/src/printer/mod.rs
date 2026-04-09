//! Astro code printer.
//!
//! Transforms an `AstroRoot` AST into JavaScript code compatible with the Astro runtime.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
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

mod components;
mod elements;
mod escape;
mod expressions;
pub mod result;
mod slots;
mod style;
mod whitespace;

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

    /// Get the async function prefix if needed (`"async "` or `""`).
    fn get_async_prefix(&self) -> &'static str {
        if self.scan_result.has_await {
            "async "
        } else {
            ""
        }
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

    /// Print arrow function parameters including parentheses and the `=>` arrow.
    ///
    /// Prints the async prefix (if applicable), the parameter list (with
    /// optional parentheses for single-identifier params), and ` => `.
    /// The caller is responsible for printing the body.
    fn print_arrow_params(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        if arrow.r#async {
            self.print("async ");
        }
        // Single simple identifier param doesn't need parens, but destructuring patterns do
        let needs_parens = arrow.params.items.len() != 1
            || arrow.params.rest.is_some()
            || !matches!(
                arrow.params.items.first().map(|p| &p.pattern),
                Some(BindingPattern::BindingIdentifier(_))
            );

        if needs_parens {
            self.print("(");
        }

        let mut first = true;
        for param in &arrow.params.items {
            if !first {
                self.print(", ");
            }
            first = false;
            self.print_binding_pattern(&param.pattern);
        }
        if let Some(rest) = &arrow.params.rest {
            if !first {
                self.print(", ");
            }
            self.print("...");
            self.print_binding_pattern(&rest.rest.argument);
        }

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
                                    // Must be in client_only_component_names AND NOT in
                                    // hydrated_components (which tracks non-client:only usage)
                                    self.scan_result
                                        .client_only_component_names
                                        .contains(local_name)
                                        && !self
                                            .scan_result
                                            .hydrated_components
                                            .iter()
                                            .any(|c| c.name == local_name)
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
        let last_real_idx = remaining
            .iter()
            .rposition(|c| match c {
                JSXChild::Text(t) => !t.value.trim().is_empty(),
                JSXChild::AstroDoctype(_) | JSXChild::AstroScript(_) => false,
                _ => true,
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
        let compact = self.options.compact;

        // Pre-compute which children will be extracted (produce no output),
        // so we can exclude them from whitespace decisions.
        let extracted: Vec<bool> = children
            .iter()
            .map(|c| self.is_extracted_child(c))
            .collect();

        if compact == CompactMode::Disabled {
            for (i, child) in children.iter().enumerate() {
                if let JSXChild::Text(text) = child {
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
            if let JSXChild::Text(text) = child {
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
    fn is_extracted_child(&self, child: &JSXChild<'a>) -> bool {
        if let JSXChild::Element(el) = child {
            let name = get_jsx_element_name(&el.opening_element.name);
            name == "style" && !self.in_non_hoistable && style::should_extract_style_element(el)
        } else {
            false
        }
    }

    /// Dispatch a JSX element to either component or HTML element printing.
    fn print_jsx_element(&mut self, el: &JSXElement<'a>) {
        let name = get_jsx_element_name(&el.opening_element.name);

        // Handle <style> elements — extract CSS, skip from template output
        // (only if not inside svg/noscript/template)
        if name == "style" && !self.in_non_hoistable && self.should_extract_style(el) {
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
fn is_edge_whitespace(children: &[JSXChild<'_>], extracted: &[bool], idx: usize) -> bool {
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

    fn compile_astro(source: &str) -> String {
        compile_astro_with_options(
            source,
            TransformOptions::new().with_internal_url("http://localhost:3000/"),
        )
        .code
    }

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

    #[test]
    fn test_basic_no_frontmatter() {
        let source = "<button>Click</button>";
        let output = compile_astro(source);

        assert!(output.contains("import {"));
        assert!(output.contains("$$createComponent"));
        assert!(output.contains("<button>Click</button>"));
        assert!(output.contains("export default $$Component"));
    }

    #[test]
    fn test_basic_with_frontmatter() {
        let source = r"---
const href = '/about';
---
<a href={href}>About</a>";
        let output = compile_astro(source);

        assert!(
            output.contains("const href = \"/about\""),
            "Missing const declaration"
        );
        assert!(
            output.contains("$$addAttribute(href, \"href\")"),
            "Missing $$addAttribute"
        );
    }

    #[test]
    fn test_component_rendering() {
        let source = r"---
import Component from 'test';
---
<Component />";
        let output = compile_astro(source);

        assert!(
            output.contains("import Component from \"test\""),
            "Missing import"
        );
        assert!(
            output.contains("$$renderComponent"),
            "Missing $$renderComponent"
        );
        assert!(output.contains("\"Component\""), "Missing component name");
    }

    #[test]
    fn test_doctype() {
        let source = "<!DOCTYPE html><div></div>";
        let output = compile_astro(source);

        assert!(output.contains("<div></div>"), "Missing div element");
        assert!(
            output.contains("$$maybeRenderHead"),
            "Missing maybeRenderHead"
        );
    }

    #[test]
    fn test_fragment() {
        let source = "<><div>1</div><div>2</div></>";
        let output = compile_astro(source);

        assert!(
            output.contains("$$renderComponent"),
            "Missing renderComponent"
        );
        assert!(output.contains("Fragment"), "Missing Fragment reference");
        assert!(output.contains("<div>1</div>"), "Missing first div");
        assert!(output.contains("<div>2</div>"), "Missing second div");
    }

    #[test]
    fn test_html_head_body() {
        let source = r"<html>
  <head>
    <title>Test</title>
  </head>
  <body>
    <h1>Hello</h1>
  </body>
</html>";
        let output = compile_astro(source);

        assert!(
            output.contains("$$renderHead($$result)"),
            "Missing renderHead in head"
        );
        assert!(output.contains("<title>Test</title>"), "Missing title");
        assert!(output.contains("<h1>Hello</h1>"), "Missing h1");
    }

    #[test]
    fn test_expression_in_attribute() {
        let source = r#"---
const src = "image.png";
---
<img src={src} alt="test" />"#;
        let output = compile_astro(source);

        assert!(
            output.contains("$$addAttribute(src, \"src\")"),
            "Missing dynamic src attribute"
        );
        assert!(
            output.contains("alt=\"test\""),
            "Missing static alt attribute"
        );
    }

    #[test]
    fn test_expression_in_content() {
        let source = r#"---
const name = "World";
---
<h1>Hello {name}!</h1>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("Hello ${name}!"),
            "Missing interpolated expression"
        );
    }

    #[test]
    fn test_slots_basic() {
        let source = r#"---
import Component from "test";
---
<Component>
    <div>Default</div>
    <div slot="named">Named</div>
</Component>"#;
        let output = compile_astro(source);

        assert!(output.contains("\"default\":"), "Missing default slot");
        assert!(output.contains("\"named\":"), "Missing named slot");
    }

    #[test]
    fn test_conditional_slot() {
        let source = r#"---
import Component from "test";
---
<Component>{value && <div slot="test">foo</div>}</Component>"#;
        let output = compile_astro(source);

        assert!(output.contains("\"test\":"), "Missing named slot 'test'");
        assert!(
            !output.contains("slot=\"test\""),
            "Slot attribute should be removed from element"
        );
        assert!(
            output.contains("<div>foo</div>"),
            "Missing div element without slot attr"
        );
    }

    #[test]
    fn test_expression_slot_multiple() {
        let source = r#"---
import Component from "test";
---
<Component>{true && <div slot="a">A</div>}{false && <div slot="b">B</div>}</Component>"#;
        let output = compile_astro(source);

        assert!(output.contains("\"a\":"), "Missing named slot 'a'");
        assert!(output.contains("\"b\":"), "Missing named slot 'b'");
        assert!(
            !output.contains("\"default\":"),
            "Should not have default slot"
        );
    }

    #[test]
    fn test_client_load_directive() {
        let source = r"---
import Component from 'test';
---
<Component client:load />";
        let output = compile_astro(source);

        assert!(
            output.contains("client:component-hydration") && output.contains("load"),
            "Missing hydration directive"
        );
    }

    #[test]
    fn test_void_elements() {
        let source = r#"<meta charset="utf-8"><input type="text"><br><img src="x.png"><link rel="stylesheet" href="style.css"><hr>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("<meta charset=\"utf-8\">"),
            "Missing meta tag"
        );
        assert!(
            output.contains("<input type=\"text\">"),
            "Missing input tag"
        );
        assert!(output.contains("<br>"), "Missing br tag");
        assert!(output.contains("<img"), "Missing img tag");
        assert!(output.contains("<link"), "Missing link tag");
        assert!(output.contains("<hr>"), "Missing hr tag");

        assert!(
            !output.contains("</meta>"),
            "Found </meta> - void elements should not have closing tags"
        );
        assert!(
            !output.contains("</input>"),
            "Found </input> - void elements should not have closing tags"
        );
        assert!(
            !output.contains("</br>"),
            "Found </br> - void elements should not have closing tags"
        );
        assert!(
            !output.contains("</img>"),
            "Found </img> - void elements should not have closing tags"
        );
        assert!(
            !output.contains("</link>"),
            "Found </link> - void elements should not have closing tags"
        );
        assert!(
            !output.contains("</hr>"),
            "Found </hr> - void elements should not have closing tags"
        );
    }

    #[test]
    fn test_no_maybe_render_head_with_explicit_head() {
        let source = r"<html>
  <head>
    <title>Test</title>
  </head>
  <body>
    <main>
      <h1>Hello</h1>
    </main>
  </body>
</html>";
        let output = compile_astro(source);

        assert!(
            output.contains("$$renderHead($$result)"),
            "Missing $$renderHead in head"
        );

        assert!(
            !output.contains("$$maybeRenderHead("),
            "Body should not have $$maybeRenderHead when explicit <head> exists"
        );
    }

    #[test]
    fn test_head_elements_skip_maybe_render_head() {
        let source = r#"<Component /><link href="style.css"><meta charset="utf-8"><script src="app.js"></script>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("<link href=\"style.css\">"),
            "Missing link element"
        );
        assert!(
            output.contains("<meta charset=\"utf-8\">"),
            "Missing meta element"
        );

        assert!(
            !output.contains("$$maybeRenderHead("),
            "Head elements should not trigger $$maybeRenderHead"
        );
    }

    #[test]
    fn test_custom_element() {
        let source = r#"<my-element foo="bar"></my-element>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("$$renderComponent"),
            "Custom elements should use $$renderComponent"
        );
        assert!(
            output.contains("\"my-element\"") && output.matches("\"my-element\"").count() >= 2,
            "Custom element should have tag name as both display name and quoted identifier"
        );

        assert!(
            !output.contains("$$maybeRenderHead("),
            "Custom elements should not trigger $$maybeRenderHead"
        );

        assert!(
            !output.contains("<my-element"),
            "Custom elements should not be rendered as HTML tags"
        );
    }

    #[test]
    fn test_html_comments_preserved() {
        let source = r#"<!-- Global Metadata -->
<meta charset="utf-8">
<!-- Another comment -->
<link rel="icon" href="/favicon.ico" />"#;
        let output = compile_astro(source);

        assert!(
            output.contains("<!-- Global Metadata -->"),
            "Missing first HTML comment"
        );
        assert!(
            output.contains("<!-- Another comment -->"),
            "Missing second HTML comment"
        );
        assert!(
            output.contains("<meta charset=\"utf-8\">"),
            "Missing meta tag"
        );
        assert!(
            output.contains("<link rel=\"icon\" href=\"/favicon.ico\">"),
            "Missing link tag"
        );
    }

    // === Metadata tests ===

    #[test]
    fn test_hydrated_component_metadata_default_import() {
        let source = r#"---
import One from "../components/one.jsx";
---
<One client:load />"#;
        let result = compile_astro_with_options(
            source,
            TransformOptions::new().with_internal_url("http://localhost:3000/"),
        );

        assert_eq!(result.hydrated_components.len(), 1);
        let c = &result.hydrated_components[0];
        assert_eq!(c.export_name, "default");
        assert_eq!(c.local_name, "One");
        assert_eq!(c.specifier, "../components/one.jsx");
    }

    #[test]
    fn test_hydrated_component_metadata_named_import() {
        let source = r#"---
import { Three } from "../components/three.tsx";
---
<Three client:load />"#;
        let result = compile_astro_with_options(
            source,
            TransformOptions::new().with_internal_url("http://localhost:3000/"),
        );

        assert_eq!(result.hydrated_components.len(), 1);
        let c = &result.hydrated_components[0];
        assert_eq!(c.export_name, "Three");
        assert_eq!(c.local_name, "Three");
        assert_eq!(c.specifier, "../components/three.tsx");
    }

    #[test]
    fn test_hydrated_component_metadata_namespace_dot_notation() {
        let source = r#"---
import * as Two from "../components/two.jsx";
---
<Two.someName client:load />"#;
        let result = compile_astro_with_options(
            source,
            TransformOptions::new().with_internal_url("http://localhost:3000/"),
        );

        assert_eq!(result.hydrated_components.len(), 1);
        let c = &result.hydrated_components[0];
        assert_eq!(c.export_name, "someName");
        assert_eq!(c.local_name, "Two.someName");
        assert_eq!(c.specifier, "../components/two.jsx");
    }

    #[test]
    fn test_hydrated_component_metadata_namespace_deep_dot_notation() {
        let source = r#"---
import * as four from "../components/four.jsx";
---
<four.nested.deep.Component client:load />"#;
        let result = compile_astro_with_options(
            source,
            TransformOptions::new().with_internal_url("http://localhost:3000/"),
        );

        assert_eq!(result.hydrated_components.len(), 1);
        let c = &result.hydrated_components[0];
        assert_eq!(c.export_name, "nested.deep.Component");
        assert_eq!(c.local_name, "four.nested.deep.Component");
        assert_eq!(c.specifier, "../components/four.jsx");
    }

    #[test]
    fn test_client_only_component_metadata() {
        let source = r#"---
import Five from "../components/five.jsx";
---
<Five client:only="react" />"#;
        let result = compile_astro_with_options(
            source,
            TransformOptions::new().with_internal_url("http://localhost:3000/"),
        );

        assert_eq!(result.client_only_components.len(), 1);
        let c = &result.client_only_components[0];
        assert_eq!(c.export_name, "default");
        assert_eq!(c.local_name, "Five");
        assert_eq!(c.specifier, "../components/five.jsx");
    }

    #[test]
    fn test_client_only_component_metadata_named() {
        let source = r#"---
import { Named } from "../components/named.jsx";
---
<Named client:only="react" />"#;
        let result = compile_astro_with_options(
            source,
            TransformOptions::new().with_internal_url("http://localhost:3000/"),
        );

        assert_eq!(result.client_only_components.len(), 1);
        let c = &result.client_only_components[0];
        assert_eq!(c.export_name, "Named");
        assert_eq!(c.local_name, "Named");
        assert_eq!(c.specifier, "../components/named.jsx");
    }

    #[test]
    fn test_client_only_component_metadata_star_export() {
        let source = r#"---
import * as Five from "../components/five.jsx";
---
<Five.someName client:only />"#;
        let result = compile_astro_with_options(
            source,
            TransformOptions::new().with_filename("/users/astro/apps/pacman/src/pages/index.astro"),
        );

        assert_eq!(result.client_only_components.len(), 1);
        let c = &result.client_only_components[0];
        assert_eq!(c.export_name, "someName");
        assert_eq!(c.specifier, "../components/five.jsx");
    }

    #[test]
    fn test_client_only_component_metadata_deep_nested() {
        let source = r#"---
import * as eight from "../components/eight.jsx";
---
<eight.nested.deep.Component client:only />"#;
        let result = compile_astro_with_options(
            source,
            TransformOptions::new().with_filename("/users/astro/apps/pacman/src/pages/index.astro"),
        );

        assert_eq!(result.client_only_components.len(), 1);
        let c = &result.client_only_components[0];
        assert_eq!(c.export_name, "nested.deep.Component");
        assert_eq!(c.specifier, "../components/eight.jsx");
    }

    #[test]
    fn test_server_deferred_component_metadata() {
        let source = r#"---
import Avatar from "../components/Avatar.jsx";
import { Other } from "../components/Other.jsx";
---
<Avatar server:defer />
<Other server:defer />"#;
        let result = compile_astro_with_options(
            source,
            TransformOptions::new().with_internal_url("http://localhost:3000/"),
        );

        assert_eq!(result.server_components.len(), 2);

        let c0 = &result.server_components[0];
        assert_eq!(c0.export_name, "default");
        assert_eq!(c0.local_name, "Avatar");
        assert_eq!(c0.specifier, "../components/Avatar.jsx");

        let c1 = &result.server_components[1];
        assert_eq!(c1.export_name, "Other");
        assert_eq!(c1.local_name, "Other");
        assert_eq!(c1.specifier, "../components/Other.jsx");

        assert!(result.propagation, "server:defer should enable propagation");
    }

    #[test]
    fn test_contains_head_metadata() {
        let source = r"<html>
<head><title>Test</title></head>
<body><p>content</p></body>
</html>";
        let result = compile_astro_with_options(
            source,
            TransformOptions::new().with_internal_url("http://localhost:3000/"),
        );

        assert!(result.contains_head, "Should detect explicit <head>");
    }

    #[test]
    fn test_no_head_metadata() {
        let source = "<p>no head here</p>";
        let result = compile_astro_with_options(
            source,
            TransformOptions::new().with_internal_url("http://localhost:3000/"),
        );

        assert!(
            !result.contains_head,
            "Should not detect <head> when absent"
        );
    }

    #[test]
    fn test_resolve_path_filepath_join_fallback() {
        let source = r#"---
import Counter from "../components/Counter.jsx";
---
<Counter client:load />"#;
        let result = compile_astro_with_options(
            source,
            TransformOptions::new()
                .with_internal_url("http://localhost:3000/")
                .with_filename("src/pages/index.astro"),
        );

        assert_eq!(result.hydrated_components.len(), 1);
        let c = &result.hydrated_components[0];
        assert_eq!(c.specifier, "../components/Counter.jsx");
        assert_eq!(c.resolved_path, "src/components/Counter.jsx");
    }

    #[test]
    fn test_resolve_path_custom_function() {
        let source = r#"---
import Counter from "../components/Counter.jsx";
---
<Counter client:load />"#;
        let result = compile_astro_with_options(
            source,
            TransformOptions::new()
                .with_internal_url("http://localhost:3000/")
                .with_filename("src/pages/index.astro")
                .with_resolve_path(|specifier| format!("/resolved{specifier}")),
        );

        assert_eq!(result.hydrated_components.len(), 1);
        let c = &result.hydrated_components[0];
        assert_eq!(c.resolved_path, "/resolved../components/Counter.jsx");

        assert!(
            !result.code.contains("$$createMetadata"),
            "Should skip $$createMetadata"
        );
        assert!(
            !result.code.contains("$$metadata"),
            "Should skip $$metadata export"
        );
        assert!(
            !result.code.contains("$$module1"),
            "Should skip $$module imports"
        );
    }

    #[test]
    fn test_resolve_path_bare_specifier_fallback() {
        let source = r#"---
import Counter from "some-package";
---
<Counter client:load />"#;
        let result = compile_astro_with_options(
            source,
            TransformOptions::new()
                .with_internal_url("http://localhost:3000/")
                .with_filename("src/pages/index.astro"),
        );

        assert_eq!(result.hydrated_components.len(), 1);
        assert_eq!(result.hydrated_components[0].resolved_path, "some-package");
    }

    #[test]
    fn test_server_defer_skips_attribute() {
        let source = r"---
import Avatar from './Avatar.jsx';
---
<Avatar server:defer />";
        let result = compile_astro_with_options(
            source,
            TransformOptions::new().with_internal_url("http://localhost:3000/"),
        );

        assert!(
            !result.code.contains("\"server:defer\""),
            "server:defer should be stripped from props"
        );
    }

    #[test]
    fn test_typescript_satisfies_stripped() {
        let source = r"---
interface SEOProps { title: string; }
const seo = { title: 'Hello' } satisfies SEOProps;
---
<h1>{seo.title}</h1>";
        let output = compile_astro(source);

        assert!(
            !output.contains("satisfies"),
            "satisfies keyword should be stripped: {output}"
        );
        assert!(
            !output.contains("interface SEOProps"),
            "interface should be stripped: {output}"
        );
        assert!(
            output.contains("title: \"Hello\"") || output.contains("title: 'Hello'"),
            "value expression should remain: {output}"
        );
    }

    #[test]
    fn test_type_only_import_stripped() {
        let source = r"---
import type { Props } from './types';
const x: Props = { title: 'hi' };
---
<h1>{x.title}</h1>";
        let output = compile_astro(source);

        assert!(
            !output.contains("import type"),
            "import type should be stripped: {output}"
        );
    }

    // === Bug #2 regression: semicolons after return in slot-aware statements ===

    #[test]
    fn test_slot_aware_return_has_semicolon() {
        // A ternary in a component child that routes to named slots triggers
        // print_slot_aware_statement → ReturnStatement. The return must end
        // with a semicolon to avoid ASI hazards.
        let source = r#"---
import Component from "test";
---
<Component>{(() => {
  if (condition) {
    return <div slot="a">A</div>;
  }
  return <div slot="b">B</div>;
})()}</Component>"#;
        let output = compile_astro(source);

        // The compiled output should not contain "return " followed by something
        // without a semicolon before the newline. We check that every "return "
        // in the output has a matching ";" on the same statement.
        assert!(
            !output.contains("return \n"),
            "Return statement should not be followed by bare newline (ASI hazard): {output}"
        );
    }

    // === Bug #3 regression: transition:persist-props on HTML elements ===

    #[test]
    fn test_transition_persist_props_html_element() {
        // transition:persist-props on an HTML element should produce a simple
        // rename to data-astro-transition-persist-props, NOT trigger
        // $$createTransitionScope hash generation in the template body.
        let source = r#"<div transition:persist-props="all">content</div>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("data-astro-transition-persist-props"),
            "Should rename transition:persist-props to data-astro-transition-persist-props: {output}"
        );
        // $$createTransitionScope appears in the import, but should NOT appear
        // in the template body (i.e. inside the $$render`` template literal).
        let template_start = output.find("$$render`").unwrap();
        let template_body = &output[template_start..];
        assert!(
            !template_body.contains("$$createTransitionScope("),
            "transition:persist-props should NOT invoke $$createTransitionScope in template: {output}"
        );
    }

    #[test]
    fn test_transition_persist_and_persist_props_together() {
        // When both transition:persist and transition:persist-props are on the
        // same element, persist should get its normal handling and persist-props
        // should be a simple rename.
        let source = r#"<div transition:persist transition:persist-props="all">content</div>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("data-astro-transition-persist"),
            "Should have data-astro-transition-persist: {output}"
        );
        assert!(
            output.contains("data-astro-transition-persist-props"),
            "Should rename transition:persist-props: {output}"
        );
    }

    #[test]
    fn test_transitions_animation_url_option() {
        // When transitionsAnimationURL is provided, the compiler should use it
        // instead of the default "transitions.css" bare specifier.
        let source = r#"<div transition:persist>content</div>"#;
        let result = compile_astro_with_options(
            source,
            TransformOptions::new()
                .with_internal_url("http://localhost:3000/")
                .with_transitions_animation_url("astro/transitions.css"),
        );

        assert!(
            result.code.contains(r#"import "astro/transitions.css";"#),
            "Should use the provided transitionsAnimationURL: {}",
            result.code
        );
        assert!(
            !result.code.contains(r#"import "transitions.css";"#),
            "Should NOT use the default transitions.css when URL is provided: {}",
            result.code
        );
    }

    #[test]
    fn test_transitions_default_url_without_option() {
        // When transitionsAnimationURL is NOT provided, fall back to "transitions.css".
        let source = r#"<div transition:persist>content</div>"#;
        let output = compile_astro(source);

        assert!(
            output.contains(r#"import "transitions.css";"#),
            "Should use default transitions.css when no URL option is provided: {output}"
        );
    }

    // === CSS scope injection tests ===

    #[test]
    fn test_dynamic_class_with_scoped_styles_no_extra_brace() {
        // When a dynamic class expression is used on an element with scoped styles,
        // the output should not have an extra closing brace leaking into HTML.
        // Regression: `class}=""` was produced due to `}}}}` in format string.
        let source = r#"---
const myClass = "foo";
---
<svg class={myClass}><path d="M0 0"/></svg>
<style>svg { color: red; }</style>"#;
        let output = compile_astro(source);

        // The template literal expression should end with `)}` not `)}}`
        assert!(
            !output.contains("}}"),
            "Should not have double closing braces in template expression: {output}"
        );
        // And there should be no `class}` in the output
        assert!(
            !output.contains("class}"),
            "Should not have malformed class}} attribute: {output}"
        );
        // The $$addAttribute call for class should be well-formed
        assert!(
            output.contains(r#", "class")"#),
            "Should have well-formed $$addAttribute call for class: {output}"
        );
    }

    #[test]
    fn test_component_static_class_merged_with_scope() {
        // When a component has a static class="foo" and scoped styles are active,
        // the scope class should be merged into the value: "class":"foo astro-HASH"
        // NOT two separate "class" keys (which would lose the first one in JS).
        let source = r#"---
import Comp from './Comp.astro';
---
<Comp class="hide" />
<style>div { color: red; }</style>"#;
        let output = compile_astro(source);

        // Should have merged class value like "hide astro-XXXX"
        assert!(
            output.contains(r#""hide astro-"#),
            "Component static class should be merged with scope class: {output}"
        );
        // Should NOT have two separate "class" keys
        let class_count = output.matches(r#""class""#).count();
        assert_eq!(
            class_count, 1,
            "Should have exactly one \"class\" key, got {class_count}: {output}"
        );
    }

    #[test]
    fn test_component_dynamic_class_merged_with_scope() {
        // When a component has a dynamic class={expr} and scoped styles are active,
        // the scope class should be merged: "class":(((expr) ?? "") + " astro-HASH")
        let source = r#"---
import Comp from './Comp.astro';
const cls = "foo";
---
<Comp class={cls} />
<style>div { color: red; }</style>"#;
        let output = compile_astro(source);

        // Should have the ?? "" + " astro-" pattern
        assert!(
            output.contains(r#"?? "") + " astro-"#),
            "Component dynamic class should use nullish coalescing with scope class: {output}"
        );
        // Should NOT have two separate "class" keys
        let class_count = output.matches(r#""class""#).count();
        assert_eq!(
            class_count, 1,
            "Should have exactly one \"class\" key, got {class_count}: {output}"
        );
    }

    #[test]
    fn test_component_no_class_gets_scope_class() {
        // When a component has no class attribute and scoped styles are active,
        // a separate "class":"astro-HASH" should be added.
        let source = r#"---
import Comp from './Comp.astro';
---
<Comp variant="primary" />
<style>div { color: red; }</style>"#;
        let output = compile_astro(source);

        assert!(
            output.contains(r#""astro-"#),
            "Component without class should get scope class prop: {output}"
        );
        // Should have exactly one "class" key
        let class_count = output.matches(r#""class""#).count();
        assert_eq!(
            class_count, 1,
            "Should have exactly one \"class\" key, got {class_count}: {output}"
        );
    }

    // === Test 3: Adversarial/edge-case input tests ===

    #[test]
    fn test_template_literal_injection_in_text() {
        // Backticks in text content should be escaped since output
        // is inside a template literal.
        let source = r"<div>some `backtick` text</div>";
        let output = compile_astro(source);

        assert!(
            output.contains("\\`backtick\\`"),
            "Backticks in text should be escaped: {output}"
        );
    }

    #[test]
    fn test_empty_frontmatter() {
        let source = "---\n---\n<p>hello</p>";
        let output = compile_astro(source);

        assert!(
            output.contains("<p>hello</p>"),
            "Should render content after empty frontmatter: {output}"
        );
        assert!(
            output.contains("$$createComponent"),
            "Should still create component wrapper: {output}"
        );
    }

    #[test]
    fn test_empty_frontmatter_no_template() {
        // Edge case: empty frontmatter with no template content
        let source = "---\n---\n";
        let output = compile_astro(source);

        assert!(
            output.contains("$$createComponent"),
            "Should still create component wrapper: {output}"
        );
    }

    #[test]
    fn test_deeply_nested_ternary_in_expression() {
        let source = r#"<div>{a ? b ? c ? "deep" : "d" : "e" : "f"}</div>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("deep"),
            "Should handle deeply nested ternary: {output}"
        );
    }

    #[test]
    fn test_html_entity_in_text_content() {
        // HTML entities in text should be decoded when used in expressions
        let source = "<div>&lt;script&gt;</div>";
        let output = compile_astro(source);

        // The text content should appear in the template literal
        assert!(
            output.contains("&lt;script&gt;") || output.contains("<script>"),
            "Should handle HTML entities in text: {output}"
        );
    }

    #[test]
    fn test_attribute_with_special_characters() {
        let source = r#"<div data-value="a&b<c>d&quot;e"></div>"#;
        let output = compile_astro(source);

        // The attribute should be preserved or properly escaped
        assert!(
            output.contains("data-value="),
            "Should include the attribute: {output}"
        );
    }

    // === Test 5: JSX statement coverage (for/while/try/throw in JSX) ===

    #[test]
    fn test_for_statement_in_jsx_expression() {
        let source = r"<div>{(() => {
  const items = [];
  for (let i = 0; i < 3; i++) {
    items.push(i);
  }
  return items;
})()}</div>";
        let output = compile_astro(source);

        assert!(
            output.contains("for"),
            "Should handle for statement in JSX: {output}"
        );
    }

    #[test]
    fn test_while_statement_in_jsx_expression() {
        let source = r"<div>{(() => {
  let i = 0;
  while (i < 3) {
    i++;
  }
  return i;
})()}</div>";
        let output = compile_astro(source);

        assert!(
            output.contains("while"),
            "Should handle while statement in JSX: {output}"
        );
    }

    #[test]
    fn test_try_catch_in_jsx_expression() {
        let source = r#"<div>{(() => {
  try {
    return riskyCall();
  } catch (e) {
    return "fallback";
  }
})()}</div>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("try") && output.contains("catch"),
            "Should handle try/catch in JSX: {output}"
        );
    }

    #[test]
    fn test_throw_in_jsx_expression() {
        let source = r#"<div>{(() => {
  if (!data) {
    throw new Error("missing");
  }
  return data;
})()}</div>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("throw"),
            "Should handle throw in JSX: {output}"
        );
    }

    // === Test 7: Empty frontmatter variations ===

    #[test]
    fn test_frontmatter_only_comments() {
        let source = "---\n// just a comment\n---\n<p>hi</p>";
        let output = compile_astro(source);

        assert!(
            output.contains("<p>hi</p>"),
            "Should handle comment-only frontmatter: {output}"
        );
    }

    #[test]
    fn test_no_frontmatter_at_all() {
        let source = "<p>no frontmatter</p>";
        let output = compile_astro(source);

        assert!(
            output.contains("<p>no frontmatter</p>"),
            "Should handle missing frontmatter: {output}"
        );
        assert!(
            output.contains("$$createComponent"),
            "Should still create component: {output}"
        );
    }

    // === Spread attributes ===

    #[test]
    fn test_spread_attributes_on_element() {
        let source = r#"---
const props = { class: "foo", id: "bar" };
---
<div {...props}>content</div>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("$$spreadAttributes"),
            "Should use $$spreadAttributes for spread: {output}"
        );
    }

    // === Boolean and valueless attributes ===

    #[test]
    fn test_boolean_attribute() {
        let source = r"<input disabled />";
        let output = compile_astro(source);

        assert!(
            output.contains("disabled"),
            "Should handle boolean attribute: {output}"
        );
    }

    // === set:html and set:text directives ===

    #[test]
    fn test_set_html_directive() {
        let source = r#"---
const html = "<strong>bold</strong>";
---
<div set:html={html} />"#;
        let output = compile_astro(source);

        assert!(
            output.contains("$$unescapeHTML"),
            "Should use $$unescapeHTML for set:html: {output}"
        );
        // set:html should not appear as an attribute
        assert!(
            !output.contains("set:html="),
            "set:html should be stripped from attributes: {output}"
        );
    }

    #[test]
    fn test_set_text_directive() {
        let source = r#"---
const text = "hello <world>";
---
<div set:text={text} />"#;
        let output = compile_astro(source);

        // set:text should not appear as an attribute
        assert!(
            !output.contains("set:text="),
            "set:text should be stripped from attributes: {output}"
        );
    }

    // === Conditional rendering patterns ===

    #[test]
    fn test_logical_and_rendering() {
        let source = r"<div>{show && <p>visible</p>}</div>";
        let output = compile_astro(source);

        assert!(
            output.contains("show") && output.contains("<p>visible</p>"),
            "Should handle logical AND rendering: {output}"
        );
    }

    #[test]
    fn test_ternary_rendering() {
        let source = r"<div>{show ? <p>yes</p> : <p>no</p>}</div>";
        let output = compile_astro(source);

        assert!(
            output.contains("<p>yes</p>") && output.contains("<p>no</p>"),
            "Should handle ternary rendering: {output}"
        );
    }

    // === Map/iteration patterns ===

    #[test]
    fn test_array_map_rendering() {
        let source = r#"---
const items = ["a", "b", "c"];
---
<ul>{items.map(item => <li>{item}</li>)}</ul>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("<li>"),
            "Should handle array map rendering: {output}"
        );
    }

    // === Transition directives ===

    #[test]
    fn test_transition_name_on_element() {
        let source = r#"<div transition:name="fade">content</div>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("$$renderTransition") || output.contains("data-astro-transition-scope"),
            "Should handle transition:name: {output}"
        );
        assert!(
            output.contains("fade"),
            "Should include transition name: {output}"
        );
    }

    #[test]
    fn test_transition_persist_on_element() {
        let source = r"<div transition:persist>content</div>";
        let output = compile_astro(source);

        assert!(
            output.contains("data-astro-transition-persist")
                || output.contains("$$createTransitionScope"),
            "Should handle transition:persist: {output}"
        );
    }

    // === Test 4: Slot analysis tests (through full compilation) ===

    #[test]
    fn test_slot_ternary_multiple_named_slots() {
        // Ternary expression with different slot names on each branch
        // should trigger $$mergeSlots.
        let source = r#"---
import Component from "test";
---
<Component>{cond ? <div slot="a">A</div> : <div slot="b">B</div>}</Component>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("$$mergeSlots"),
            "Ternary with different slot names should use $$mergeSlots: {output}"
        );
    }

    #[test]
    fn test_slot_ternary_same_slot_name() {
        // Ternary where both branches have the SAME slot name
        // should not need $$mergeSlots — it's a single slot.
        let source = r#"---
import Component from "test";
---
<Component>{cond ? <div slot="x">A</div> : <span slot="x">B</span>}</Component>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("\"x\":"),
            "Both branches with same slot name should produce slot 'x': {output}"
        );
    }

    #[test]
    fn test_slot_logical_and_named() {
        // Logical AND with a named slot
        let source = r#"---
import Component from "test";
---
<Component>{show && <div slot="footer">Footer</div>}</Component>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("\"footer\":"),
            "Logical AND with named slot should produce slot: {output}"
        );
    }

    #[test]
    fn test_slot_default_and_named_together() {
        // Mix of default and named slot children
        let source = r#"---
import Component from "test";
---
<Component>
    <p>Default content</p>
    <div slot="header">Header</div>
</Component>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("\"default\":"),
            "Should have default slot: {output}"
        );
        assert!(
            output.contains("\"header\":"),
            "Should have header slot: {output}"
        );
    }

    #[test]
    fn test_slot_no_children() {
        // Component with no children — no slots at all
        let source = r#"---
import Component from "test";
---
<Component />"#;
        let output = compile_astro(source);

        // Should not have any slot definitions
        assert!(
            !output.contains("\"default\":") || output.contains("\"default\": () =>"),
            "Self-closing component should have no slots: {output}"
        );
    }

    #[test]
    fn test_slot_dynamic_slot_attribute() {
        // Dynamic slot name: slot={name}
        let source = r#"---
import Component from "test";
const slotName = "dynamic";
---
<Component><div slot={slotName}>Content</div></Component>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("slotName"),
            "Dynamic slot name should reference the variable: {output}"
        );
    }

    #[test]
    fn test_slot_fragment_children() {
        // Fragment as component child
        let source = r#"---
import Component from "test";
---
<Component><>fragment child</></Component>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("fragment child"),
            "Fragment children should be rendered: {output}"
        );
    }

    // === Test 6: JSXAttributeValue::Element/Fragment paths ===

    #[test]
    fn test_jsx_element_as_attribute_value_on_component() {
        // JSX element as attribute value on a component renders as "[JSX]"
        let source = r#"---
import Comp from "test";
---
<Comp attr=<span>hi</span> />"#;
        let output = compile_astro(source);

        assert!(
            output.contains("[JSX]"),
            "JSX element as component attribute should render as [JSX]: {output}"
        );
    }

    #[test]
    fn test_jsx_fragment_as_attribute_value_on_component() {
        // JSX fragment as attribute value on a component renders as "[Fragment]"
        let source = r#"---
import Comp from "test";
---
<Comp attr=<>fragment</> />"#;
        let output = compile_astro(source);

        assert!(
            output.contains("[Fragment]"),
            "JSX fragment as component attribute should render as [Fragment]: {output}"
        );
    }

    // === Test 8: Dynamic slot names on <slot> element ===

    #[test]
    fn test_slot_element_with_static_name() {
        // <slot name="header" /> should generate a renderSlot call with "header"
        let source = r#"<slot name="header" />"#;
        let output = compile_astro(source);

        assert!(
            output.contains("$$renderSlot") && output.contains("header"),
            "Static <slot name> should use $$renderSlot with 'header': {output}"
        );
    }

    #[test]
    fn test_slot_element_default() {
        // <slot /> without name should use "default"
        let source = "<slot />";
        let output = compile_astro(source);

        assert!(
            output.contains("$$renderSlot") && output.contains("default"),
            "Default <slot /> should use $$renderSlot with 'default': {output}"
        );
    }

    #[test]
    fn test_slot_element_with_dynamic_name() {
        // <slot name={expr} /> should generate a dynamic slot name
        let source = r#"---
const slotName = "dynamic";
---
<slot name={slotName} />"#;
        let output = compile_astro(source);

        assert!(
            output.contains("$$renderSlot"),
            "Dynamic <slot name={{expr}}> should use $$renderSlot: {output}"
        );
        assert!(
            output.contains("slotName"),
            "Dynamic slot name should reference the variable: {output}"
        );
    }

    #[test]
    fn test_slot_element_with_fallback_content() {
        // <slot>fallback</slot> should include fallback content
        let source = "<slot><p>Fallback content</p></slot>";
        let output = compile_astro(source);

        assert!(
            output.contains("$$renderSlot"),
            "Slot with fallback should use $$renderSlot: {output}"
        );
        assert!(
            output.contains("Fallback content"),
            "Fallback content should be present: {output}"
        );
    }

    // -------------------------------------------------------------------------
    // extract_styles tests
    // -------------------------------------------------------------------------

    fn parse_and_extract_styles(source: &str) -> Vec<StyleBlock> {
        let allocator = Allocator::default();
        let source_type = SourceType::astro();
        let ret = Parser::new(&allocator, source, source_type).parse_astro();
        assert!(ret.errors.is_empty(), "Parse errors: {:?}", ret.errors);
        extract_styles(&ret.root)
    }

    #[test]
    fn test_extract_styles_boolean_attr_is_global() {
        // Boolean attribute `is:global` must be present in attrs with an empty-string
        // value. The caller (TS side) uses `'is:global' in attrs` to detect it, so the
        // key must exist — the value being "" vs true is intentional for the Rust path.
        let source = "<style is:global>h1 { color: red; }</style>";
        let blocks = parse_and_extract_styles(source);

        assert_eq!(blocks.len(), 1, "Should extract one style block");
        let attrs = &blocks[0].attrs;
        let is_global = attrs.iter().find(|(k, _)| k == "is:global");
        assert!(
            is_global.is_some(),
            "is:global key must be present in attrs: {attrs:?}"
        );
        assert_eq!(
            is_global.unwrap().1,
            "",
            "Boolean attr value must be empty string for Rust compiler path: {attrs:?}"
        );
        assert_eq!(
            blocks[0].content.trim(),
            "h1 { color: red; }",
            "Style content should be extracted"
        );
    }

    #[test]
    fn test_extract_styles_quoted_attr_lang() {
        // Quoted attribute `lang="scss"` must round-trip with its value preserved.
        let source = r#"<style lang="scss">$color: red;</style>"#;
        let blocks = parse_and_extract_styles(source);

        assert_eq!(blocks.len(), 1, "Should extract one style block");
        let attrs = &blocks[0].attrs;
        let lang = attrs.iter().find(|(k, _)| k == "lang");
        assert!(lang.is_some(), "lang key must be present: {attrs:?}");
        assert_eq!(
            lang.unwrap().1,
            "scss",
            "lang value must be 'scss': {attrs:?}"
        );
    }

    #[test]
    fn test_extract_styles_is_inline_not_extracted() {
        // `<style is:inline>` must NOT be extracted — it's rendered as-is in HTML.
        let source = "<style is:inline>h1 { color: red; }</style>";
        let blocks = parse_and_extract_styles(source);

        assert_eq!(
            blocks.len(),
            0,
            "is:inline style must not be extracted: {blocks:?}"
        );
    }

    #[test]
    fn test_extract_styles_inside_svg_not_extracted() {
        // Styles inside <svg> are non-hoistable and must not be extracted.
        let source = "<svg><style>circle { fill: red; }</style></svg>";
        let blocks = parse_and_extract_styles(source);

        assert_eq!(
            blocks.len(),
            0,
            "Style inside <svg> must not be extracted: {blocks:?}"
        );
    }

    #[test]
    fn test_extract_styles_inside_noscript_not_extracted() {
        let source = "<noscript><style>h1 { color: red; }</style></noscript>";
        let blocks = parse_and_extract_styles(source);

        assert_eq!(
            blocks.len(),
            0,
            "Style inside <noscript> must not be extracted: {blocks:?}"
        );
    }

    #[test]
    fn test_extract_styles_expression_attr_omitted() {
        // Expression attributes like `define:vars={...}` must be omitted from attrs
        // (they can't be serialised to string), matching the Go compiler's GetAttrs.
        let source = "<style define:vars={{ color: 'red' }}>h1 { color: var(--color); }</style>";
        let blocks = parse_and_extract_styles(source);

        assert_eq!(
            blocks.len(),
            1,
            "Style with define:vars should still be extracted"
        );
        let has_define_vars = blocks[0].attrs.iter().any(|(k, _)| k == "define:vars");
        assert!(
            !has_define_vars,
            "Expression attr define:vars must not appear in attrs: {:?}",
            blocks[0].attrs
        );
    }

    #[test]
    fn test_extract_styles_multiple_blocks_sequential_indices() {
        // Multiple <style> blocks must get sequential indices 0, 1, 2 in document order.
        let source = r"<style>a { color: red; }</style>
<div></div>
<style>b { color: blue; }</style>
<style>c { color: green; }</style>";
        let blocks = parse_and_extract_styles(source);

        assert_eq!(
            blocks.len(),
            3,
            "Should extract three style blocks: {blocks:?}"
        );
        assert_eq!(blocks[0].index, 0);
        assert_eq!(blocks[1].index, 1);
        assert_eq!(blocks[2].index, 2);
        assert!(
            blocks[0].content.contains("red"),
            "First block should have red style"
        );
        assert!(
            blocks[1].content.contains("blue"),
            "Second block should have blue style"
        );
        assert!(
            blocks[2].content.contains("green"),
            "Third block should have green style"
        );
    }

    #[test]
    fn test_extract_styles_multiple_attrs() {
        // A style with both boolean and quoted attrs — both must appear.
        let source = r#"<style is:global lang="scss">h1 { color: red; }</style>"#;
        let blocks = parse_and_extract_styles(source);

        assert_eq!(blocks.len(), 1);
        let attrs = &blocks[0].attrs;
        assert!(
            attrs.iter().any(|(k, v)| k == "is:global" && v.is_empty()),
            "is:global must be present with empty value: {attrs:?}"
        );
        assert!(
            attrs.iter().any(|(k, v)| k == "lang" && v == "scss"),
            "lang must be present with value 'scss': {attrs:?}"
        );
    }

    // -------------------------------------------------------------------------
    // HTML element directive stripping tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_global_stripped_from_html_output() {
        // `is:global` is a compile-time directive and must NOT appear in the
        // rendered HTML string output.
        let source = "<style is:global>h1 { color: red; }</style>";
        let output = compile_astro(source);

        assert!(
            !output.contains("is:global"),
            "is:global must be stripped from compiled output: {output}"
        );
    }

    #[test]
    fn test_is_inline_stripped_from_style_output() {
        // `is:inline` is a compile-time directive and must NOT appear in the
        // rendered HTML string output.
        let source = "<style is:inline>h1 { color: red; }</style>";
        let output = compile_astro(source);

        assert!(
            !output.contains("is:inline"),
            "is:inline must be stripped from compiled output: {output}"
        );
    }

    #[test]
    fn test_is_inline_stripped_from_script_output() {
        let source = "<script is:inline>console.log(1)</script>";
        let output = compile_astro(source);

        assert!(
            !output.contains("is:inline"),
            "is:inline must be stripped from script output: {output}"
        );
        assert!(
            output.contains("console.log(1)"),
            "Inline script content should be in output: {output}"
        );
    }

    #[test]
    fn test_define_vars_stripped_from_style_output() {
        // `define:vars` on a style element must not appear as an HTML attribute.
        let source = "<style define:vars={{ color: 'red' }}>h1 { color: var(--color); }</style>";
        let output = compile_astro(source);

        assert!(
            !output.contains("define:vars"),
            "define:vars must be stripped from style HTML output: {output}"
        );
    }

    // -------------------------------------------------------------------------
    // Component boolean attribute tests — must emit `true` in JS props object
    // -------------------------------------------------------------------------

    #[test]
    fn test_component_boolean_attr_emits_true() {
        // A boolean (valueless) attribute on a component element must be emitted
        // as the JS boolean `true` in the props object, matching Go compiler output.
        let source = r"---
import MyComp from 'test';
---
<MyComp disabled />";
        let output = compile_astro(source);

        assert!(
            output.contains("\"disabled\":true") || output.contains("\"disabled\": true"),
            "Boolean component attr must emit true in props: {output}"
        );
    }

    #[test]
    fn test_component_boolean_attr_is_global_emits_true() {
        // Even Astro-namespace boolean attrs on components must emit `true`, not "".
        let source = r"---
import MyComp from 'test';
---
<MyComp is:global />";
        let output = compile_astro(source);

        assert!(
            output.contains("\"is:global\":true") || output.contains("\"is:global\": true"),
            "Boolean is:global on component must emit true in props: {output}"
        );
    }

    #[test]
    fn test_component_quoted_attr_preserves_value() {
        let source = r#"---
import MyComp from 'test';
---
<MyComp class="foo" />"#;
        let output = compile_astro(source);

        assert!(
            output.contains("\"class\":\"foo\"") || output.contains("\"class\": \"foo\""),
            "Quoted component attr must preserve its value: {output}"
        );
    }

    // -------------------------------------------------------------------------
    // should_hoist_script / scanner tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_script_plain_is_hoisted() {
        // A plain `<script>` with no attributes must be hoisted as inline.
        let source = "<script>console.log('hello')</script>";
        let output = compile_astro(source);

        // Hoisted scripts appear in the `scripts` metadata array, not inline in the template.
        assert!(
            output.contains("\"inline\"") || output.contains("type:\"inline\""),
            "Plain script must be hoisted as inline: {output}"
        );
    }

    #[test]
    fn test_script_type_module_is_hoisted() {
        // `<script type="module">` must be hoisted.
        let source = r#"<script type="module">console.log('hello')</script>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("\"inline\"") || output.contains("type:\"inline\""),
            "type=module script must be hoisted as inline: {output}"
        );
    }

    #[test]
    fn test_script_src_only_is_hoisted_external() {
        // `<script src="...">` with ONLY `src` must be hoisted as external.
        let source = r#"<script src="/script.js"></script>"#;
        let output = compile_astro(source);

        assert!(
            output.contains("\"external\"") || output.contains("type:\"external\""),
            "src-only script must be hoisted as external: {output}"
        );
        assert!(
            output.contains("/script.js"),
            "Hoisted external script must preserve src: {output}"
        );
    }

    #[test]
    fn test_script_src_with_extra_attr_not_hoisted() {
        // `<script src="..." type="module">` must NOT be hoisted (extra attrs).
        // It stays as inline HTML in the template string.
        let source = r#"<script src="/script.js" type="module"></script>"#;
        let output = compile_astro(source);

        assert!(
            !output.contains("\"external\""),
            "Script with src+type must not be hoisted as external: {output}"
        );
        // The raw tag should appear in the template string.
        assert!(
            output.contains("/script.js"),
            "Non-hoisted script should still appear in template: {output}"
        );
    }

    #[test]
    fn test_script_is_inline_not_hoisted() {
        // `<script is:inline>` must NEVER be hoisted.
        let source = "<script is:inline>console.log('hello')</script>";
        let output = compile_astro(source);

        assert!(
            !output.contains("type:\"inline\"") && !output.contains("\"inline\":{"),
            "is:inline script must not be hoisted: {output}"
        );
        // Should appear literally in the template.
        assert!(
            output.contains("console.log"),
            "is:inline script content should be in template: {output}"
        );
    }

    #[test]
    fn test_script_inside_html_comment_not_hoisted() {
        // Full no_extra_script_tag fixture content
        let source = r#"<!-- Global Metadata -->
<meta charset="utf-8">
<meta name="viewport" content="width=device-width">

<link rel="icon" type="image/svg+xml" href="/favicon.svg" />
<link rel="alternate icon" type="image/x-icon" href="/favicon.ico" />

<link rel="sitemap" href="/sitemap.xml"/>

<!-- Global CSS -->
<link rel="stylesheet" href="/theme.css" />
<link rel="stylesheet" href="/code.css" />
<link rel="stylesheet" href="/index.css" />

<!-- Preload Fonts -->
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:ital@0;1&display=swap" rel="stylesheet">

<!-- Scrollable a11y code helper -->
<script type="module" src="/make-scrollable-code-focusable.js" />

<!-- This is intentionally inlined to avoid FOUC -->
<script is:inline>
  const root = document.documentElement;
  const theme = localStorage.getItem('theme');
  if (theme === 'dark' || (!theme) && window.matchMedia('(prefers-color-scheme: dark)').matches) {
    root.classList.add('theme-dark');
  } else {
    root.classList.remove('theme-dark');
  }
</script>

<!-- Global site tag (gtag.js) - Google Analytics -->
<!-- <script async src="https://www.googletagmanager.com/gtag/js?id=G-TEL60V1WM9"></script>
<script>
  window.dataLayer = window.dataLayer || [];
  function gtag(){dataLayer.push(arguments);}
  gtag('js', new Date());
  gtag('config', 'G-TEL60V1WM9');
</script> -->"#;

        let output = compile_astro(source);
        // The commented-out scripts should be inside an HTML comment in the template,
        // NOT in hoisted metadata. Verify hoisted is empty.
        assert!(
            output.contains("hoisted: []"),
            "hoisted should be empty: {output}"
        );
        // Verify the HTML comment containing script tags IS rendered in the template
        assert!(
            output.contains("<!-- <script async"),
            "HTML comment with script tags should be rendered: {output}"
        );
    }

    #[test]
    fn test_script_src_with_data_attr_not_hoisted() {
        // `<script src="..." data-foo="bar">` — any extra attr blocks hoisting.
        let source = r#"<script src="/script.js" data-foo="bar"></script>"#;
        let output = compile_astro(source);

        assert!(
            !output.contains("\"external\""),
            "Script with extra data-attr must not be hoisted: {output}"
        );
    }

    // -------------------------------------------------------------------------
    // set:html must always use $$unescapeHTML (including template literals)
    // -------------------------------------------------------------------------

    #[test]
    fn test_set_html_variable_uses_unescape_html() {
        // `set:html={someVar}` must wrap the value in $$unescapeHTML.
        let source = r"---
const html = '<b>bold</b>';
---
<div set:html={html} />";
        let output = compile_astro(source);

        assert!(
            output.contains("$$unescapeHTML"),
            "set:html with variable must use $$unescapeHTML: {output}"
        );
        assert!(
            !output.contains("set:html"),
            "set:html directive must be stripped from output: {output}"
        );
    }

    #[test]
    fn test_set_html_template_literal_uses_unescape_html() {
        // `set:html={`template ${literal}`}` must wrap with $$unescapeHTML.
        // The $$render tagged template escapes HTML by default, so all set:html
        // expressions — including template literals — need $$unescapeHTML to render
        // as raw HTML.
        let source = r#"---
const name = 'world';
---
<div set:html={`Hello <b>${name}</b>`} />"#;
        let output = compile_astro(source);

        assert!(
            output.contains("$$unescapeHTML(`Hello <b>${name}</b>`)"),
            "set:html with template literal must use $$unescapeHTML: {output}"
        );
    }

    #[test]
    fn test_set_html_string_literal_uses_unescape_html() {
        // Even a plain string literal must go through $$unescapeHTML.
        let source = r#"<div set:html={"<b>bold</b>"} />"#;
        let output = compile_astro(source);

        assert!(
            output.contains("$$unescapeHTML"),
            "set:html with string literal must use $$unescapeHTML: {output}"
        );
    }

    #[test]
    fn test_set_text_does_not_use_unescape_html() {
        // `set:text` is the safe (escaped) counterpart — the rendered expression must
        // NOT be wrapped in $$unescapeHTML(). $$unescapeHTML may still appear in the
        // import statement, so we check the template body specifically.
        let source = r"---
const text = '<b>bold</b>';
---
<div set:text={text} />";
        let output = compile_astro(source);

        // The template render string must contain the raw variable, not wrapped.
        assert!(
            output.contains("${text}"),
            "set:text should emit the variable directly: {output}"
        );
        assert!(
            !output.contains("$$unescapeHTML(text)"),
            "set:text must not wrap in $$unescapeHTML(): {output}"
        );
        assert!(
            !output.contains("set:text"),
            "set:text directive must be stripped from output: {output}"
        );
    }

    // -------------------------------------------------------------------------
    // server:defer prop injection tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_server_defer_injects_component_directive() {
        let source = r"---
import MyComp from './MyComp.astro';
---
<MyComp server:defer />";
        let output = compile_astro(source);

        assert!(
            output.contains("server:component-directive"),
            "server:defer must inject server:component-directive: {output}"
        );
    }

    #[test]
    fn test_server_defer_injects_component_path() {
        let source = r"---
import MyComp from './MyComp.astro';
---
<MyComp server:defer />";
        let output = compile_astro(source);

        assert!(
            output.contains("server:component-path"),
            "server:defer must inject server:component-path: {output}"
        );
    }

    #[test]
    fn test_server_defer_injects_component_export() {
        let source = r"---
import MyComp from './MyComp.astro';
---
<MyComp server:defer />";
        let output = compile_astro(source);

        assert!(
            output.contains("server:component-export"),
            "server:defer must inject server:component-export: {output}"
        );
    }

    #[test]
    fn test_server_defer_does_not_set_client_hydration() {
        // server:defer is NOT a client hydration directive — must not emit
        // client:component-hydration or any transition-related props.
        let source = r"---
import MyComp from './MyComp.astro';
---
<MyComp server:defer />";
        let output = compile_astro(source);

        assert!(
            !output.contains("client:component-hydration"),
            "server:defer must not emit client:component-hydration: {output}"
        );
    }

    // -------------------------------------------------------------------------
    // client:only import suppression tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_client_only_import_is_suppressed() {
        // When a component is used exclusively with client:only, its import must be
        // suppressed (the server never needs to load it).
        let source = r"---
import MyComp from 'test';
---
<MyComp client:only='react' />";
        let output = compile_astro(source);

        // The import must be replaced / suppressed — should not appear as a live import.
        assert!(
            !output.contains("import MyComp from \"test\""),
            "client:only import must be suppressed: {output}"
        );
    }

    #[test]
    fn test_non_client_only_import_not_suppressed() {
        // A component used with client:load (not client:only) must keep its import.
        let source = r"---
import MyComp from 'test';
---
<MyComp client:load />";
        let output = compile_astro(source);

        assert!(
            output.contains("import MyComp from \"test\""),
            "client:load import must NOT be suppressed: {output}"
        );
    }

    #[test]
    fn test_mixed_import_not_suppressed_when_also_client_only() {
        // If the SAME import specifier exports one component used normally and another
        // exclusively as client:only, the import must be kept (not suppressed).
        let source = r"---
import { CompA, CompB } from 'test';
---
<CompA />
<CompB client:only='react' />";
        let output = compile_astro(source);

        // CompA is used normally so the import must be preserved.
        assert!(
            output.contains("import") && output.contains("\"test\""),
            "Mixed import must not be suppressed when one specifier is used normally: {output}"
        );
    }

    // -------------------------------------------------------------------------
    // transition:persist tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_transition_persist_with_explicit_value_on_html_element() {
        // transition:persist="form" on an HTML element should use the string value
        // directly as data-astro-transition-persist, not a generated hash.
        let source = r#"<form transition:persist="form"><input /></form>"#;
        let output = compile_astro(source);

        assert!(
            output.contains(r#"data-astro-transition-persist="form""#),
            "transition:persist with explicit value must use that value: {output}"
        );
        // The output must not contain a $$createTransitionScope( call (only the import alias is ok)
        assert!(
            !output.contains("$$createTransitionScope("),
            "transition:persist with explicit value must not call $$createTransitionScope: {output}"
        );
    }

    #[test]
    fn test_transition_persist_with_transition_name_on_html_element() {
        // transition:persist + transition:name="counter" — persist ID should be "counter"
        let source = r#"<div transition:persist transition:name="counter"></div>"#;
        let output = compile_astro(source);

        assert!(
            output.contains(r#"data-astro-transition-persist="counter""#),
            "transition:persist must use transition:name value as persist ID: {output}"
        );
    }

    #[test]
    fn test_transition_persist_with_explicit_value_on_component() {
        // transition:persist="form" on a component should use the string value directly.
        let source = r#"---
import MyComp from './MyComp.astro';
---
<MyComp transition:persist="form" />"#;
        let output = compile_astro(source);

        assert!(
            output.contains(r#""data-astro-transition-persist": "form""#),
            "component transition:persist with explicit value must use that value: {output}"
        );
        assert!(
            !output.contains("$$createTransitionScope("),
            "component transition:persist with explicit value must not call $$createTransitionScope: {output}"
        );
    }

    #[test]
    fn test_transition_persist_with_transition_name_on_component() {
        // transition:persist + transition:name="counter" on a component.
        let source = r#"---
import MyComp from './MyComp.astro';
---
<MyComp transition:persist transition:name="counter" />"#;
        let output = compile_astro(source);

        assert!(
            output.contains(r#""data-astro-transition-persist": "counter""#),
            "component transition:persist must use transition:name value: {output}"
        );
    }

    #[test]
    fn test_array_expression_in_jsx_transforms_children() {
        // JSX elements inside array expressions must be transformed.
        // Previously they were left as raw JSX in output.
        let source = r#"---
import Foo from './Foo.astro';
import Bar from './Bar.astro';
---
<div>{[<Foo/>, <Bar/>]}</div>"#;
        let output = compile_astro(source);

        assert!(
            !output.contains("<Foo/>") && !output.contains("<Bar/>"),
            "JSX elements inside arrays must be transformed: {output}"
        );
        assert!(
            output.contains("$$renderComponent"),
            "JSX elements inside arrays must use $$renderComponent: {output}"
        );
    }

    // -- Expression edge whitespace trimming tests --
    //
    // Implicit JSX fragments inside expressions ({ <el/>... }) should not
    // have whitespace-only text nodes at the leading/trailing edges inside
    // the $$render backtick. Explicit fragments (<>...</>) preserve their
    // whitespace.

    #[test]
    fn test_expression_implicit_fragment_trims_leading_whitespace() {
        // Leading \n\t before <li> should be trimmed
        let source = "<ul>{\n\t<li>one</li>\n}</ul>";
        let output = compile_astro(source);
        // The backtick should open right before <li>, not with leading whitespace
        assert!(
            output.contains("$$render`<li>one</li>"),
            "implicit fragment should trim leading whitespace: {output}"
        );
    }

    #[test]
    fn test_expression_implicit_fragment_trims_trailing_whitespace() {
        // Trailing \n after </li> should be trimmed
        let source = "<ul>{<li>one</li>\n}</ul>";
        let output = compile_astro(source);
        assert!(
            output.contains("<li>one</li>`"),
            "implicit fragment should trim trailing whitespace: {output}"
        );
    }

    #[test]
    fn test_expression_implicit_fragment_preserves_interior_whitespace() {
        // Whitespace between elements should be preserved
        let source = "<ul>{\n\t<li>one</li>\n\t<li>two</li>\n}</ul>";
        let output = compile_astro(source);
        assert!(
            output.contains("<li>one</li>\n\t<li>two</li>"),
            "implicit fragment should preserve interior whitespace: {output}"
        );
    }

    #[test]
    fn test_expression_explicit_fragment_preserves_all_whitespace() {
        // Explicit <>...</> should keep leading/trailing whitespace
        let source = "<div>{<>\n  <span>hi</span>\n</>}</div>";
        let output = compile_astro(source);
        // The whitespace after <> and before </> should be inside the backtick
        assert!(
            output.contains("$$render`\n  <span>hi</span>\n`"),
            "explicit fragment should preserve all whitespace: {output}"
        );
    }

    #[test]
    fn test_expression_implicit_fragment_inline_elements_no_extra_space() {
        // Inline elements shouldn't get extra whitespace injected
        let source = "<span>{\n  <strong>hello</strong>\n  <em>world</em>\n}</span>";
        let output = compile_astro(source);
        // Leading/trailing \n should be trimmed — no extra space at edges
        assert!(
            output.contains("$$render`<strong>hello</strong>"),
            "should not inject leading whitespace before inline element: {output}"
        );
        assert!(
            output.contains("<em>world</em>`"),
            "should not inject trailing whitespace after inline element: {output}"
        );
    }
}
