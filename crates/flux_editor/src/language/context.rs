//! Cursor-context detection: given the source and a byte offset, work out what
//! the user is doing (typing a member after `.`/`:`, a bare identifier, or
//! sitting inside a call's argument list). Pure backward scanning over bytes —
//! identifiers and separators are ASCII, so this stays simple.

use std::ops::Range;

/// What the cursor is positioned to complete.
#[derive(Debug, Clone, PartialEq)]
pub enum Ctx {
    /// After `base.` / `base:` — completing a member of `base`.
    Member {
        base: String,
        sep: char,
        prefix: String,
        replace: Range<usize>,
    },
    /// A bare identifier being typed.
    Ident { prefix: String, replace: Range<usize> },
    None,
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Determine the completion context at `cursor` (a byte offset).
pub fn completion_context(src: &str, cursor: usize) -> Ctx {
    let b = src.as_bytes();
    let cursor = cursor.min(b.len());

    // Partial identifier immediately left of the cursor.
    let mut start = cursor;
    while start > 0 && is_ident(b[start - 1]) {
        start -= 1;
    }
    let prefix = src[start..cursor].to_string();

    // Is it a member access (something.  something:)?
    if start > 0 && (b[start - 1] == b'.' || b[start - 1] == b':') {
        // A `..` (concat) is not member access.
        if b[start - 1] == b'.' && start >= 2 && b[start - 2] == b'.' {
            return ident_or_none(prefix, start..cursor);
        }
        let sep = b[start - 1] as char;
        let base = extract_base(b, start - 1);
        if base.is_empty() {
            return ident_or_none(prefix, start..cursor);
        }
        return Ctx::Member {
            base,
            sep,
            prefix,
            replace: start..cursor,
        };
    }

    ident_or_none(prefix, start..cursor)
}

fn ident_or_none(prefix: String, replace: Range<usize>) -> Ctx {
    if prefix.is_empty() {
        Ctx::None
    } else {
        Ctx::Ident { prefix, replace }
    }
}

/// The base expression ending just before `sep` (the index of `.`/`:`).
/// Handles dotted/method chains and trailing calls: `game:GetService("X")`.
pub fn extract_base(b: &[u8], sep: usize) -> String {
    let mut j = sep;
    loop {
        if j == 0 {
            break;
        }
        let c = b[j - 1];
        if c == b')' || c == b']' {
            match match_open(b, j - 1) {
                Some(open) => {
                    j = open;
                    continue;
                }
                None => break,
            }
        }
        if is_ident(c) || c == b'.' || c == b':' {
            j -= 1;
            continue;
        }
        break;
    }
    String::from_utf8_lossy(&b[j..sep]).trim().to_string()
}

/// Index of the bracket matching the closing bracket at `close`. Approximate:
/// ignores brackets inside strings (acceptable for a lightweight editor).
fn match_open(b: &[u8], close: usize) -> Option<usize> {
    let (open, shut) = match b[close] {
        b')' => (b'(', b')'),
        b']' => (b'[', b']'),
        b'}' => (b'{', b'}'),
        _ => return None,
    };
    let mut depth = 0i32;
    let mut i = close as isize;
    while i >= 0 {
        let c = b[i as usize];
        if c == shut {
            depth += 1;
        } else if c == open {
            depth -= 1;
            if depth == 0 {
                return Some(i as usize);
            }
        }
        i -= 1;
    }
    None
}

/// The identifier the cursor sits on/next to, and whether it is a member access
/// (with its base expression). Used for hover.
#[derive(Debug, Clone, PartialEq)]
pub struct WordTarget {
    pub word: String,
    pub range: Range<usize>,
    pub is_member: bool,
    pub base: String,
}

pub fn word_at(src: &str, byte: usize) -> Option<WordTarget> {
    let b = src.as_bytes();
    let byte = byte.min(b.len());
    if b.is_empty() {
        return None;
    }
    // Find a word boundary around `byte` (prefer the char under the cursor,
    // falling back to the one just left of it).
    let mut probe = byte;
    if probe >= b.len() || !is_ident(b[probe]) {
        if probe == 0 || !is_ident(b[probe - 1]) {
            return None;
        }
        probe -= 1;
    }
    let mut start = probe;
    while start > 0 && is_ident(b[start - 1]) {
        start -= 1;
    }
    let mut end = probe;
    while end < b.len() && is_ident(b[end]) {
        end += 1;
    }
    let word = src[start..end].to_string();
    let is_member = start > 0 && (b[start - 1] == b'.' || b[start - 1] == b':');
    let base = if is_member {
        extract_base(b, start - 1)
    } else {
        String::new()
    };
    Some(WordTarget {
        word,
        range: start..end,
        is_member,
        base,
    })
}

/// If the cursor is inside a call's argument list, return the function
/// expression and the active (zero-based) parameter index.
pub fn enclosing_call(src: &str, cursor: usize) -> Option<(String, usize)> {
    let b = src.as_bytes();
    let cursor = cursor.min(b.len());
    let mut depth = 0i32;
    let mut commas = 0usize;
    let mut i = cursor;
    while i > 0 {
        i -= 1;
        match b[i] {
            b')' | b']' | b'}' => depth += 1,
            b'(' => {
                if depth == 0 {
                    let expr = extract_base(b, i);
                    if expr.is_empty() {
                        return None;
                    }
                    return Some((expr, commas));
                }
                depth -= 1;
            }
            b'[' | b'{' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            b',' if depth == 0 => commas += 1,
            b';' if depth == 0 => return None,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_member_context() {
        let src = "local v = Vec2.ne";
        let ctx = completion_context(src, src.len());
        match ctx {
            Ctx::Member { base, sep, prefix, .. } => {
                assert_eq!(base, "Vec2");
                assert_eq!(sep, '.');
                assert_eq!(prefix, "ne");
            }
            other => panic!("expected member, got {other:?}"),
        }
    }

    #[test]
    fn member_context_after_getservice_call() {
        let src = "game:GetService(\"Input\").";
        let ctx = completion_context(src, src.len());
        match ctx {
            Ctx::Member { base, prefix, .. } => {
                assert_eq!(base, "game:GetService(\"Input\")");
                assert_eq!(prefix, "");
            }
            other => panic!("expected member, got {other:?}"),
        }
    }

    #[test]
    fn concat_is_not_member() {
        let ctx = completion_context("a ..b", 5);
        assert!(matches!(ctx, Ctx::Ident { .. }));
    }

    #[test]
    fn word_at_finds_member() {
        let src = "Vec2.new";
        let w = word_at(src, 6).unwrap();
        assert_eq!(w.word, "new");
        assert!(w.is_member);
        assert_eq!(w.base, "Vec2");
    }

    #[test]
    fn enclosing_call_counts_params() {
        let src = "Vec2.new(1, ";
        let (expr, active) = enclosing_call(src, src.len()).unwrap();
        assert_eq!(expr, "Vec2.new");
        assert_eq!(active, 1);
    }
}
