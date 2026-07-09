//! Symbol indexing for the active file: local variables, functions, parameters,
//! for-loop variables, and service/library variables with an inferred type.
//!
//! Lightweight token-based scanning (no AST). Isolated here so completion/hover
//! can ask "what's declared in this file?" and "what type is this variable?"
//! without knowing how it was discovered.

use std::collections::HashMap;

use super::lex::{Tok, Token, line_col, tokenize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymKind {
    Local,
    Function,
    Param,
    ForVar,
}

impl SymKind {
    pub fn label(self) -> &'static str {
        match self {
            SymKind::Local => "local",
            SymKind::Function => "function",
            SymKind::Param => "parameter",
            SymKind::ForVar => "loop variable",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymKind,
    /// Inferred value-type hint (see [`SymbolIndex::var_type`]), if any.
    pub type_hint: Option<String>,
    /// 1-based line of the declaration.
    pub line: usize,
    /// Byte offset of the declaration name.
    pub byte: usize,
}

#[derive(Debug, Default)]
pub struct SymbolIndex {
    pub symbols: Vec<Symbol>,
    /// name -> latest inferred type hint.
    types: HashMap<String, String>,
}

impl SymbolIndex {
    pub fn build(src: &str) -> Self {
        let toks = tokenize(src);
        let mut idx = SymbolIndex::default();
        let mut i = 0;
        while i < toks.len() {
            let t = toks[i];
            if t.kind == Tok::Keyword {
                match t.text(src) {
                    "local" => i = idx.scan_local(src, &toks, i),
                    "function" => i = idx.scan_function(src, &toks, i, false),
                    "for" => i = idx.scan_for(src, &toks, i),
                    _ => i += 1,
                }
            } else {
                i += 1;
            }
        }
        idx
    }

    fn push(&mut self, src: &str, tok: &Token, kind: SymKind, hint: Option<String>) {
        let name = tok.text(src).to_string();
        if name == "_" {
            return;
        }
        let (line, _) = line_col(src, tok.start);
        if let Some(h) = &hint {
            self.types.insert(name.clone(), h.clone());
        }
        self.symbols.push(Symbol {
            name,
            kind,
            type_hint: hint,
            line,
            byte: tok.start,
        });
    }

    /// `local a, b = ...` / `local function f(...)`.
    fn scan_local(&mut self, src: &str, toks: &[Token], at: usize) -> usize {
        let mut i = at + 1;
        if i < toks.len() && toks[i].kind == Tok::Keyword && toks[i].text(src) == "function" {
            return self.scan_function(src, toks, i, true);
        }
        // Collect comma-separated names up to `=` or a non-name token.
        let name_start = i;
        let mut names: Vec<usize> = Vec::new();
        loop {
            if i < toks.len() && toks[i].kind == Tok::Ident {
                names.push(i);
                i += 1;
                if i < toks.len() && toks[i].kind == Tok::Symbol && toks[i].text(src) == "," {
                    i += 1;
                    continue;
                }
            }
            break;
        }
        // Optional `= rhs` to infer the first variable's type.
        let mut first_hint = None;
        if i < toks.len() && toks[i].kind == Tok::Symbol && toks[i].text(src) == "=" {
            first_hint = infer_rhs_type(src, toks, i + 1);
        }
        for (n, &ti) in names.iter().enumerate() {
            let hint = if n == 0 { first_hint.clone() } else { None };
            self.push(src, &toks[ti], SymKind::Local, hint);
        }
        name_start.max(i)
    }

    /// `function name(...)` or `local function name(...)`. `at` points at
    /// `function`. Records the function name and its parameters.
    fn scan_function(&mut self, src: &str, toks: &[Token], at: usize, _local: bool) -> usize {
        let mut i = at + 1;
        // Name chain: ident (('.' | ':') ident)* — record the final segment.
        let mut last_name: Option<usize> = None;
        while i < toks.len() {
            if toks[i].kind == Tok::Ident {
                last_name = Some(i);
                i += 1;
                if i < toks.len()
                    && toks[i].kind == Tok::Symbol
                    && matches!(toks[i].text(src), "." | ":")
                {
                    i += 1;
                    continue;
                }
            }
            break;
        }
        if let Some(n) = last_name {
            self.push(src, &toks[n], SymKind::Function, None);
        }
        // Parameters: everything between the next `(` and its `)`.
        if i < toks.len() && toks[i].kind == Tok::Symbol && toks[i].text(src) == "(" {
            i += 1;
            while i < toks.len() {
                let t = toks[i];
                if t.kind == Tok::Symbol && t.text(src) == ")" {
                    i += 1;
                    break;
                }
                if t.kind == Tok::Ident {
                    self.push(src, &t, SymKind::Param, None);
                }
                i += 1;
            }
        }
        i
    }

    /// `for i = ...` or `for k, v in ...`.
    fn scan_for(&mut self, src: &str, toks: &[Token], at: usize) -> usize {
        let mut i = at + 1;
        while i < toks.len() {
            let t = toks[i];
            if t.kind == Tok::Ident {
                self.push(src, &t, SymKind::ForVar, None);
                i += 1;
                if i < toks.len() && toks[i].kind == Tok::Symbol && toks[i].text(src) == "," {
                    i += 1;
                    continue;
                }
            }
            break;
        }
        // Stop at `=` or `in`; the loop body is scanned by the outer walk.
        i
    }

    /// Latest inferred type hint for `name`. Hints are either an API type name
    /// (`"Vec2"`, `"DataStore"`) or `"service:<Name>"` for a `GetService` result,
    /// which the API layer maps to a concrete type.
    pub fn var_type(&self, name: &str) -> Option<String> {
        self.types.get(name).cloned()
    }

    pub fn is_defined(&self, name: &str) -> bool {
        self.symbols.iter().any(|s| s.name == name)
    }
}

/// Infer a value type from the tokens directly after `=`.
fn infer_rhs_type(src: &str, toks: &[Token], start: usize) -> Option<String> {
    let head = toks.get(start)?;
    if head.kind != Tok::Ident {
        return None;
    }
    let head_name = head.text(src);
    let sep = toks.get(start + 1).filter(|t| t.kind == Tok::Symbol);
    let member = toks.get(start + 2).filter(|t| t.kind == Tok::Ident);

    match (head_name, sep.map(|t| t.text(src)), member.map(|t| t.text(src))) {
        // Constructor libraries return their own type.
        ("Vec2", Some("."), _) => Some("Vec2".into()),
        ("Color", Some("."), _) => Some("Color".into()),
        ("UDim", Some("."), _) => Some("UDim".into()),
        ("UDim2", Some("."), _) => Some("UDim2".into()),
        // game:GetService("Name") -> service type (resolved later).
        (_, Some(":"), Some("GetService")) => {
            service_arg(src, toks, start + 3).map(|s| format!("service:{s}"))
        }
        // store = DataStoreService:GetDataStore(...)
        (_, Some(":"), Some("GetDataStore")) => Some("DataStore".into()),
        _ => None,
    }
}

/// The first string-literal argument of a call starting at `paren` (`(`).
fn service_arg(src: &str, toks: &[Token], paren: usize) -> Option<String> {
    let open = toks.get(paren)?;
    if !(open.kind == Tok::Symbol && open.text(src) == "(") {
        return None;
    }
    let arg = toks.get(paren + 1)?;
    if let Tok::Str { .. } = arg.kind {
        let raw = arg.text(src);
        Some(raw.trim_matches(['"', '\'']).to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_locals_functions_params() {
        let src = "local speed = 5\nlocal function move(dx, dy)\n  return dx + dy\nend\n";
        let idx = SymbolIndex::build(src);
        assert!(idx.symbols.iter().any(|s| s.name == "speed" && s.kind == SymKind::Local));
        assert!(idx.symbols.iter().any(|s| s.name == "move" && s.kind == SymKind::Function));
        assert!(idx.symbols.iter().any(|s| s.name == "dx" && s.kind == SymKind::Param));
        assert!(idx.symbols.iter().any(|s| s.name == "dy" && s.kind == SymKind::Param));
    }

    #[test]
    fn infers_service_and_constructor_types() {
        let src = "local Input = game:GetService(\"Input\")\nlocal v = Vec2.new(1, 2)\n";
        let idx = SymbolIndex::build(src);
        assert_eq!(idx.var_type("Input").as_deref(), Some("service:Input"));
        assert_eq!(idx.var_type("v").as_deref(), Some("Vec2"));
    }

    #[test]
    fn indexes_for_loop_vars() {
        let idx = SymbolIndex::build("for k, part in pairs(t) do end");
        assert!(idx.symbols.iter().any(|s| s.name == "k" && s.kind == SymKind::ForVar));
        assert!(idx.symbols.iter().any(|s| s.name == "part" && s.kind == SymKind::ForVar));
    }
}
