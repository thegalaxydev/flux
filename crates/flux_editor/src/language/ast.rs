//! Abstract syntax tree for the Luau subset the analyzer understands. Nodes
//! carry byte [`Span`]s so every diagnostic can point at real source.
//!
//! The parser is error-recovering: malformed input still yields a tree with
//! [`Stmt::Error`] / [`Expr::Error`] placeholders so semantic analysis can keep
//! going around the damage.

use super::diagnostics::Span;

#[derive(Debug, Default)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

/// A declared name (local, parameter, loop variable), with an optional type
/// annotation span (the annotation itself is parsed and skipped for now).
#[derive(Debug, Clone)]
pub struct NameDef {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct NameRef {
    pub name: String,
    pub span: Span,
}

#[derive(Debug)]
pub struct FuncBody {
    pub params: Vec<NameDef>,
    /// Whether the parameter list ends in `...` (kept for future vararg checks).
    #[allow(dead_code)]
    pub is_vararg: bool,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug)]
pub enum Stmt {
    Local { names: Vec<NameDef>, values: Vec<Expr>, span: Span },
    LocalFunction { name: NameDef, body: FuncBody, span: Span },
    Function { path: Vec<NameRef>, method: Option<NameRef>, body: FuncBody, span: Span },
    Assign { targets: Vec<Expr>, values: Vec<Expr>, compound: bool, span: Span },
    /// A bare expression used as a statement (only calls are valid Lua).
    ExprStmt(Expr),
    Do { body: Block, span: Span },
    While { cond: Expr, body: Block, span: Span },
    Repeat { body: Block, cond: Expr, span: Span },
    If { arms: Vec<(Expr, Block)>, else_block: Option<Block>, span: Span },
    NumericFor { var: NameDef, start: Expr, end: Expr, step: Option<Expr>, body: Block, span: Span },
    GenericFor { vars: Vec<NameDef>, exprs: Vec<Expr>, body: Block, span: Span },
    Return { values: Vec<Expr>, span: Span },
    Break { span: Span },
    Continue { span: Span },
    /// `type X = ...` — parsed and skipped (kept so it doesn't confuse flow).
    TypeAlias { span: Span },
    Error { span: Span },
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Local { span, .. }
            | Stmt::LocalFunction { span, .. }
            | Stmt::Function { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::Do { span, .. }
            | Stmt::While { span, .. }
            | Stmt::Repeat { span, .. }
            | Stmt::If { span, .. }
            | Stmt::NumericFor { span, .. }
            | Stmt::GenericFor { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Break { span }
            | Stmt::Continue { span }
            | Stmt::TypeAlias { span }
            | Stmt::Error { span } => *span,
            Stmt::ExprStmt(e) => e.span(),
        }
    }
}

#[derive(Debug)]
pub enum Expr {
    Nil(Span),
    True(Span),
    False(Span),
    Vararg(Span),
    Number { span: Span },
    Str { span: Span },
    Name(NameRef),
    /// `object[key]`
    Index { object: Box<Expr>, key: Box<Expr>, span: Span },
    /// `object.field`
    Field { object: Box<Expr>, field: NameRef, span: Span },
    /// `callee(args)` or `callee:method(args)`
    Call { callee: Box<Expr>, method: Option<NameRef>, args: Vec<Expr>, span: Span },
    Table { fields: Vec<TableField>, span: Span },
    Function { body: FuncBody, span: Span },
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr>, span: Span },
    Unary { op: UnOp, expr: Box<Expr>, span: Span },
    Paren { expr: Box<Expr>, span: Span },
    Error(Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Nil(s)
            | Expr::True(s)
            | Expr::False(s)
            | Expr::Vararg(s)
            | Expr::Error(s) => *s,
            Expr::Number { span }
            | Expr::Str { span }
            | Expr::Index { span, .. }
            | Expr::Field { span, .. }
            | Expr::Call { span, .. }
            | Expr::Table { span, .. }
            | Expr::Function { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Paren { span, .. } => *span,
            Expr::Name(n) => n.span,
        }
    }
}

#[derive(Debug)]
pub enum TableField {
    Positional(Expr),
    Named { name: NameRef, value: Expr },
    Keyed { key: Expr, value: Expr },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Len,
}
