//! Semantic analysis: scope resolution, symbol usage, lightweight type checking,
//! builtin/member validation, and flow-based warnings. Walks the (possibly
//! partial) AST from the parser and pushes diagnostics.
//!
//! Everything here is conservative — a check only fires when the analyzer is
//! confident — so half-typed code produces useful, low-noise feedback.

use std::collections::HashSet;

use super::api::Param;
use super::ast::*;
use super::builtin::Builtins;
use super::diagnostics::{Diagnostics, Span};
use super::flow;
use super::scope::{Decl, Scope, SymKind, Symbol};
use super::suggestions::{closest, did_you_mean};
use super::types::{Ty, members, resolve_callable, type_of};

pub fn analyze(src: &str, block: &Block, b: &Builtins, diags: &mut Diagnostics) {
    let mut globals = HashSet::new();
    collect_globals(block, &mut globals);
    let mut a = Analyzer {
        src,
        b,
        diags,
        scope: Scope::default(),
        globals,
        loop_depth: 0,
    };
    a.scope.push(true);
    a.walk_block(block);
    let closed = a.scope.pop();
    a.report_unused(&closed);
}

struct Analyzer<'a> {
    src: &'a str,
    b: &'a Builtins,
    diags: &'a mut Diagnostics,
    scope: Scope,
    /// Names assigned as globals somewhere in the file (so a forward reference
    /// to a global function/variable isn't flagged as undefined).
    globals: HashSet<String>,
    loop_depth: u32,
}

impl<'a> Analyzer<'a> {
    fn text(&self, span: Span) -> &str {
        &self.src[span.range()]
    }

    fn ty(&self, e: &Expr) -> Ty {
        type_of(e, &self.scope, self.b, self.src)
    }

    // --- declarations -------------------------------------------------------

    fn declare(&mut self, def: &NameDef, kind: SymKind, ty: Ty) {
        if def.name.is_empty() {
            return;
        }
        match self.scope.declare(&def.name, kind, def.span, ty) {
            Decl::Duplicate => {
                let what = if kind == SymKind::Param { "parameter" } else { "declaration" };
                self.diags
                    .warning(def.span, format!("duplicate {what} `{}`", def.name));
            }
            Decl::Shadows if kind == SymKind::Local => {
                self.diags
                    .info(def.span, format!("`{}` shadows an outer declaration", def.name));
            }
            _ => {}
        }
    }

    fn report_unused(&mut self, closed: &[Symbol]) {
        for s in closed {
            if s.used || s.name.starts_with('_') || s.name.is_empty() {
                continue;
            }
            match s.kind {
                SymKind::Local => self.diags.warning(s.span, format!("unused local `{}`", s.name)),
                SymKind::Function => self
                    .diags
                    .warning(s.span, format!("unused local function `{}`", s.name)),
                SymKind::Param => self
                    .diags
                    .hint(s.span, format!("unused parameter `{}`", s.name)),
                SymKind::ForVar => self
                    .diags
                    .hint(s.span, format!("unused loop variable `{}`", s.name)),
            }
        }
    }

    // --- blocks / statements ------------------------------------------------

    /// Walk a block that already has its scope frame pushed by the caller.
    fn walk_block(&mut self, block: &Block) {
        let mut diverged = false;
        let mut reported = false;
        for stmt in &block.stmts {
            if diverged && !reported && !matches!(stmt, Stmt::Error { .. }) {
                self.diags.warning(stmt.span(), "unreachable code");
                reported = true;
            }
            self.stmt(stmt);
            if flow::stmt_diverges(stmt) {
                diverged = true;
            }
        }
    }

    /// Walk a block in its own fresh scope.
    fn scoped(&mut self, block: &Block, is_function: bool) {
        self.scope.push(is_function);
        self.walk_block(block);
        let closed = self.scope.pop();
        self.report_unused(&closed);
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Local { names, values, .. } => {
                for v in values {
                    self.expr(v);
                }
                for (i, def) in names.iter().enumerate() {
                    let ty = values.get(i).map(|e| self.ty(e)).unwrap_or(Ty::Unknown);
                    self.declare(def, SymKind::Local, ty);
                }
            }
            Stmt::LocalFunction { name, body, .. } => {
                self.declare(name, SymKind::Function, Ty::Function);
                self.func_body(body, false);
            }
            Stmt::Function { path, method, body, .. } => {
                if let Some(base) = path.first() {
                    if self.scope.is_declared(&base.name) {
                        self.scope.mark_used(&base.name);
                    }
                }
                self.func_body(body, method.is_some());
            }
            Stmt::Assign { targets, values, compound, .. } => {
                for v in values {
                    self.expr(v);
                }
                for t in targets {
                    // A compound assignment (`a += b`) also *reads* the target.
                    if *compound {
                        self.expr(t);
                    }
                    self.assign_target(t);
                }
            }
            Stmt::ExprStmt(e) => self.expr(e),
            Stmt::Do { body, .. } => self.scoped(body, false),
            Stmt::While { cond, body, span } => {
                self.condition(cond);
                self.empty_loop(body, *span);
                self.loop_depth += 1;
                self.scoped(body, false);
                self.loop_depth -= 1;
            }
            Stmt::Repeat { body, cond, span } => {
                // `until` can see the body's locals, so analyze it inside the frame.
                self.empty_loop(body, *span);
                self.loop_depth += 1;
                self.scope.push(false);
                self.walk_block(body);
                self.expr(cond);
                let closed = self.scope.pop();
                self.report_unused(&closed);
                self.loop_depth -= 1;
            }
            Stmt::If { arms, else_block, span } => self.if_stmt(arms, else_block, *span),
            Stmt::NumericFor { var, start, end, step, body, span } => {
                self.expr(start);
                self.expr(end);
                if let Some(s) = step {
                    self.expr(s);
                }
                self.empty_loop(body, *span);
                self.loop_depth += 1;
                self.scope.push(false);
                self.declare(var, SymKind::ForVar, Ty::Number);
                self.walk_block(body);
                let closed = self.scope.pop();
                self.report_unused(&closed);
                self.loop_depth -= 1;
            }
            Stmt::GenericFor { vars, exprs, body, span } => {
                for e in exprs {
                    self.expr(e);
                }
                self.empty_loop(body, *span);
                self.loop_depth += 1;
                self.scope.push(false);
                for v in vars {
                    self.declare(v, SymKind::ForVar, Ty::Unknown);
                }
                self.walk_block(body);
                let closed = self.scope.pop();
                self.report_unused(&closed);
                self.loop_depth -= 1;
            }
            Stmt::Return { values, .. } => {
                for v in values {
                    self.expr(v);
                }
            }
            Stmt::Break { span } => {
                if self.loop_depth == 0 {
                    self.diags.error(*span, "`break` outside of a loop");
                }
            }
            Stmt::Continue { span } => {
                if self.loop_depth == 0 {
                    self.diags.error(*span, "`continue` outside of a loop");
                }
            }
            Stmt::TypeAlias { .. } | Stmt::Error { .. } => {}
        }
    }

    fn func_body(&mut self, body: &FuncBody, is_method: bool) {
        // Function bodies interrupt loop context for break/continue.
        let saved = self.loop_depth;
        self.loop_depth = 0;
        self.scope.push(true);
        if is_method {
            // Implicit `self`; declared used so it's never "unused".
            self.scope.declare("self", SymKind::Param, Span::default(), Ty::Unknown);
            self.scope.mark_used("self");
        }
        for p in &body.params {
            self.declare(p, SymKind::Param, Ty::Unknown);
        }
        self.walk_block(&body.body);
        let closed = self.scope.pop();
        self.report_unused(&closed);
        self.loop_depth = saved;
    }

    fn if_stmt(&mut self, arms: &[(Expr, Block)], else_block: &Option<Block>, _span: Span) {
        let mut seen: Vec<String> = Vec::new();
        for (cond, body) in arms {
            self.condition(cond);
            // Duplicate condition within the same chain.
            let norm = normalize(self.text(cond.span()));
            if !norm.is_empty() && seen.contains(&norm) {
                self.diags.warning(cond.span(), "duplicate condition in `if`/`elseif` chain");
            } else {
                seen.push(norm);
            }
            self.scoped(body, false);
        }
        if let Some(eb) = else_block {
            self.scoped(eb, false);
        }
        // Empty if: every branch body empty.
        let all_empty = arms.iter().all(|(_, b)| b.stmts.is_empty())
            && else_block.as_ref().map(|b| b.stmts.is_empty()).unwrap_or(true);
        if all_empty {
            if let Some((_, first)) = arms.first() {
                let sp = first.stmts.first().map(|s| s.span()).unwrap_or_else(|| arms[0].0.span());
                self.diags.hint(sp, "empty `if` statement");
            }
        }
    }

    fn condition(&mut self, cond: &Expr) {
        self.expr(cond);
        let msg = match cond {
            Expr::True(_) => Some("condition is always true"),
            Expr::False(_) | Expr::Nil(_) => Some("condition is always false"),
            Expr::Number { .. } | Expr::Str { .. } | Expr::Table { .. } | Expr::Function { .. } => {
                Some("condition is always truthy")
            }
            _ => None,
        };
        if let Some(m) = msg {
            self.diags.warning(cond.span(), m);
        }
    }

    fn empty_loop(&mut self, body: &Block, at: Span) {
        if body.stmts.is_empty() {
            self.diags.hint(at, "empty loop body");
        }
    }

    fn assign_target(&mut self, target: &Expr) {
        match target {
            Expr::Name(n) => {
                if self.scope.is_declared(&n.name) {
                    self.scope.mark_reassigned(&n.name);
                }
                // Assigning a global is legal; nothing to report.
            }
            Expr::Field { object, .. } | Expr::Index { object, .. } => {
                self.expr(object);
                if let Expr::Index { key, .. } = target {
                    self.expr(key);
                }
            }
            other => self.expr(other),
        }
    }

    // --- expressions --------------------------------------------------------

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Name(n) => self.name_read(n),
            Expr::Paren { expr, .. } => self.expr(expr),
            Expr::Field { object, field, .. } => {
                self.expr(object);
                self.check_member(object, &field.name, field.span, false);
            }
            Expr::Index { object, key, .. } => {
                self.expr(object);
                self.expr(key);
                self.check_index(object);
            }
            Expr::Call { callee, method, args, span } => self.call(callee, method.as_ref(), args, *span),
            Expr::Binary { op, lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
                self.check_arithmetic(*op, lhs, rhs);
            }
            Expr::Unary { expr, .. } => self.expr(expr),
            Expr::Table { fields, .. } => self.table(fields),
            Expr::Function { body, .. } => self.func_body(body, false),
            _ => {}
        }
    }

    fn name_read(&mut self, n: &NameRef) {
        if self.scope.is_declared(&n.name) {
            self.scope.mark_used(&n.name);
            return;
        }
        if self.b.is_global(&n.name) || self.globals.contains(&n.name) || n.name.is_empty() {
            return;
        }
        let candidates: Vec<&str> = self
            .b
            .db()
            .globals
            .keys()
            .map(String::as_str)
            .chain(self.globals.iter().map(String::as_str))
            .collect();
        let hint = did_you_mean(closest(&n.name, candidates));
        self.diags
            .warning(n.span, format!("unknown variable `{}`.{hint}", n.name));
    }

    /// Validate `object.member` (or a method name when `method` is true).
    fn check_member(&mut self, object: &Expr, member: &str, span: Span, method: bool) {
        let ot = self.ty(object);
        if !ot.is_indexable() {
            self.diags
                .error(span.to(object.span()), format!("attempt to index {}", ot.describe()));
            return;
        }
        // Field access on a dynamic type (Instance children/properties) can't be
        // validated. Method calls *can* — an instance's method set is fixed.
        if (!method && is_open(&ot)) || member.is_empty() {
            return;
        }
        let Some(map) = members(&ot, self.b) else {
            return;
        };
        if !map.contains_key(member) {
            let kind = if method { "method" } else { "member" };
            let hint = did_you_mean(closest(member, map.keys().map(String::as_str)));
            self.diags.warning(
                span,
                format!("unknown {kind} `{member}` on {}.{hint}", ot.describe()),
            );
        }
    }

    fn check_index(&mut self, object: &Expr) {
        let ot = self.ty(object);
        if !ot.is_indexable() {
            self.diags
                .error(object.span(), format!("attempt to index {}", ot.describe()));
        }
    }

    fn call(&mut self, callee: &Expr, method: Option<&NameRef>, args: &[Expr], span: Span) {
        self.expr(callee);
        for a in args {
            self.expr(a);
        }

        if let Some(m) = method {
            // Method call: validate the method against the receiver.
            self.check_member(callee, &m.name, m.span, true);
        } else {
            let ct = self.ty(callee);
            if !ct.is_callable() {
                self.diags
                    .error(callee.span(), format!("attempt to call {}", ct.describe()));
            }
            // `obj.Method(...)` where Method needs `self`.
            if let Expr::Field { object, field, .. } = callee {
                let ot = self.ty(object);
                if let Some(map) = members(&ot, self.b) {
                    if map.get(&field.name).is_some_and(|e| e.kind == "method") {
                        self.diags.warning(
                            field.span,
                            format!("`{}` is a method; call it with `:` not `.`", field.name),
                        );
                    }
                }
            }
        }

        self.check_arg_count(callee, method, args, span);
    }

    fn check_arg_count(&mut self, callee: &Expr, method: Option<&NameRef>, args: &[Expr], span: Span) {
        // A trailing call/vararg can expand to many values — arity is unknown.
        if matches!(args.last(), Some(Expr::Call { .. } | Expr::Vararg(_))) {
            return;
        }
        let Some(entry) = resolve_callable(callee, method, &self.scope, self.b, self.src) else {
            return;
        };
        if entry.params.iter().any(Param::is_variadic) {
            return;
        }
        let name = method
            .map(|m| m.name.as_str())
            .or_else(|| match callee {
                Expr::Field { field, .. } => Some(field.name.as_str()),
                Expr::Name(n) => Some(n.name.as_str()),
                _ => None,
            })
            .unwrap_or("function");
        let max = entry.params.len();
        let required = entry.params.iter().filter(|p| !p.is_optional()).count();
        let argc = args.len();
        if argc > max {
            self.diags.warning(
                span,
                format!("`{name}` takes at most {max} argument(s), but {argc} were given"),
            );
        } else if argc < required {
            self.diags.warning(
                span,
                format!("`{name}` takes at least {required} argument(s), but {argc} were given"),
            );
        }
    }

    fn check_arithmetic(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) {
        use BinOp::*;
        let arithmetic = matches!(op, Add | Sub | Mul | Div | Mod | Pow);
        let concat = matches!(op, Concat);
        if !arithmetic && !concat {
            return;
        }
        for operand in [lhs, rhs] {
            let t = self.ty(operand);
            let bad = if arithmetic {
                matches!(t, Ty::String | Ty::Bool | Ty::Nil)
            } else {
                matches!(t, Ty::Bool | Ty::Nil) // concat allows string+number
            };
            if bad {
                let verb = if arithmetic { "perform arithmetic on" } else { "concatenate" };
                self.diags
                    .warning(operand.span(), format!("attempt to {verb} {}", t.describe()));
            }
        }
    }

    fn table(&mut self, fields: &[TableField]) {
        let mut named: Vec<String> = Vec::new();
        let mut keyed: Vec<String> = Vec::new();
        for f in fields {
            match f {
                TableField::Positional(v) => self.expr(v),
                TableField::Named { name, value } => {
                    if named.contains(&name.name) {
                        self.diags
                            .warning(name.span, format!("duplicate table key `{}`", name.name));
                    } else {
                        named.push(name.name.clone());
                    }
                    self.expr(value);
                }
                TableField::Keyed { key, value } => {
                    // Only literal keys can be compared cheaply.
                    if matches!(key, Expr::Str { .. } | Expr::Number { .. }) {
                        let k = normalize(self.text(key.span()));
                        if keyed.contains(&k) {
                            self.diags.warning(key.span(), "duplicate table key");
                        } else {
                            keyed.push(k);
                        }
                    }
                    self.expr(key);
                    self.expr(value);
                }
            }
        }
    }
}

/// Standard Lua libraries whose member sets are large/version-dependent; we
/// don't validate members against them to avoid false positives.
const OPEN_LIBS: &[&str] = &["math", "string", "table", "os", "coroutine", "utf8", "bit32", "debug", "buffer"];

/// A type whose members can't be exhaustively validated — dynamic instances
/// (children/properties) and the big standard libraries.
fn is_open(ty: &Ty) -> bool {
    match ty {
        Ty::Named(n) => n == "Instance",
        Ty::Library(n) => OPEN_LIBS.contains(&n.as_str()),
        _ => false,
    }
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Pre-pass: names defined as file-level globals (global functions and global
/// assignment targets), so forward references aren't flagged as undefined.
fn collect_globals(block: &Block, out: &mut HashSet<String>) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Function { path, method, body, .. } => {
                if method.is_none() && path.len() == 1 {
                    out.insert(path[0].name.clone());
                }
                collect_globals(&body.body, out);
            }
            Stmt::LocalFunction { body, .. } => collect_globals(&body.body, out),
            Stmt::Assign { targets, .. } => {
                for t in targets {
                    if let Expr::Name(n) = t {
                        out.insert(n.name.clone());
                    }
                }
            }
            Stmt::Do { body, .. }
            | Stmt::While { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::NumericFor { body, .. }
            | Stmt::GenericFor { body, .. } => collect_globals(body, out),
            Stmt::If { arms, else_block, .. } => {
                for (_, b) in arms {
                    collect_globals(b, out);
                }
                if let Some(b) = else_block {
                    collect_globals(b, out);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::parser::parse;

    fn check(src: &str) -> Vec<super::super::diagnostics::Diagnostic> {
        let b = Builtins::load();
        let mut d = Diagnostics::new(src);
        let block = parse(src, &mut d);
        analyze(src, &block, &b, &mut d);
        d.finish()
    }

    fn has(src: &str, needle: &str) -> bool {
        check(src).iter().any(|x| x.message.contains(needle))
    }

    #[test]
    fn clean_code_is_quiet() {
        let src = "\
local function add(a, b)
    return a + b
end
local total = add(1, 2)
print(total)
";
        let d = check(src);
        assert!(d.is_empty(), "unexpected diagnostics: {d:?}");
    }

    #[test]
    fn calling_a_number() {
        assert!(has("local x = 5\nprint(x())\n", "attempt to call a number"));
    }

    #[test]
    fn arithmetic_on_string() {
        assert!(has("local x = \"hi\"\nreturn x + 5\n", "arithmetic on a string"));
    }

    #[test]
    fn indexing_a_boolean() {
        assert!(has("return true.foo\n", "attempt to index a boolean"));
    }

    #[test]
    fn unknown_method_suggests() {
        let d = check("game:GetServi()\n");
        assert!(
            d.iter().any(|x| x.message.contains("unknown method `GetServi`")
                && x.message.contains("GetService")),
            "{d:?}"
        );
    }

    #[test]
    fn unknown_member_suggests() {
        // Vec2 is a closed type: `.Magnitud` should suggest `Magnitude`.
        let d = check("local v = Vec2.new(1, 2)\nreturn v.Magnitud\n");
        assert!(d.iter().any(|x| x.message.contains("Magnitude")), "{d:?}");
    }

    #[test]
    fn instance_members_not_flagged() {
        // Instances are dynamic — arbitrary child/property access is fine.
        assert!(!has("return workspace.SomeRandomPart\n", "unknown member"));
    }

    #[test]
    fn undefined_variable() {
        assert!(has("print(nonexistentThing)\n", "unknown variable `nonexistentThing`"));
    }

    #[test]
    fn known_forward_global_ok() {
        let src = "callLater()\nfunction callLater() end\n";
        assert!(!has(src, "unknown variable"));
    }

    #[test]
    fn unused_local_warns() {
        assert!(has("local unusedX = 5\nprint(1)\n", "unused local `unusedX`"));
    }

    #[test]
    fn unreachable_after_flow_divergence() {
        // Both branches return, so the statement after the `if` is unreachable.
        let src = "\
local function f(x)
    if x then return 1 else return 2 end
    print(3)
end
";
        assert!(has(src, "unreachable"));
        // An `if` without `else` falls through — the next statement is reachable.
        assert!(!has("local function f(x)\n if x then return 1 end\n return 2\nend\n", "unreachable"));
    }

    #[test]
    fn break_outside_loop() {
        assert!(has("break\n", "`break` outside of a loop"));
    }

    #[test]
    fn duplicate_parameter() {
        assert!(has("local function f(a, a) return a end\n", "duplicate parameter `a`"));
    }

    #[test]
    fn wrong_arg_count() {
        assert!(has("local v = Vec2.new(1, 2, 3)\nprint(v)\n", "at most 2"));
    }

    #[test]
    fn duplicate_table_key() {
        assert!(has("local t = { a = 1, a = 2 }\nprint(t)\n", "duplicate table key `a`"));
    }

    #[test]
    fn constant_condition() {
        assert!(has("if true then print(1) end\n", "always true"));
    }

    #[test]
    fn compound_assignment_ok() {
        let src = "local n = 0\nn += 1\nn -= 2\nlocal s = \"a\"\ns ..= \"b\"\nprint(n, s)\n";
        let d = check(src);
        assert!(d.is_empty(), "compound assignment should be clean: {d:?}");
    }

    #[test]
    fn empty_loop_hinted() {
        assert!(has("while true do end\n", "empty loop body"));
    }

    #[test]
    fn no_panic_on_garbage() {
        for src in ["", "((((", "end end end", "local = = =", "function", "for in do", ")]}", "::"] {
            let _ = check(src); // must not panic
        }
    }

    /// Real Flux sample scripts must not produce spurious errors or "unknown …"
    /// false positives (guards the whole pipeline against over-eager checks).
    ///
    /// Scripts are read at runtime (not `include_str!`) so deleting a sample
    /// project never breaks the build — missing files are simply skipped.
    #[test]
    fn real_scripts_have_no_false_positives() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let candidates = [
            "projects/demo/scripts/movement.luau",
            "projects/demo/scripts/ui.luau",
            "projects/flux_sample_clicker/scripts/main.luau",
            "projects/dino_run/scripts/main.luau",
        ];
        for rel in candidates {
            let Ok(src) = std::fs::read_to_string(root.join(rel)) else {
                continue;
            };
            let d = check(&src);
            let bad: Vec<_> = d
                .iter()
                .filter(|x| {
                    x.severity == super::super::diagnostics::DiagnosticSeverity::Error
                        || x.message.contains("unknown")
                })
                .collect();
            assert!(bad.is_empty(), "{rel}: unexpected diagnostics: {bad:?}");
        }
    }
}
