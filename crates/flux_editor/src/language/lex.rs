//! Minimal Luau tokenizer shared by the language-service providers.
//!
//! This is deliberately lightweight (byte-level scanning, no AST). It is the
//! single place that understands Luau lexical structure — strings, comments,
//! numbers, identifiers, brackets — so the completion / hover / diagnostics
//! providers never re-implement it and it can later be swapped for a real
//! Luau parser without touching them.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Ident,
    Keyword,
    Number,
    /// A quoted or long (`[[ ]]`) string. `terminated` is false if it ran to EOF
    /// or (for quotes) hit a newline without closing.
    Str {
        terminated: bool,
    },
    Comment,
    Symbol,
}

#[derive(Debug, Clone, Copy)]
pub struct Token {
    pub kind: Tok,
    /// Byte offsets into the source, `start..end`.
    pub start: usize,
    pub end: usize,
}

impl Token {
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start..self.end]
    }
}

pub const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while", "continue",
    "type", "typeof", "export",
];

/// Tokenizes `src`. Whitespace is skipped (not emitted). Every non-whitespace
/// byte is covered by exactly one token.
pub fn tokenize(src: &str) -> Vec<Token> {
    let b = src.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut i = 0;

    while i < n {
        let c = b[i];
        let next = if i + 1 < n { b[i + 1] } else { 0 };

        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Comments: -- line, or --[[ block ]].
        if c == b'-' && next == b'-' {
            let start = i;
            if i + 3 < n && b[i + 2] == b'[' && b[i + 3] == b'[' {
                let end = find2(b, i + 4, b']', b']').map(|e| e + 2).unwrap_or(n);
                i = end;
            } else {
                i = memchr(b, i + 2, b'\n').unwrap_or(n);
            }
            out.push(Token { kind: Tok::Comment, start, end: i });
            continue;
        }

        // Quoted strings.
        if c == b'"' || c == b'\'' {
            let start = i;
            i += 1;
            let mut terminated = false;
            while i < n {
                let d = b[i];
                if d == b'\\' {
                    i += 2;
                } else if d == b'\n' {
                    break; // unterminated: quotes don't span lines
                } else if d == c {
                    i += 1;
                    terminated = true;
                    break;
                } else {
                    i += char_len(src, i);
                }
            }
            i = i.min(n);
            out.push(Token { kind: Tok::Str { terminated }, start, end: i });
            continue;
        }

        // Long strings [[ ]].
        if c == b'[' && next == b'[' {
            let start = i;
            let terminated = find2(b, i + 2, b']', b']').is_some();
            i = find2(b, i + 2, b']', b']').map(|e| e + 2).unwrap_or(n);
            out.push(Token { kind: Tok::Str { terminated }, start, end: i });
            continue;
        }

        // Numbers.
        if c.is_ascii_digit() || (c == b'.' && next.is_ascii_digit()) {
            let start = i;
            i += 1;
            while i < n {
                let d = b[i];
                let dn = if i + 1 < n { b[i + 1] } else { 0 };
                if d.is_ascii_alphanumeric() || d == b'.' || d == b'_' {
                    i += 1;
                } else if (d == b'e' || d == b'E' || d == b'p' || d == b'P')
                    && (dn == b'+' || dn == b'-')
                {
                    i += 2;
                } else {
                    break;
                }
            }
            out.push(Token { kind: Tok::Number, start, end: i });
            continue;
        }

        // Identifiers / keywords.
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let kind = if KEYWORDS.contains(&&src[start..i]) {
                Tok::Keyword
            } else {
                Tok::Ident
            };
            out.push(Token { kind, start, end: i });
            continue;
        }

        // Anything else: a one-character symbol.
        let start = i;
        i += char_len(src, i);
        out.push(Token { kind: Tok::Symbol, start, end: i });
    }

    out
}

fn char_len(text: &str, i: usize) -> usize {
    text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1)
}

fn memchr(b: &[u8], from: usize, needle: u8) -> Option<usize> {
    (from..b.len()).find(|&i| b[i] == needle)
}

fn find2(b: &[u8], from: usize, a: u8, c: u8) -> Option<usize> {
    (from..b.len().saturating_sub(1)).find(|&i| b[i] == a && b[i + 1] == c)
}

/// Byte offset of the `char_idx`-th char (as egui reports cursor positions in
/// char units). Clamps to `src.len()`.
pub fn byte_of_char(src: &str, char_idx: usize) -> usize {
    src.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(src.len())
}

/// Char offset of a byte position (inverse of [`byte_of_char`]).
pub fn char_of_byte(src: &str, byte_idx: usize) -> usize {
    src[..byte_idx.min(src.len())].chars().count()
}

/// 1-based (line, column) for a byte offset. Column is in chars.
pub fn line_col(src: &str, byte_idx: usize) -> (usize, usize) {
    let upto = &src[..byte_idx.min(src.len())];
    let line = upto.matches('\n').count() + 1;
    let col = upto.rsplit('\n').next().map(|s| s.chars().count()).unwrap_or(0) + 1;
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_every_byte() {
        let src = "local x = 1 -- c\nprint(\"hi\")\n";
        let toks = tokenize(src);
        // Non-whitespace bytes are all covered by some token.
        let mut covered = vec![false; src.len()];
        for t in &toks {
            for b in t.start..t.end {
                covered[b] = true;
            }
        }
        for (i, &ch) in src.as_bytes().iter().enumerate() {
            if !ch.is_ascii_whitespace() {
                assert!(covered[i], "byte {i} ({}) uncovered", ch as char);
            }
        }
    }

    #[test]
    fn flags_unterminated_strings() {
        let toks = tokenize("local s = \"oops\n");
        assert!(toks.iter().any(|t| matches!(t.kind, Tok::Str { terminated: false })));
        let ok = tokenize("local s = \"fine\"");
        assert!(ok.iter().any(|t| matches!(t.kind, Tok::Str { terminated: true })));
    }

    #[test]
    fn line_col_is_one_based() {
        let src = "a\nbc\n";
        assert_eq!(line_col(src, 0), (1, 1));
        assert_eq!(line_col(src, 3), (2, 2));
    }
}
