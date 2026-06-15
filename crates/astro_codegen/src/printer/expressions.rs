//! JavaScript and JSX expression printing.
//!
//! Contains `impl AstroCodegen` methods for rendering JS expressions that may
//! contain JSX. This includes ternary/logical expressions, arrow functions,
//! call expressions, optional chaining, and JSX fragments — any case where
//! JavaScript expressions and JSX are interleaved.

use oxc_ast::ast::*;
use oxc_span::GetSpan;

use super::AstroCodegen;
use super::runtime;
use super::{expr_to_string, gen_to_string};

impl<'a> AstroCodegen<'a> {
    pub(super) fn print_jsx_fragment(&mut self, frag: &JSXFragment<'a>) {
        self.add_source_mapping_for_span(frag.span);
        let async_prefix = Self::async_prefix(Self::children_have_await(&frag.children));
        let slot_params = self.get_slot_params();
        self.print("${");
        self.print(runtime::RENDER_COMPONENT);
        self.print("(");
        self.print(runtime::RESULT);
        self.print(",\"Fragment\",");
        self.print(runtime::FRAGMENT);
        self.print(&format!(",{{}},{{\"default\": {async_prefix}{slot_params}"));
        self.print(runtime::RENDER);
        self.print("`");
        self.print_jsx_children_compact(&frag.children);
        // Map the closing fragment tag (</>) to the `)` that closes
        // $$renderComponent(...) — the semantic equivalent in generated code.
        if !frag.closing_fragment.span.is_empty() {
            self.add_source_mapping_for_span(frag.closing_fragment.span);
        }
        self.print("`,})}");
    }

    pub(super) fn print_jsx_expression_container(&mut self, expr: &JSXExpressionContainer<'a>) {
        self.add_source_mapping_for_span(expr.span);
        // Check if this is a comment-only expression that should be stripped
        if let JSXExpression::EmptyExpression(empty) = &expr.expression {
            let content = &self.source_text[empty.span.start as usize..empty.span.end as usize];
            let trimmed = content.trim();
            // Comment-only expressions should be stripped entirely
            if !trimmed.is_empty() {
                return;
            }
        }

        self.print("${");
        self.print_jsx_expression(&expr.expression);
        self.print("}");
    }

    pub(super) fn print_jsx_expression(&mut self, expr: &JSXExpression<'a>) {
        match expr {
            JSXExpression::EmptyExpression(_) => {
                // Empty {} or whitespace-only {   } renders as (void 0)
                // Comment-only expressions are already filtered out in print_jsx_expression_container
                self.print("(void 0)");
            }
            JSXExpression::Identifier(ident) => {
                self.print(ident.name.as_str());
            }
            other => {
                // Use regular codegen for complex expressions
                if let Some(expr) = other.as_expression() {
                    self.print_expression(expr);
                }
            }
        }
    }

    pub(super) fn print_jsx_spread_child(&mut self, spread: &JSXSpreadChild<'a>) {
        self.add_source_mapping_for_span(spread.span);
        self.print("${");
        self.print_expression(&spread.expression);
        self.print("}");
    }

    pub(super) fn print_expression(&mut self, expr: &Expression<'a>) {
        // Handle expressions that may contain JSX
        // JSX inside expressions needs to be wrapped in $$render`...`
        //
        // NOTE: source mappings are added per-arm rather than unconditionally
        // at the top.  For JSXElement and JSXFragment the `$$render` boilerplate
        // is printed first; mapping the expression span at that position would
        // cause `$$render` to be mapped to the element's source location in
        // the sourcemap (confusing in visualizers).  Instead we let the inner
        // element/fragment printing emit its own mapping at the correct
        // position (after the boilerplate).
        match expr {
            Expression::JSXElement(el) => {
                // $$render is runtime boilerplate — skip mapping here;
                // print_jsx_element will map <tag> to the correct source span.
                self.print(runtime::RENDER);
                self.print("`");
                self.print_jsx_element(el);
                self.print("`");
            }
            Expression::JSXFragment(frag) => {
                // Same rationale as JSXElement — don't map $$render boilerplate.
                // Child printing (print_jsx_child / print_jsx_fragment) will
                // emit the appropriate source mappings.
                let is_explicit_fragment = !frag.opening_fragment.span.is_empty();

                if is_explicit_fragment {
                    // Explicit <>...</> syntax gets wrapped in $$renderComponent with Fragment.
                    // Whitespace inside <>..</> is intentional authored content — preserve it.
                    let async_prefix =
                        Self::async_prefix(Self::children_have_await(&frag.children));
                    let slot_params = self.get_slot_params();
                    self.print(runtime::RENDER);
                    self.print("`${");
                    self.print(runtime::RENDER_COMPONENT);
                    self.print(&format!(
                        "($$result,\"Fragment\",Fragment,{{}},{{\"default\":{async_prefix}{slot_params}"
                    ));
                    self.print(runtime::RENDER);
                    self.print("`");
                    self.print_jsx_children_compact(&frag.children);
                    // Map closing fragment tag (</>) before the closing boilerplate.
                    if !frag.closing_fragment.span.is_empty() {
                        self.add_source_mapping_for_span(frag.closing_fragment.span);
                    }
                    self.print("`,})}`");
                } else {
                    // Implicit fragments (multiple JSX siblings) are just wrapped
                    // in $$render`...`. Edge whitespace-only text nodes are
                    // stripped by the parser, so we render children directly.
                    self.print(runtime::RENDER);
                    self.print("`");
                    self.print_jsx_children_compact(&frag.children);
                    self.print("`");
                }
            }
            Expression::ConditionalExpression(cond) => {
                self.add_source_mapping_for_span(expr.span());
                // Recursively handle ternary: test ? consequent : alternate
                self.print_expression(&cond.test);
                self.print(" ? ");
                self.print_expression(&cond.consequent);
                self.print(" : ");
                self.print_expression(&cond.alternate);
            }
            Expression::LogicalExpression(logic) => {
                self.add_source_mapping_for_span(expr.span());
                // Recursively handle && and ||
                self.print_expression(&logic.left);
                self.print(match logic.operator {
                    oxc_ast::ast::LogicalOperator::And => " && ",
                    oxc_ast::ast::LogicalOperator::Or => " || ",
                    oxc_ast::ast::LogicalOperator::Coalesce => " ?? ",
                });
                self.print_expression(&logic.right);
            }
            Expression::ParenthesizedExpression(paren) => {
                self.add_source_mapping_for_span(expr.span());
                self.print("(");
                self.print_expression(&paren.expression);
                self.print(")");
            }
            Expression::ChainExpression(chain) => {
                self.add_source_mapping_for_span(expr.span());
                // Handle optional chaining like arr?.map(x => <JSX>)
                self.print_chain_expression(chain);
            }
            Expression::CallExpression(call) => {
                self.add_source_mapping_for_span(expr.span());
                // Handle call expressions like arr.map(x => <JSX>)
                self.print_call_expression(call);
            }
            Expression::ArrowFunctionExpression(arrow) => {
                self.add_source_mapping_for_span(expr.span());
                // Handle arrow functions that may return JSX
                self.print_arrow_function(arrow);
            }
            Expression::FunctionExpression(func) => {
                self.add_source_mapping_for_span(expr.span());
                self.print_function(func);
            }
            Expression::ClassExpression(class) => {
                self.add_source_mapping_for_span(expr.span());
                self.print_class(class);
            }
            Expression::YieldExpression(yield_expr) => {
                self.add_source_mapping_for_span(expr.span());
                self.print("yield");
                if yield_expr.delegate {
                    self.print("*");
                }
                if let Some(arg) = &yield_expr.argument {
                    self.print(" ");
                    self.print_expression(arg);
                }
            }
            Expression::AwaitExpression(await_expr) => {
                self.add_source_mapping_for_span(expr.span());
                self.print("await ");
                self.print_expression(&await_expr.argument);
            }
            Expression::ArrayExpression(arr) => {
                self.add_source_mapping_for_span(expr.span());
                // Arrays may contain JSX elements — iterate and transform each element.
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
                            self.print_expression(&spread.argument);
                        }
                        oxc_ast::ast::ArrayExpressionElement::Elision(_) => {
                            // Elision — empty slot, e.g. [1,,3]
                        }
                        _ => {
                            if let Some(e) = element.as_expression() {
                                self.print_expression(e);
                            }
                        }
                    }
                }
                self.print("]");
            }
            Expression::AssignmentExpression(assign) => {
                self.add_source_mapping_for_span(expr.span());
                let left_code = gen_to_string(&assign.left);
                self.print(&left_code);
                self.print(&format!(" {} ", assign.operator.as_str()));
                self.print_expression(&assign.right);
            }
            Expression::SequenceExpression(seq) => {
                self.add_source_mapping_for_span(expr.span());
                for (i, expr) in seq.expressions.iter().enumerate() {
                    if i > 0 {
                        self.print(", ");
                    }
                    self.print_expression(expr);
                }
            }
            Expression::ObjectExpression(obj) => {
                self.add_source_mapping_for_span(expr.span());
                self.print("{");
                let mut first = true;
                for prop in &obj.properties {
                    if !first {
                        self.print(", ");
                    }
                    first = false;
                    match prop {
                        oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) => {
                            if p.computed {
                                self.print("[");
                                self.print_expression(p.key.as_expression().unwrap());
                                self.print("]");
                            } else {
                                let key_code = gen_to_string(&p.key);
                                self.print(&key_code);
                            }
                            if !p.shorthand {
                                self.print(": ");
                                self.print_expression(&p.value);
                            }
                        }
                        oxc_ast::ast::ObjectPropertyKind::SpreadProperty(spread) => {
                            self.print("...");
                            self.print_expression(&spread.argument);
                        }
                    }
                }
                self.print("}");
            }
            Expression::NewExpression(new_expr) => {
                self.add_source_mapping_for_span(expr.span());
                self.print("new ");
                self.print_expression(&new_expr.callee);
                self.print("(");
                let mut first = true;
                for arg in &new_expr.arguments {
                    if !first {
                        self.print(", ");
                    }
                    first = false;
                    match arg {
                        oxc_ast::ast::Argument::SpreadElement(spread) => {
                            self.print("...");
                            self.print_expression(&spread.argument);
                        }
                        _ => {
                            if let Some(e) = arg.as_expression() {
                                self.print_expression(e);
                            }
                        }
                    }
                }
                self.print(")");
            }
            _ => {
                self.add_source_mapping_for_span(expr.span());
                // For all other expressions, use regular codegen
                let code = expr_to_string(expr);
                // Use multiline-aware print so that each line of the expanded
                // expression gets a Phase 1 mapping.  Without this, lines like
                // `1,` / `2,` / `3` from `[1, 2, 3]` would have no Phase 1
                // token and Phase 2 composition would fail (lookup_token only
                // searches within a single line).
                self.print_multiline_with_mappings(&code, expr.span());
            }
        }
    }

    fn print_chain_expression(&mut self, chain: &oxc_ast::ast::ChainExpression<'a>) {
        match &chain.expression {
            oxc_ast::ast::ChainElement::CallExpression(call) => {
                self.print_call_expression(call);
            }
            oxc_ast::ast::ChainElement::StaticMemberExpression(member) => {
                self.print_expression(&member.object);
                self.print(if member.optional { "?." } else { "." });
                self.print(member.property.name.as_str());
            }
            oxc_ast::ast::ChainElement::ComputedMemberExpression(member) => {
                self.print_expression(&member.object);
                self.print(if member.optional { "?.[" } else { "[" });
                self.print_expression(&member.expression);
                self.print("]");
            }
            _ => {
                // TSNonNullExpression, PrivateFieldExpression — use source text fallback
                let start = chain.span.start as usize;
                let end = chain.span.end as usize;
                if start < self.source_text.len() && end <= self.source_text.len() {
                    self.print(&self.source_text[start..end]);
                }
            }
        }
    }

    pub(super) fn print_call_expression(&mut self, call: &oxc_ast::ast::CallExpression<'a>) {
        // Print callee
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
        // Print optional call syntax
        if call.optional {
            self.print("?.");
        }
        // Print arguments
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
                    self.print_expression(&spread.argument);
                }
                _ => {
                    if let Some(expr) = arg.as_expression() {
                        self.print_expression(expr);
                    }
                }
            }
        }
        self.print(")");
    }

    pub(super) fn print_arrow_function(
        &mut self,
        arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>,
    ) {
        self.print_arrow_params(arrow);

        // Print body
        if arrow.expression {
            // Expression body — may contain JSX
            if let Some(expr) = arrow.body.statements.first()
                && let oxc_ast::ast::Statement::ExpressionStatement(expr_stmt) = expr
            {
                self.print_expression(&expr_stmt.expression);
            }
        } else {
            // Block body — need to handle JSX in return statements
            self.print_jsx_aware_function_body(&arrow.body);
        }
    }

    fn print_jsx_aware_function_body(&mut self, body: &oxc_ast::ast::FunctionBody<'a>) {
        self.add_source_mapping_for_span(body.span);
        self.print("{\n");
        for stmt in &body.statements {
            self.print_jsx_aware_statement(stmt);
        }
        self.print("}");
    }

    fn print_jsx_aware_statement(&mut self, stmt: &oxc_ast::ast::Statement<'a>) {
        use oxc_ast::ast::Statement;
        match stmt {
            Statement::ReturnStatement(ret) => {
                self.add_source_mapping_for_span(ret.span);
                self.print("\treturn ");
                if let Some(arg) = &ret.argument {
                    self.print_expression(arg);
                }
                self.print(";\n");
            }
            Statement::ExpressionStatement(expr_stmt) => {
                self.add_source_mapping_for_span(expr_stmt.span);
                self.print_expression(&expr_stmt.expression);
                self.print(";\n");
            }
            Statement::VariableDeclaration(decl) => {
                self.add_source_mapping_for_span(decl.span);
                let kind = match decl.kind {
                    VariableDeclarationKind::Var => "var",
                    VariableDeclarationKind::Let => "let",
                    VariableDeclarationKind::Const => "const",
                    VariableDeclarationKind::Using => "using",
                    VariableDeclarationKind::AwaitUsing => "await using",
                };
                for (i, declarator) in decl.declarations.iter().enumerate() {
                    if i == 0 {
                        self.print(kind);
                        self.print(" ");
                    } else {
                        self.print(", ");
                    }
                    let pattern_code = gen_to_string(&declarator.id);
                    self.print(&pattern_code);
                    if let Some(init) = &declarator.init {
                        self.print(" = ");
                        self.print_expression(init);
                    }
                }
                self.print(";\n");
            }
            Statement::IfStatement(if_stmt) => {
                self.add_source_mapping_for_span(if_stmt.span);
                self.print("if (");
                self.print_expression(&if_stmt.test);
                self.print(") ");
                self.print_jsx_aware_statement(&if_stmt.consequent);
                if let Some(alt) = &if_stmt.alternate {
                    self.print(" else ");
                    self.print_jsx_aware_statement(alt);
                }
            }
            Statement::BlockStatement(block) => {
                self.add_source_mapping_for_span(block.span);
                self.print("{\n");
                for s in &block.body {
                    self.print_jsx_aware_statement(s);
                }
                self.print("}");
            }
            Statement::ForOfStatement(for_of) => {
                self.add_source_mapping_for_span(for_of.span);
                self.print(if for_of.r#await { "for await(" } else { "for(" });
                // left side: variable declaration or assignment target
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
                self.print_jsx_aware_statement(&for_of.body);
                self.print("\n");
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
                        self.print_jsx_aware_statement(s);
                    }
                    self.print("\n");
                }
                self.print("}");
            }
            Statement::TryStatement(try_stmt) => {
                self.add_source_mapping_for_span(try_stmt.span);
                self.print("try {\n");
                for s in &try_stmt.block.body {
                    self.print_jsx_aware_statement(s);
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
                        self.print_jsx_aware_statement(s);
                    }
                    self.print("}");
                }
                if let Some(finalizer) = &try_stmt.finalizer {
                    self.print(" finally {\n");
                    for s in &finalizer.body {
                        self.print_jsx_aware_statement(s);
                    }
                    self.print("}");
                }
                self.print("\n");
            }
            Statement::ForStatement(for_stmt) => {
                self.add_source_mapping_for_span(for_stmt.span);
                self.print("for(");
                if let Some(init) = &for_stmt.init {
                    self.print_for_init(init);
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
                self.print_jsx_aware_statement(&for_stmt.body);
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
                self.print_jsx_aware_statement(&for_in.body);
                self.print("\n");
            }
            Statement::WhileStatement(while_stmt) => {
                self.add_source_mapping_for_span(while_stmt.span);
                self.print("while (");
                self.print_expression(&while_stmt.test);
                self.print(") ");
                self.print_jsx_aware_statement(&while_stmt.body);
                self.print("\n");
            }
            Statement::DoWhileStatement(do_while) => {
                self.add_source_mapping_for_span(do_while.span);
                self.print("do ");
                self.print_jsx_aware_statement(&do_while.body);
                self.print(" while (");
                self.print_expression(&do_while.test);
                self.print(");\n");
            }
            Statement::LabeledStatement(labeled) => {
                self.add_source_mapping_for_span(labeled.span);
                self.print(&labeled.label.name);
                self.print(": ");
                self.print_jsx_aware_statement(&labeled.body);
            }
            Statement::FunctionDeclaration(func) => {
                self.add_source_mapping_for_span(func.span);
                self.print_function(func);
                self.print("\n");
            }
            Statement::ClassDeclaration(class) => {
                self.add_source_mapping_for_span(class.span);
                self.print_class(class);
                self.print("\n");
            }
            Statement::ThrowStatement(throw_stmt) => {
                self.add_source_mapping_for_span(throw_stmt.span);
                self.print("throw ");
                self.print_expression(&throw_stmt.argument);
                self.print(";\n");
            }
            _ => {
                self.add_source_mapping_for_span(stmt.span());
                // For other statements, use regular codegen
                let code = gen_to_string(stmt);
                self.print(&code);
                self.print("\n");
            }
        }
    }

    /// Print a function declaration or expression with JSX-aware body.
    pub(super) fn print_function(&mut self, func: &oxc_ast::ast::Function<'a>) {
        if func.r#async {
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
            self.print_jsx_aware_function_body(body);
        }
    }

    /// Print a `FormalParameters` list, including default-value initializers
    /// and the rest parameter. No surrounding parens.
    ///
    /// Parameter-property modifiers and decorators have runtime effect (Phase 2
    /// turns them into `this.x = x` assignments), so they're emitted faithfully.
    pub(super) fn print_formal_parameters(&mut self, params: &oxc_ast::ast::FormalParameters<'a>) {
        use oxc_ast::ast::TSAccessibility;
        let mut first = true;
        for param in &params.items {
            if !first {
                self.print(", ");
            }
            first = false;
            self.print_decorators(&param.decorators);
            match param.accessibility {
                Some(TSAccessibility::Public) => self.print("public "),
                Some(TSAccessibility::Protected) => self.print("protected "),
                Some(TSAccessibility::Private) => self.print("private "),
                None => {}
            }
            if param.r#override {
                self.print("override ");
            }
            if param.readonly {
                self.print("readonly ");
            }
            self.print_binding_pattern(&param.pattern);
            if let Some(init) = &param.initializer {
                self.print(" = ");
                self.print_expression(init);
            }
        }
        if let Some(rest) = &params.rest {
            if !first {
                self.print(", ");
            }
            self.print("...");
            self.print_binding_pattern(&rest.rest.argument);
        }
    }

    pub(super) fn print_binding_pattern(&mut self, pattern: &oxc_ast::ast::BindingPattern<'a>) {
        if let oxc_ast::ast::BindingPattern::BindingIdentifier(ident) = pattern {
            self.print(ident.name.as_str());
        } else {
            // For complex patterns, use regular codegen
            let code = gen_to_string(pattern);
            self.print(&code);
        }
    }

    /// Print leading decorators as `@expr `.
    fn print_decorators(&mut self, decorators: &[oxc_ast::ast::Decorator<'a>]) {
        for decorator in decorators {
            self.print("@");
            self.print_expression(&decorator.expression);
            self.print(" ");
        }
    }

    /// Print a class declaration or expression with JSX-aware method bodies,
    /// property initializers, and static blocks.
    pub(super) fn print_class(&mut self, class: &oxc_ast::ast::Class<'a>) {
        self.print_decorators(&class.decorators);
        self.print("class");
        if let Some(id) = &class.id {
            self.print(" ");
            self.print(id.name.as_str());
        }
        if let Some(super_class) = &class.super_class {
            self.print(" extends ");
            self.print_expression(super_class);
        }
        self.print(" {\n");
        for element in &class.body.body {
            self.print_class_element(element);
        }
        self.print("}");
    }

    fn print_class_element(&mut self, element: &oxc_ast::ast::ClassElement<'a>) {
        use oxc_ast::ast::{ClassElement, MethodDefinitionKind};
        match element {
            ClassElement::MethodDefinition(method) => {
                self.add_source_mapping_for_span(method.span);
                self.print_decorators(&method.decorators);
                if method.r#static {
                    self.print("static ");
                }
                match method.kind {
                    MethodDefinitionKind::Get => self.print("get "),
                    MethodDefinitionKind::Set => self.print("set "),
                    MethodDefinitionKind::Constructor | MethodDefinitionKind::Method => {}
                }
                if method.value.r#async {
                    self.print("async ");
                }
                if method.value.generator {
                    self.print("*");
                }
                if method.computed {
                    self.print("[");
                }
                let key_code = gen_to_string(&method.key);
                self.print(&key_code);
                if method.computed {
                    self.print("]");
                }
                self.print("(");
                self.print_formal_parameters(&method.value.params);
                self.print(") ");
                if let Some(body) = &method.value.body {
                    self.print_jsx_aware_function_body(body);
                }
                self.print("\n");
            }
            ClassElement::PropertyDefinition(prop) => {
                self.add_source_mapping_for_span(prop.span);
                self.print_decorators(&prop.decorators);
                if prop.r#static {
                    self.print("static ");
                }
                if prop.computed {
                    self.print("[");
                }
                let key_code = gen_to_string(&prop.key);
                self.print(&key_code);
                if prop.computed {
                    self.print("]");
                }
                if let Some(value) = &prop.value {
                    self.print(" = ");
                    self.print_expression(value);
                }
                self.print(";\n");
            }
            ClassElement::StaticBlock(block) => {
                self.add_source_mapping_for_span(block.span);
                self.print("static {\n");
                for stmt in &block.body {
                    self.print_jsx_aware_statement(stmt);
                }
                self.print("}\n");
            }
            // TSIndexSignature is type-only; AccessorProperty rarely carries
            // JSX. Both fall back to plain codegen.
            ClassElement::AccessorProperty(_) | ClassElement::TSIndexSignature(_) => {
                let code = gen_to_string(element);
                self.print(&code);
                self.print("\n");
            }
        }
    }

    fn print_for_init(&mut self, init: &oxc_ast::ast::ForStatementInit<'a>) {
        match init {
            oxc_ast::ast::ForStatementInit::VariableDeclaration(decl) => {
                let kind = match decl.kind {
                    VariableDeclarationKind::Var => "var",
                    VariableDeclarationKind::Let => "let",
                    VariableDeclarationKind::Const => "const",
                    VariableDeclarationKind::Using => "using",
                    VariableDeclarationKind::AwaitUsing => "await using",
                };
                for (i, declarator) in decl.declarations.iter().enumerate() {
                    if i == 0 {
                        self.print(kind);
                        self.print(" ");
                    } else {
                        self.print(", ");
                    }
                    let pattern_code = gen_to_string(&declarator.id);
                    self.print(&pattern_code);
                    if let Some(init_expr) = &declarator.init {
                        self.print(" = ");
                        self.print_expression(init_expr);
                    }
                }
            }
            other => {
                if let Some(expr) = other.as_expression() {
                    self.print_expression(expr);
                }
            }
        }
    }
}
