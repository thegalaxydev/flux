//! Lightweight structural/nominal type inference. Deliberately conservative:
//! it only produces a concrete type when confident, and returns [`Ty::Unknown`]
//! otherwise so the analyzer never warns on something it can't prove.

use indexmap::IndexMap;

use super::api::Entry;
use super::ast::{BinOp, Expr, UnOp};
use super::builtin::Builtins;
use super::scope::Scope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    /// Not known well enough to reason about — never triggers a warning.
    Unknown,
    Nil,
    Bool,
    Number,
    String,
    Function,
    Table,
    /// A documented global namespace/library (inline members), e.g. `math`.
    Library(String),
    /// A documented value type (members in the type table), e.g. `Instance`.
    Named(String),
}

impl Ty {
    /// A human phrase for messages, e.g. "a number".
    pub fn describe(&self) -> String {
        match self {
            Ty::Unknown => "a value".into(),
            Ty::Nil => "nil".into(),
            Ty::Bool => "a boolean".into(),
            Ty::Number => "a number".into(),
            Ty::String => "a string".into(),
            Ty::Function => "a function".into(),
            Ty::Table => "a table".into(),
            Ty::Library(n) | Ty::Named(n) => format!("a {n}"),
        }
    }

    /// Whether indexing (`.x` / `[x]`) is meaningful. Strings are indexable via
    /// their metatable; unknowns are assumed fine.
    pub fn is_indexable(&self) -> bool {
        !matches!(self, Ty::Number | Ty::Bool | Ty::Nil | Ty::Function)
    }

    /// Whether calling is plausible. Only clearly-non-callable primitives fail.
    pub fn is_callable(&self) -> bool {
        !matches!(self, Ty::Number | Ty::Bool | Ty::Nil | Ty::String)
    }

}

/// Infer the type of `expr` in `scope`. `src` is the full source (needed to read
/// string-literal arguments, e.g. the service name in `GetService`).
pub fn type_of(expr: &Expr, scope: &Scope, b: &Builtins, src: &str) -> Ty {
    match expr {
        Expr::Nil(_) => Ty::Nil,
        Expr::True(_) | Expr::False(_) => Ty::Bool,
        Expr::Number { .. } => Ty::Number,
        Expr::Str { .. } => Ty::String,
        Expr::Vararg(_) => Ty::Unknown,
        Expr::Paren { expr, .. } => type_of(expr, scope, b, src),
        Expr::Function { .. } => Ty::Function,
        Expr::Table { .. } => Ty::Table,
        Expr::Unary { op, .. } => match op {
            UnOp::Not => Ty::Bool,
            UnOp::Len | UnOp::Neg => Ty::Number,
        },
        Expr::Binary { op, rhs, .. } => match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => Ty::Number,
            BinOp::Concat => Ty::String,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => Ty::Bool,
            // `and`/`or` yield one of their operands; only trust the rhs loosely.
            BinOp::And | BinOp::Or => {
                let r = type_of(rhs, scope, b, src);
                if r == Ty::Unknown { Ty::Unknown } else { r }
            }
        },
        Expr::Name(n) => name_ty(&n.name, scope, b),
        Expr::Field { object, field, .. } => {
            let obj = type_of(object, scope, b, src);
            match member_entry(&obj, &field.name, b) {
                Some(e) => entry_ty(e, b),
                None => Ty::Unknown,
            }
        }
        Expr::Index { .. } => Ty::Unknown,
        Expr::Call { callee, method, args, .. } => {
            call_ty(callee, method.as_ref(), args, scope, b, src)
        }
        Expr::Error(_) => Ty::Unknown,
    }
}

fn name_ty(name: &str, scope: &Scope, b: &Builtins) -> Ty {
    if scope.is_declared(name) {
        return scope.type_of(name);
    }
    match b.global(name) {
        Some(g) if g.members.is_some() => Ty::Library(name.to_string()),
        Some(g) => match &g.ty {
            Some(t) => value_ty(t),
            None if g.is_callable() => Ty::Function,
            None => Ty::Unknown,
        },
        None => Ty::Unknown,
    }
}

/// The type produced by reading a member (a property's value type; a method is a
/// function value).
fn entry_ty(e: &Entry, _b: &Builtins) -> Ty {
    if e.is_callable() {
        Ty::Function
    } else {
        e.ty.as_deref().map(value_ty).unwrap_or(Ty::Unknown)
    }
}

/// The type produced by *calling* something.
fn call_ty(
    callee: &Expr,
    method: Option<&super::ast::NameRef>,
    args: &[Expr],
    scope: &Scope,
    b: &Builtins,
    src: &str,
) -> Ty {
    // `X:GetService("Name")` → that service's type.
    if let Some(m) = method {
        if m.name == "GetService" {
            if let Some(Expr::Str { span }) = args.first() {
                let literal = src[span.range()].trim_matches(['"', '\'']);
                if let Some(ty) = b.service_type(literal) {
                    return Ty::Named(ty);
                }
            }
        }
    }
    match resolve_callable(callee, method, scope, b, src) {
        Some(e) => e.returns.as_deref().map(value_ty).unwrap_or(Ty::Unknown),
        None => Ty::Unknown,
    }
}

/// Resolve the [`Entry`] for a callable expression `callee(:method)?`.
pub fn resolve_callable<'a>(
    callee: &Expr,
    method: Option<&super::ast::NameRef>,
    scope: &Scope,
    b: &'a Builtins,
    src: &str,
) -> Option<&'a Entry> {
    if let Some(m) = method {
        let recv = type_of(callee, scope, b, src);
        return member_entry(&recv, &m.name, b).filter(|e| e.is_callable());
    }
    match callee {
        Expr::Name(n) if !scope.is_declared(&n.name) => b.global(&n.name).filter(|e| e.is_callable()),
        Expr::Field { object, field, .. } => {
            let recv = type_of(object, scope, b, src);
            member_entry(&recv, &field.name, b).filter(|e| e.is_callable())
        }
        _ => None,
    }
}

/// The member map available for a type, if documented.
pub fn members<'a>(ty: &Ty, b: &'a Builtins) -> Option<&'a IndexMap<String, Entry>> {
    match ty {
        Ty::Library(n) => b.library_members(n),
        Ty::Named(n) => b.type_members(n),
        _ => None,
    }
}

fn member_entry<'a>(ty: &Ty, name: &str, b: &'a Builtins) -> Option<&'a Entry> {
    members(ty, b).and_then(|m| m.get(name))
}

/// Map a documented type string to a [`Ty`].
fn value_ty(t: &str) -> Ty {
    let t = t.trim();
    match t {
        "number" => Ty::Number,
        "string" => Ty::String,
        "boolean" => Ty::Bool,
        "nil" | "()" => Ty::Nil,
        "any" | "..." => Ty::Unknown,
        "function" => Ty::Function,
        "table" => Ty::Table,
        _ if t.starts_with('{') || t.starts_with('(') => Ty::Table,
        // A named API type (Instance, Vec2, Signal, EnumItem, …). If it isn't a
        // real type, member lookups just return None → Unknown behaviour.
        _ => Ty::Named(t.split('?').next().unwrap_or(t).trim().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::ast::NameRef;
    use crate::language::diagnostics::Span;

    fn name(s: &str) -> Expr {
        Expr::Name(NameRef { name: s.into(), span: Span::new(0, 0) })
    }

    #[test]
    fn literals_and_ops() {
        let s = Scope::default();
        let b = Builtins::load();
        assert_eq!(type_of(&Expr::Number { span: Span::default() }, &s, &b, ""), Ty::Number);
        let add = Expr::Binary {
            op: BinOp::Add,
            lhs: Box::new(Expr::Number { span: Span::default() }),
            rhs: Box::new(Expr::Number { span: Span::default() }),
            span: Span::default(),
        };
        assert_eq!(type_of(&add, &s, &b, ""), Ty::Number);
    }

    #[test]
    fn global_namespaces_and_types() {
        let s = Scope::default();
        let b = Builtins::load();
        assert_eq!(type_of(&name("game"), &s, &b, ""), Ty::Named("Instance".into()));
        assert_eq!(type_of(&name("Enum"), &s, &b, ""), Ty::Library("Enum".into()));
    }
}
