//! Lexical scope + symbol table used by the semantic analyzer.
//!
//! Declarations live in a flat arena that outlives their frame, so we can report
//! unused symbols when a frame closes. Frames map names to the current binding
//! for shadowing/duplicate detection and resolution.

use std::collections::HashMap;

use super::diagnostics::Span;
use super::types::Ty;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymKind {
    Local,
    Param,
    Function,
    ForVar,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymKind,
    pub span: Span,
    pub ty: Ty,
    pub used: bool,
    /// Reassigned after declaration → its inferred type can't be trusted.
    pub reassigned: bool,
}

struct Frame {
    names: HashMap<String, usize>,
    decls: Vec<usize>,
    /// A function-body boundary (kept for future flow/closure analysis).
    #[allow(dead_code)]
    is_function: bool,
}

pub struct Scope {
    arena: Vec<Symbol>,
    frames: Vec<Frame>,
}

/// Result of declaring a name.
pub enum Decl {
    /// Fresh declaration.
    New,
    /// A name already declared in the same frame.
    Duplicate,
    /// Shadows a name from an outer frame.
    Shadows,
}

impl Default for Scope {
    fn default() -> Self {
        Scope {
            arena: Vec::new(),
            frames: vec![Frame { names: HashMap::new(), decls: Vec::new(), is_function: true }],
        }
    }
}

impl Scope {
    pub fn push(&mut self, is_function: bool) {
        self.frames.push(Frame { names: HashMap::new(), decls: Vec::new(), is_function });
    }

    /// Close the top frame, returning the symbols declared in it (for unused
    /// checks).
    pub fn pop(&mut self) -> Vec<Symbol> {
        let frame = self.frames.pop().expect("popped root scope");
        frame.decls.iter().map(|&i| self.arena[i].clone()).collect()
    }

    pub fn declare(&mut self, name: &str, kind: SymKind, span: Span, ty: Ty) -> Decl {
        let dup = self.frames.last().unwrap().names.contains_key(name);
        let shadow = !dup
            && self.frames[..self.frames.len() - 1]
                .iter()
                .any(|f| f.names.contains_key(name));

        let idx = self.arena.len();
        self.arena.push(Symbol {
            name: name.to_string(),
            kind,
            span,
            ty,
            used: false,
            reassigned: false,
        });
        let frame = self.frames.last_mut().unwrap();
        frame.names.insert(name.to_string(), idx);
        frame.decls.push(idx);

        match (dup, shadow) {
            (true, _) => Decl::Duplicate,
            (false, true) => Decl::Shadows,
            _ => Decl::New,
        }
    }

    fn resolve_idx(&self, name: &str) -> Option<usize> {
        self.frames.iter().rev().find_map(|f| f.names.get(name).copied())
    }

    pub fn is_declared(&self, name: &str) -> bool {
        self.resolve_idx(name).is_some()
    }

    pub fn mark_used(&mut self, name: &str) {
        if let Some(i) = self.resolve_idx(name) {
            self.arena[i].used = true;
        }
    }

    pub fn mark_reassigned(&mut self, name: &str) {
        if let Some(i) = self.resolve_idx(name) {
            self.arena[i].reassigned = true;
        }
    }

    /// The trusted type of a name: `Unknown` if reassigned or not found.
    pub fn type_of(&self, name: &str) -> Ty {
        match self.resolve_idx(name) {
            Some(i) if !self.arena[i].reassigned => self.arena[i].ty.clone(),
            _ => Ty::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp() -> Span {
        Span::new(0, 1)
    }

    #[test]
    fn duplicate_in_same_frame() {
        let mut s = Scope::default();
        assert!(matches!(s.declare("x", SymKind::Local, sp(), Ty::Number), Decl::New));
        assert!(matches!(s.declare("x", SymKind::Local, sp(), Ty::Number), Decl::Duplicate));
    }

    #[test]
    fn shadowing_across_frames() {
        let mut s = Scope::default();
        s.declare("x", SymKind::Local, sp(), Ty::Number);
        s.push(false);
        assert!(matches!(s.declare("x", SymKind::Local, sp(), Ty::String), Decl::Shadows));
        assert_eq!(s.type_of("x"), Ty::String);
        s.pop();
        assert_eq!(s.type_of("x"), Ty::Number);
    }

    #[test]
    fn reassignment_clears_type() {
        let mut s = Scope::default();
        s.declare("x", SymKind::Local, sp(), Ty::Number);
        assert_eq!(s.type_of("x"), Ty::Number);
        s.mark_reassigned("x");
        assert_eq!(s.type_of("x"), Ty::Unknown);
    }

    #[test]
    fn unused_reported_on_pop() {
        let mut s = Scope::default();
        s.push(false);
        s.declare("used", SymKind::Local, sp(), Ty::Number);
        s.declare("dead", SymKind::Local, sp(), Ty::Number);
        s.mark_used("used");
        let closed = s.pop();
        let dead: Vec<_> = closed.iter().filter(|s| !s.used).map(|s| s.name.as_str()).collect();
        assert_eq!(dead, ["dead"]);
    }
}
