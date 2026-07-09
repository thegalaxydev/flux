//! Error-recovering recursive-descent parser for the Luau subset. Produces a
//! (possibly partial) [`Block`] and reports syntax diagnostics. It never panics
//! and always makes forward progress, so a half-typed file still yields a tree
//! the semantic passes can walk.

use super::ast::*;
use super::diagnostics::{Diagnostics, Span};
use super::token::{Tk, Token, lex};

pub struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    prev_end: usize,
    diags: &'a mut Diagnostics,
    /// Guards against pathological inputs (keeps analysis snappy while typing).
    budget: u32,
}

/// Parse `src` into a block, reporting diagnostics into `diags`.
pub fn parse<'a>(src: &'a str, diags: &'a mut Diagnostics) -> Block {
    let toks = lex(src, diags);
    let mut p = Parser { src, toks, pos: 0, prev_end: 0, diags, budget: 2_000_000 };
    let block = p.parse_block();
    if !p.at(Tk::Eof) {
        let sp = p.peek().span;
        p.diags.error(sp, format!("unexpected `{}`", p.text(sp)));
    }
    block
}

impl<'a> Parser<'a> {
    // --- token cursor -------------------------------------------------------

    fn peek(&self) -> Token {
        self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn peek2(&self) -> Token {
        self.toks[(self.pos + 1).min(self.toks.len() - 1)]
    }

    fn kind(&self) -> Tk {
        self.peek().kind
    }

    fn at(&self, k: Tk) -> bool {
        self.kind() == k
    }

    fn text(&self, span: Span) -> &str {
        &self.src[span.start..span.end]
    }

    fn tick(&mut self) -> bool {
        self.budget = self.budget.saturating_sub(1);
        self.budget > 0
    }

    fn bump(&mut self) -> Token {
        let t = self.peek();
        self.prev_end = t.span.end;
        if t.kind != Tk::Eof {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, k: Tk) -> bool {
        if self.at(k) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Expect `k`; on mismatch report `msg` at the current token and don't consume.
    fn expect(&mut self, k: Tk, msg: &str) -> bool {
        if self.eat(k) {
            true
        } else {
            let sp = self.peek().span;
            self.diags.error(sp, msg.to_string());
            false
        }
    }

    fn span_from(&self, start: usize) -> Span {
        Span::new(start, self.prev_end.max(start))
    }

    fn is_block_end(&self) -> bool {
        matches!(
            self.kind(),
            Tk::Eof | Tk::End | Tk::Else | Tk::Elseif | Tk::Until
        )
    }

    // --- blocks / statements ------------------------------------------------

    fn parse_block(&mut self) -> Block {
        let mut stmts = Vec::new();
        while !self.is_block_end() && self.tick() {
            let before = self.pos;
            if self.eat(Tk::Semicolon) {
                continue;
            }
            if let Some(stmt) = self.parse_stmt() {
                let terminal = matches!(stmt, Stmt::Return { .. });
                stmts.push(stmt);
                if terminal {
                    break; // `return` must end a block
                }
            }
            if self.pos == before {
                // No progress — consume a token so we can't loop forever, and
                // leave an Error node so semantic analysis skips the gap.
                let sp = self.bump().span;
                self.diags.error(sp, format!("unexpected `{}`", self.text(sp)));
                stmts.push(Stmt::Error { span: sp });
            }
        }
        Block { stmts }
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        let start = self.peek().span.start;
        match self.kind() {
            Tk::Local => Some(self.parse_local(start)),
            Tk::Function => Some(self.parse_function_stmt(start)),
            Tk::Do => {
                self.bump();
                let body = self.parse_block();
                self.expect(Tk::End, "missing `end` to close `do` block");
                Some(Stmt::Do { body, span: self.span_from(start) })
            }
            Tk::While => {
                self.bump();
                let cond = self.parse_expr();
                self.expect(Tk::Do, "expected `do` after `while` condition");
                let body = self.parse_block();
                self.expect(Tk::End, "missing `end` to close `while` loop");
                Some(Stmt::While { cond, body, span: self.span_from(start) })
            }
            Tk::Repeat => {
                self.bump();
                let body = self.parse_block();
                self.expect(Tk::Until, "missing `until` to close `repeat` loop");
                let cond = self.parse_expr();
                Some(Stmt::Repeat { body, cond, span: self.span_from(start) })
            }
            Tk::If => Some(self.parse_if(start)),
            Tk::For => Some(self.parse_for(start)),
            Tk::Return => {
                self.bump();
                let mut values = Vec::new();
                if !self.is_block_end() && !self.at(Tk::Semicolon) {
                    values = self.parse_expr_list();
                }
                self.eat(Tk::Semicolon);
                Some(Stmt::Return { values, span: self.span_from(start) })
            }
            Tk::Break => {
                self.bump();
                Some(Stmt::Break { span: self.span_from(start) })
            }
            Tk::Name if self.text(self.peek().span) == "continue" && self.continue_is_stmt() => {
                self.bump();
                Some(Stmt::Continue { span: self.span_from(start) })
            }
            Tk::Name if self.text(self.peek().span) == "type" && self.type_alias_ahead() => {
                Some(self.parse_type_alias(start))
            }
            _ => Some(self.parse_expr_stmt(start)),
        }
    }

    /// `continue` is a real statement only when the next token ends it.
    fn continue_is_stmt(&self) -> bool {
        matches!(
            self.peek2().kind,
            Tk::Eof | Tk::End | Tk::Else | Tk::Elseif | Tk::Until | Tk::Semicolon
        )
    }

    /// `type Name` (optionally `type Name<...>`) `=` — a type alias.
    fn type_alias_ahead(&self) -> bool {
        self.peek2().kind == Tk::Name
    }

    fn parse_type_alias(&mut self, start: usize) -> Stmt {
        self.bump(); // `type`
        self.bump(); // name
        if self.at(Tk::Lt) {
            self.skip_balanced(Tk::Lt, Tk::Gt);
        }
        self.expect(Tk::Assign, "expected `=` in type alias");
        self.skip_type();
        Stmt::TypeAlias { span: self.span_from(start) }
    }

    fn parse_local(&mut self, start: usize) -> Stmt {
        self.bump(); // `local`
        if self.at(Tk::Function) {
            self.bump();
            let name = self.parse_name_def_no_type();
            let body = self.parse_func_body(start);
            return Stmt::LocalFunction { name, body, span: self.span_from(start) };
        }
        let mut names = Vec::new();
        loop {
            if self.at(Tk::Name) {
                names.push(self.parse_name_def());
            } else {
                self.diags.error(self.peek().span, "expected a variable name");
                break;
            }
            if !self.eat(Tk::Comma) {
                break;
            }
        }
        let mut values = Vec::new();
        if self.eat(Tk::Assign) {
            values = self.parse_expr_list();
        }
        Stmt::Local { names, values, span: self.span_from(start) }
    }

    fn parse_function_stmt(&mut self, start: usize) -> Stmt {
        self.bump(); // `function`
        let mut path = Vec::new();
        let mut method = None;
        if self.at(Tk::Name) {
            path.push(self.name_ref());
            while self.eat(Tk::Dot) {
                if self.at(Tk::Name) {
                    path.push(self.name_ref());
                } else {
                    self.diags.error(self.peek().span, "expected a name after `.`");
                    break;
                }
            }
            if self.eat(Tk::Colon) {
                if self.at(Tk::Name) {
                    method = Some(self.name_ref());
                } else {
                    self.diags.error(self.peek().span, "expected a method name after `:`");
                }
            }
        } else {
            self.diags.error(self.peek().span, "expected a function name");
        }
        let body = self.parse_func_body(start);
        Stmt::Function { path, method, body, span: self.span_from(start) }
    }

    fn parse_if(&mut self, start: usize) -> Stmt {
        self.bump(); // `if`
        let mut arms = Vec::new();
        let cond = self.parse_expr();
        self.expect(Tk::Then, "expected `then` after `if` condition");
        let body = self.parse_block();
        arms.push((cond, body));
        while self.at(Tk::Elseif) {
            self.bump();
            let cond = self.parse_expr();
            self.expect(Tk::Then, "expected `then` after `elseif` condition");
            let body = self.parse_block();
            arms.push((cond, body));
        }
        let else_block = if self.eat(Tk::Else) {
            Some(self.parse_block())
        } else {
            None
        };
        self.expect(Tk::End, "missing `end` to close `if` statement");
        Stmt::If { arms, else_block, span: self.span_from(start) }
    }

    fn parse_for(&mut self, start: usize) -> Stmt {
        self.bump(); // `for`
        let first = if self.at(Tk::Name) {
            self.parse_name_def()
        } else {
            self.diags.error(self.peek().span, "expected a loop variable");
            NameDef { name: String::new(), span: self.peek().span }
        };
        if self.eat(Tk::Assign) {
            // numeric for
            let start_e = self.parse_expr();
            self.expect(Tk::Comma, "expected `,` in numeric `for`");
            let end_e = self.parse_expr();
            let step = if self.eat(Tk::Comma) { Some(self.parse_expr()) } else { None };
            self.expect(Tk::Do, "expected `do` in `for` loop");
            let body = self.parse_block();
            self.expect(Tk::End, "missing `end` to close `for` loop");
            Stmt::NumericFor { var: first, start: start_e, end: end_e, step, body, span: self.span_from(start) }
        } else {
            // generic for
            let mut vars = vec![first];
            while self.eat(Tk::Comma) {
                if self.at(Tk::Name) {
                    vars.push(self.parse_name_def());
                } else {
                    self.diags.error(self.peek().span, "expected a loop variable");
                    break;
                }
            }
            self.expect(Tk::In, "expected `in` in generic `for`");
            let exprs = self.parse_expr_list();
            self.expect(Tk::Do, "expected `do` in `for` loop");
            let body = self.parse_block();
            self.expect(Tk::End, "missing `end` to close `for` loop");
            Stmt::GenericFor { vars, exprs, body, span: self.span_from(start) }
        }
    }

    fn parse_expr_stmt(&mut self, start: usize) -> Stmt {
        let expr = self.parse_suffixed();
        // Luau compound assignment: `target += value`, etc.
        if is_compound(self.kind()) {
            self.bump();
            let value = self.parse_expr();
            return Stmt::Assign {
                targets: vec![expr],
                values: vec![value],
                compound: true,
                span: self.span_from(start),
            };
        }
        if self.at(Tk::Assign) || self.at(Tk::Comma) {
            let mut targets = vec![expr];
            while self.eat(Tk::Comma) {
                targets.push(self.parse_suffixed());
            }
            self.expect(Tk::Assign, "expected `=` in assignment");
            let values = self.parse_expr_list();
            return Stmt::Assign { targets, values, compound: false, span: self.span_from(start) };
        }
        if !matches!(expr, Expr::Call { .. } | Expr::Error(_)) {
            self.diags
                .error(expr.span(), "unexpected expression (a statement must be a call or assignment)");
        }
        Stmt::ExprStmt(expr)
    }

    // --- names / params -----------------------------------------------------

    fn name_ref(&mut self) -> NameRef {
        let t = self.bump();
        NameRef { name: self.src[t.span.start..t.span.end].to_string(), span: t.span }
    }

    fn parse_name_def(&mut self) -> NameDef {
        let t = self.bump();
        let def = NameDef { name: self.src[t.span.start..t.span.end].to_string(), span: t.span };
        if self.eat(Tk::Colon) {
            self.skip_type();
        }
        def
    }

    fn parse_name_def_no_type(&mut self) -> NameDef {
        if !self.at(Tk::Name) {
            self.diags.error(self.peek().span, "expected a function name");
            return NameDef { name: String::new(), span: self.peek().span };
        }
        let t = self.bump();
        NameDef { name: self.src[t.span.start..t.span.end].to_string(), span: t.span }
    }

    fn parse_func_body(&mut self, start: usize) -> FuncBody {
        let mut params = Vec::new();
        let mut is_vararg = false;
        self.expect(Tk::LParen, "expected `(` to start function parameters");
        if !self.at(Tk::RParen) {
            loop {
                if self.at(Tk::Ellipsis) {
                    self.bump();
                    is_vararg = true;
                    if self.eat(Tk::Colon) {
                        self.skip_type();
                    }
                    break;
                } else if self.at(Tk::Name) {
                    params.push(self.parse_name_def());
                } else {
                    self.diags.error(self.peek().span, "expected a parameter name");
                    break;
                }
                if !self.eat(Tk::Comma) {
                    break;
                }
            }
        }
        self.expect(Tk::RParen, "missing `)` to close function parameters");
        if self.eat(Tk::Colon) {
            self.skip_type(); // return type annotation
        }
        let body = self.parse_block();
        self.expect(Tk::End, "missing `end` to close function");
        FuncBody { params, is_vararg, body, span: self.span_from(start) }
    }

    // --- expressions --------------------------------------------------------

    fn parse_expr_list(&mut self) -> Vec<Expr> {
        let mut list = vec![self.parse_expr()];
        while self.eat(Tk::Comma) {
            list.push(self.parse_expr());
        }
        list
    }

    fn parse_expr(&mut self) -> Expr {
        self.parse_bin(0)
    }

    fn parse_bin(&mut self, min_bp: u8) -> Expr {
        let mut lhs = self.parse_unary();
        while self.tick() {
            let Some((op, lbp, rbp)) = bin_op(self.kind()) else {
                break;
            };
            if lbp < min_bp {
                break;
            }
            self.bump();
            let rhs = self.parse_bin(rbp);
            let span = lhs.span().to(rhs.span());
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), span };
        }
        lhs
    }

    fn parse_unary(&mut self) -> Expr {
        let start = self.peek().span.start;
        let op = match self.kind() {
            Tk::Not => Some(UnOp::Not),
            Tk::Minus => Some(UnOp::Neg),
            Tk::Hash => Some(UnOp::Len),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let expr = self.parse_bin(UNARY_BP);
            let span = self.span_from(start);
            return Expr::Unary { op, expr: Box::new(expr), span };
        }
        self.parse_pow()
    }

    fn parse_pow(&mut self) -> Expr {
        let base = self.parse_suffixed();
        if self.at(Tk::Caret) {
            self.bump();
            // `^` is right-associative and binds tighter than unary.
            let rhs = self.parse_unary();
            let span = base.span().to(rhs.span());
            return Expr::Binary { op: BinOp::Pow, lhs: Box::new(base), rhs: Box::new(rhs), span };
        }
        base
    }

    /// primary expression followed by any chain of `.field`, `[index]`, calls,
    /// and `:method(...)`.
    fn parse_suffixed(&mut self) -> Expr {
        let start = self.peek().span.start;
        let mut expr = self.parse_primary();
        loop {
            match self.kind() {
                Tk::Dot => {
                    self.bump();
                    let field = if self.at(Tk::Name) {
                        self.name_ref()
                    } else {
                        self.diags.error(self.peek().span, "expected a field name after `.`");
                        NameRef { name: String::new(), span: self.peek().span }
                    };
                    expr = Expr::Field { object: Box::new(expr), field, span: self.span_from(start) };
                }
                Tk::LBracket => {
                    self.bump();
                    let key = self.parse_expr();
                    self.expect(Tk::RBracket, "missing `]` to close index");
                    expr = Expr::Index { object: Box::new(expr), key: Box::new(key), span: self.span_from(start) };
                }
                Tk::Colon => {
                    self.bump();
                    let method = if self.at(Tk::Name) {
                        Some(self.name_ref())
                    } else {
                        self.diags.error(self.peek().span, "expected a method name after `:`");
                        None
                    };
                    let args = self.parse_call_args();
                    expr = Expr::Call { callee: Box::new(expr), method, args, span: self.span_from(start) };
                }
                Tk::LParen | Tk::Str | Tk::LBrace => {
                    let args = self.parse_call_args();
                    expr = Expr::Call { callee: Box::new(expr), method: None, args, span: self.span_from(start) };
                }
                _ => break,
            }
        }
        expr
    }

    fn parse_call_args(&mut self) -> Vec<Expr> {
        match self.kind() {
            Tk::Str => {
                let t = self.bump();
                vec![Expr::Str { span: t.span }]
            }
            Tk::LBrace => vec![self.parse_table()],
            Tk::LParen => {
                self.bump();
                let mut args = Vec::new();
                if !self.at(Tk::RParen) {
                    args = self.parse_expr_list();
                }
                self.expect(Tk::RParen, "missing `)` to close call arguments");
                args
            }
            _ => {
                self.diags.error(self.peek().span, "expected call arguments");
                Vec::new()
            }
        }
    }

    fn parse_primary(&mut self) -> Expr {
        let t = self.peek();
        match t.kind {
            Tk::Nil => {
                self.bump();
                Expr::Nil(t.span)
            }
            Tk::True => {
                self.bump();
                Expr::True(t.span)
            }
            Tk::False => {
                self.bump();
                Expr::False(t.span)
            }
            Tk::Ellipsis => {
                self.bump();
                Expr::Vararg(t.span)
            }
            Tk::Number => {
                self.bump();
                Expr::Number { span: t.span }
            }
            Tk::Str => {
                self.bump();
                Expr::Str { span: t.span }
            }
            Tk::Name => Expr::Name(self.name_ref()),
            Tk::LParen => {
                let start = t.span.start;
                self.bump();
                let inner = self.parse_expr();
                self.expect(Tk::RParen, "missing `)` to close expression");
                Expr::Paren { expr: Box::new(inner), span: self.span_from(start) }
            }
            Tk::LBrace => self.parse_table(),
            Tk::Function => {
                let start = t.span.start;
                self.bump();
                let body = self.parse_func_body(start);
                Expr::Function { span: body.span, body }
            }
            _ => {
                self.diags.error(t.span, format!("unexpected `{}` in expression", self.text(t.span)));
                // Don't consume block-structuring tokens; let the block recover.
                if !self.is_block_end() && !matches!(t.kind, Tk::Eof) {
                    self.bump();
                }
                Expr::Error(t.span)
            }
        }
    }

    fn parse_table(&mut self) -> Expr {
        let start = self.peek().span.start;
        self.expect(Tk::LBrace, "expected `{`");
        let mut fields = Vec::new();
        while !self.at(Tk::RBrace) && !self.at(Tk::Eof) && self.tick() {
            let before = self.pos;
            if self.at(Tk::LBracket) {
                self.bump();
                let key = self.parse_expr();
                self.expect(Tk::RBracket, "missing `]` in table key");
                self.expect(Tk::Assign, "expected `=` after table key");
                let value = self.parse_expr();
                fields.push(TableField::Keyed { key, value });
            } else if self.at(Tk::Name) && self.peek2().kind == Tk::Assign {
                let name = self.name_ref();
                self.bump(); // `=`
                let value = self.parse_expr();
                fields.push(TableField::Named { name, value });
            } else {
                fields.push(TableField::Positional(self.parse_expr()));
            }
            if !self.eat(Tk::Comma) && !self.eat(Tk::Semicolon) {
                break;
            }
            if self.pos == before {
                self.bump();
            }
        }
        self.expect(Tk::RBrace, "missing `}` to close table");
        Expr::Table { fields, span: self.span_from(start) }
    }

    // --- type-annotation skipping ------------------------------------------

    /// Consume a type expression without building it (annotations are validated
    /// separately; here we only need to not choke on them).
    fn skip_type(&mut self) {
        if !self.tick() {
            return;
        }
        self.skip_type_atom();
        while matches!(self.kind(), Tk::Pipe | Tk::Amp) {
            self.bump();
            self.skip_type_atom();
        }
        while self.at(Tk::Question) {
            self.bump();
        }
    }

    fn skip_type_atom(&mut self) {
        match self.kind() {
            Tk::LParen => {
                self.skip_balanced(Tk::LParen, Tk::RParen);
                if self.eat(Tk::Arrow) {
                    self.skip_type();
                }
            }
            Tk::LBrace => {
                self.skip_balanced(Tk::LBrace, Tk::RBrace);
            }
            Tk::Name => {
                self.bump();
                while self.eat(Tk::Dot) {
                    self.eat(Tk::Name);
                }
                if self.at(Tk::Lt) {
                    self.skip_balanced(Tk::Lt, Tk::Gt);
                }
                // `typeof(expr)` etc.
                if self.at(Tk::LParen) {
                    self.skip_balanced(Tk::LParen, Tk::RParen);
                }
            }
            Tk::Nil | Tk::True | Tk::False | Tk::Str | Tk::Number | Tk::Ellipsis => {
                self.bump();
            }
            _ => {} // leave anything unexpected for the statement parser
        }
        while self.at(Tk::Question) {
            self.bump();
        }
    }

    fn skip_balanced(&mut self, open: Tk, close: Tk) {
        if !self.eat(open) {
            return;
        }
        let mut depth = 1;
        while depth > 0 && !self.at(Tk::Eof) && self.tick() {
            let k = self.kind();
            if k == open {
                depth += 1;
            } else if k == close {
                depth -= 1;
            }
            self.bump();
        }
    }
}

const UNARY_BP: u8 = 13;

/// Luau compound-assignment operators (`+=`, `-=`, …, `..=`).
fn is_compound(k: Tk) -> bool {
    matches!(
        k,
        Tk::PlusEq | Tk::MinusEq | Tk::StarEq | Tk::SlashEq | Tk::PercentEq | Tk::CaretEq | Tk::ConcatEq
    )
}

/// Binding powers for binary operators: (op, left_bp, right_bp). Right-assoc
/// operators (`..`, `^`) use `right_bp < left_bp`.
fn bin_op(k: Tk) -> Option<(BinOp, u8, u8)> {
    Some(match k {
        Tk::Or => (BinOp::Or, 1, 2),
        Tk::And => (BinOp::And, 3, 4),
        Tk::Lt => (BinOp::Lt, 5, 6),
        Tk::Gt => (BinOp::Gt, 5, 6),
        Tk::Le => (BinOp::Le, 5, 6),
        Tk::Ge => (BinOp::Ge, 5, 6),
        Tk::Ne => (BinOp::Ne, 5, 6),
        Tk::Eq => (BinOp::Eq, 5, 6),
        Tk::Concat => (BinOp::Concat, 8, 7),
        Tk::Plus => (BinOp::Add, 9, 10),
        Tk::Minus => (BinOp::Sub, 9, 10),
        Tk::Star => (BinOp::Mul, 11, 12),
        Tk::Slash => (BinOp::Div, 11, 12),
        Tk::Percent => (BinOp::Mod, 11, 12),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> Vec<super::super::diagnostics::Diagnostic> {
        let mut d = Diagnostics::new(src);
        let _ = parse(src, &mut d);
        d.finish()
    }

    #[test]
    fn clean_program_parses_without_errors() {
        let src = "\
local function add(a, b)
    return a + b
end
local t = { x = 1, y = 2, 3 }
for i = 1, 10 do
    print(add(i, t.x))
end
if t.x > 0 then
    print(\"pos\")
elseif t.x < 0 then
    print(\"neg\")
else
    print(\"zero\")
end
";
        assert!(parse_ok(src).is_empty(), "unexpected: {:?}", parse_ok(src));
    }

    #[test]
    fn missing_end_reported() {
        let d = parse_ok("if x then\n  print(1)\n");
        assert!(d.iter().any(|x| x.message.contains("missing `end`")), "{d:?}");
    }

    #[test]
    fn missing_paren_reported() {
        let d = parse_ok("print(1\n");
        assert!(d.iter().any(|x| x.message.contains("missing `)`")), "{d:?}");
    }

    #[test]
    fn method_call_parses() {
        let d = parse_ok("game:GetService(\"Players\")\n");
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn recovers_and_continues() {
        // A stray token shouldn't swallow the rest of the file: we still report
        // an error and keep parsing the following statements.
        let d = parse_ok("local a = 1 @ local b = 2\nprint(a, b)\n");
        assert!(!d.is_empty());
    }
}
