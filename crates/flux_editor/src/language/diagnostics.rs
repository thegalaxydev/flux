//! Lightweight diagnostics: syntax-level errors (unterminated strings,
//! unbalanced brackets) plus conservative semantic warnings (unused locals,
//! unknown called globals, wrong argument counts for known API functions).
//!
//! These are heuristics, not a real type checker — tuned to avoid false
//! positives. When a proper Luau parser is wired in, this module is the only
//! thing that needs to change.

use std::ops::Range;

use super::api::ApiDb;
use super::lex::{KEYWORDS, Tok, Token, line_col, tokenize};
use super::symbols::{SymKind, SymbolIndex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    /// Byte range in the source.
    pub range: Range<usize>,
    pub line: usize,
    pub col: usize,
}

/// Standard-library / engine names that are always in scope (superset of the
/// API DB, so we never warn "unknown global" for e.g. `select` or `os`).
const BUILTINS: &[&str] = &[
    "self", "_G", "_VERSION", "select", "next", "unpack", "rawget", "rawset", "rawequal", "rawlen",
    "setmetatable", "getmetatable", "xpcall", "collectgarbage", "newproxy", "os", "coroutine",
    "bit32", "utf8", "debug", "buffer", "tick", "gcinfo",
];

#[derive(Default)]
pub struct DiagnosticsProvider;

impl DiagnosticsProvider {
    pub fn diagnostics(&self, db: &ApiDb, idx: &SymbolIndex, src: &str) -> Vec<Diagnostic> {
        let toks = tokenize(src);
        let mut out = Vec::new();
        check_strings(src, &toks, &mut out);
        check_brackets(src, &toks, &mut out);
        check_unused_locals(src, idx, &toks, &mut out);
        check_calls(db, idx, src, &toks, &mut out);
        out.sort_by_key(|d| d.range.start);
        out
    }
}

fn push(out: &mut Vec<Diagnostic>, src: &str, severity: Severity, range: Range<usize>, msg: String) {
    let (line, col) = line_col(src, range.start);
    out.push(Diagnostic { severity, message: msg, range, line, col });
}

fn check_strings(src: &str, toks: &[Token], out: &mut Vec<Diagnostic>) {
    for t in toks {
        if matches!(t.kind, Tok::Str { terminated: false }) {
            push(out, src, Severity::Error, t.start..t.end, "unterminated string".to_string());
        }
    }
}

fn check_brackets(src: &str, toks: &[Token], out: &mut Vec<Diagnostic>) {
    let mut stack: Vec<(char, Range<usize>)> = Vec::new();
    for t in toks {
        if t.kind != Tok::Symbol {
            continue;
        }
        match t.text(src) {
            "(" | "[" | "{" => {
                stack.push((t.text(src).chars().next().unwrap(), t.start..t.end));
            }
            c @ (")" | "]" | "}") => {
                let want = match c {
                    ")" => '(',
                    "]" => '[',
                    _ => '{',
                };
                match stack.pop() {
                    Some((open, _)) if open == want => {}
                    Some((open, span)) => {
                        push(
                            out,
                            src,
                            Severity::Error,
                            t.start..t.end,
                            format!("mismatched bracket: expected to close `{open}`"),
                        );
                        // Re-push so a later matching close can still balance it.
                        stack.push((open, span));
                    }
                    None => push(
                        out,
                        src,
                        Severity::Error,
                        t.start..t.end,
                        format!("unmatched closing `{c}`"),
                    ),
                }
            }
            _ => {}
        }
    }
    for (open, span) in stack {
        push(out, src, Severity::Error, span, format!("unclosed `{open}`"));
    }
}

fn check_unused_locals(src: &str, idx: &SymbolIndex, toks: &[Token], out: &mut Vec<Diagnostic>) {
    for sym in &idx.symbols {
        if sym.kind != SymKind::Local {
            continue;
        }
        let uses = toks
            .iter()
            .filter(|t| t.kind == Tok::Ident && t.text(src) == sym.name)
            .count();
        // The declaration itself is one Ident occurrence; anything more is a use.
        if uses <= 1 {
            let end = sym.byte + sym.name.len();
            push(
                out,
                src,
                Severity::Warning,
                sym.byte..end,
                format!("unused local `{}`", sym.name),
            );
        }
    }
}

fn check_calls(db: &ApiDb, idx: &SymbolIndex, src: &str, toks: &[Token], out: &mut Vec<Diagnostic>) {
    let known = known_names(db, idx);
    for i in 0..toks.len() {
        if toks[i].kind != Tok::Ident {
            continue;
        }
        let open = i + 1;
        let is_call = toks.get(open).is_some_and(|t| t.kind == Tok::Symbol && t.text(src) == "(");
        if !is_call {
            continue;
        }
        let prev = i.checked_sub(1).map(|p| toks[p]);
        let is_member = prev.is_some_and(|p| p.kind == Tok::Symbol && matches!(p.text(src), "." | ":"));
        let is_decl = prev.is_some_and(|p| p.kind == Tok::Keyword && p.text(src) == "function");
        let name = toks[i].text(src);

        // Unknown called global: a plain `foo(` where foo isn't defined anywhere.
        if !is_member && !is_decl && !known.contains(&name) {
            push(
                out,
                src,
                Severity::Warning,
                toks[i].start..toks[i].end,
                format!("unknown global `{name}`"),
            );
            continue;
        }

        // Argument-count check for resolvable API callables.
        let func_expr = super::context::extract_base(src.as_bytes(), toks[open].start);
        let Some(entry) = db.resolve_member(&func_expr, idx) else {
            continue;
        };
        if !entry.is_callable() || entry.params.iter().any(|p| p.is_variadic()) {
            continue;
        }
        let Some((argc, close_end)) = call_args(src, toks, open) else {
            continue;
        };
        let max = entry.params.len();
        let required = entry.params.iter().filter(|p| !p.is_optional()).count();
        if argc > max {
            push(
                out,
                src,
                Severity::Warning,
                toks[i].start..close_end,
                format!("`{name}` takes at most {max} argument(s), but {argc} were given"),
            );
        } else if argc < required {
            push(
                out,
                src,
                Severity::Warning,
                toks[i].start..close_end,
                format!("`{name}` takes at least {required} argument(s), but {argc} were given"),
            );
        }
    }
}

/// Count the arguments of a call whose `(` is `toks[open]`. Returns the argument
/// count and the byte offset just past the matching `)`.
fn call_args(src: &str, toks: &[Token], open: usize) -> Option<(usize, usize)> {
    let mut depth = 0i32;
    let mut commas = 0usize;
    let mut any = false;
    for t in &toks[open..] {
        if t.kind == Tok::Symbol {
            match t.text(src) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    depth -= 1;
                    if depth == 0 {
                        let count = if any { commas + 1 } else { 0 };
                        return Some((count, t.end));
                    }
                }
                "," if depth == 1 => commas += 1,
                _ if depth >= 1 => any = true,
                _ => {}
            }
        } else if depth >= 1 {
            any = true;
        }
    }
    None
}

fn known_names<'a>(db: &'a ApiDb, idx: &'a SymbolIndex) -> Vec<&'a str> {
    let mut set: Vec<&str> = Vec::new();
    set.extend(KEYWORDS.iter().copied());
    set.extend(BUILTINS.iter().copied());
    set.extend(db.globals.keys().map(String::as_str));
    set.extend(idx.symbols.iter().map(|s| s.name.as_str()));
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diags(src: &str) -> Vec<Diagnostic> {
        let db = ApiDb::load();
        let idx = SymbolIndex::build(src);
        DiagnosticsProvider.diagnostics(&db, &idx, src)
    }

    #[test]
    fn flags_unterminated_string() {
        let d = diags("local s = \"oops\n");
        assert!(d.iter().any(|x| x.severity == Severity::Error && x.message.contains("string")));
    }

    #[test]
    fn flags_unbalanced_parens() {
        let d = diags("print((1)\n");
        assert!(d.iter().any(|x| x.severity == Severity::Error && x.message.contains("unclosed")));
    }

    #[test]
    fn clean_code_has_no_errors() {
        let d = diags("local x = 1\nprint(x)\nlocal v = Vec2.new(1, 2)\nprint(v)\n");
        assert!(d.iter().all(|x| x.severity != Severity::Error), "unexpected: {d:?}");
    }

    #[test]
    fn flags_too_many_arguments() {
        let d = diags("local v = Vec2.new(1, 2, 3)\nprint(v)\n");
        assert!(d.iter().any(|x| x.message.contains("at most 2")), "{d:?}");
    }

    #[test]
    fn flags_unused_local() {
        let d = diags("local unusedThing = 5\nprint(1)\n");
        assert!(d.iter().any(|x| x.message.contains("unused local `unusedThing`")));
    }

    #[test]
    fn flags_unknown_called_global() {
        let d = diags("doTheThing()\n");
        assert!(d.iter().any(|x| x.message.contains("unknown global `doTheThing`")));
    }

    #[test]
    fn does_not_flag_known_calls() {
        let d = diags("print(1)\nwarn(2)\nlocal t = {}\ntable.insert(t, 3)\n");
        assert!(d.iter().all(|x| !x.message.contains("unknown global")), "{d:?}");
    }
}
