//! Full Luau lexer for the parser: multi-character operators, keyword
//! classification, number-literal validation, and string-termination tracking.
//! Emits lexical diagnostics (unterminated string, invalid number) into the
//! shared [`Diagnostics`] collector.
//!
//! Kept separate from [`super::lex`] (which powers completion/hover) so the two
//! consumers can evolve independently; this one is parser-shaped.

use super::diagnostics::{Diagnostics, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tk {
    // Literals / identifiers.
    Name,
    Number,
    Str,
    // Keywords.
    And,
    Break,
    Do,
    Else,
    Elseif,
    End,
    False,
    For,
    Function,
    If,
    In,
    Local,
    Nil,
    Not,
    Or,
    Repeat,
    Return,
    Then,
    True,
    Until,
    While,
    // Symbols / operators.
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Hash,
    Eq,
    Ne,
    Le,
    Ge,
    Lt,
    Gt,
    Assign,
    // Luau compound assignment.
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    CaretEq,
    ConcatEq,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semicolon,
    Colon,
    DoubleColon,
    Comma,
    Dot,
    Concat,
    Ellipsis,
    Arrow,
    Question,
    Pipe,
    Amp,
    Eof,
    /// Any byte we don't recognise (keeps the stream total).
    Unknown,
}

impl Tk {
    /// Reserved keywords (contextual keywords like `type`/`continue`/`export`
    /// are lexed as [`Tk::Name`] and handled by the parser).
    fn keyword(word: &str) -> Option<Tk> {
        Some(match word {
            "and" => Tk::And,
            "break" => Tk::Break,
            "do" => Tk::Do,
            "else" => Tk::Else,
            "elseif" => Tk::Elseif,
            "end" => Tk::End,
            "false" => Tk::False,
            "for" => Tk::For,
            "function" => Tk::Function,
            "if" => Tk::If,
            "in" => Tk::In,
            "local" => Tk::Local,
            "nil" => Tk::Nil,
            "not" => Tk::Not,
            "or" => Tk::Or,
            "repeat" => Tk::Repeat,
            "return" => Tk::Return,
            "then" => Tk::Then,
            "true" => Tk::True,
            "until" => Tk::Until,
            "while" => Tk::While,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Token {
    pub kind: Tk,
    pub span: Span,
}

/// Tokenize `src`, reporting lexical errors into `diags`. Always ends with an
/// [`Tk::Eof`] token so the parser can rely on a sentinel.
pub fn lex(src: &str, diags: &mut Diagnostics) -> Vec<Token> {
    let b = src.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut i = 0;

    while i < n {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Comments.
        if c == b'-' && i + 1 < n && b[i + 1] == b'-' {
            if let Some((_, end)) = long_bracket(b, i + 2) {
                i = end;
            } else {
                i = memchr(b, i + 2, b'\n').unwrap_or(n);
            }
            continue;
        }

        let start = i;
        let kind = match c {
            b'"' | b'\'' => {
                i = scan_quoted(src, b, i, diags);
                Tk::Str
            }
            b'[' if matches!(b.get(i + 1), Some(b'[') | Some(b'=')) && long_bracket(b, i).is_some() => {
                let (_, end) = long_bracket(b, i).unwrap();
                if end == n && !long_bracket_closed(b, i) {
                    diags.error(Span::new(start, n), "unterminated long string");
                }
                i = end;
                Tk::Str
            }
            b'0'..=b'9' => {
                i = scan_number(src, b, i, diags);
                Tk::Number
            }
            b'.' if b.get(i + 1).is_some_and(|d| d.is_ascii_digit()) => {
                i = scan_number(src, b, i, diags);
                Tk::Number
            }
            _ if c.is_ascii_alphabetic() || c == b'_' => {
                i += 1;
                while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                Tk::keyword(&src[start..i]).unwrap_or(Tk::Name)
            }
            _ => {
                let (k, len) = symbol(b, i);
                i += len;
                k
            }
        };
        out.push(Token { kind, span: Span::new(start, i) });
    }

    out.push(Token { kind: Tk::Eof, span: Span::new(n, n) });
    out
}

/// Match a symbol/operator at `i`; returns its kind and byte length.
fn symbol(b: &[u8], i: usize) -> (Tk, usize) {
    let c = b[i];
    let d = b.get(i + 1).copied().unwrap_or(0);
    let e = b.get(i + 2).copied().unwrap_or(0);
    match (c, d, e) {
        (b'.', b'.', b'.') => (Tk::Ellipsis, 3),
        (b'.', b'.', b'=') => (Tk::ConcatEq, 3),
        (b'.', b'.', _) => (Tk::Concat, 2),
        (b':', b':', _) => (Tk::DoubleColon, 2),
        (b'=', b'=', _) => (Tk::Eq, 2),
        (b'~', b'=', _) => (Tk::Ne, 2),
        (b'<', b'=', _) => (Tk::Le, 2),
        (b'>', b'=', _) => (Tk::Ge, 2),
        (b'-', b'>', _) => (Tk::Arrow, 2),
        (b'+', b'=', _) => (Tk::PlusEq, 2),
        (b'-', b'=', _) => (Tk::MinusEq, 2),
        (b'*', b'=', _) => (Tk::StarEq, 2),
        (b'/', b'=', _) => (Tk::SlashEq, 2),
        (b'%', b'=', _) => (Tk::PercentEq, 2),
        (b'^', b'=', _) => (Tk::CaretEq, 2),
        (b'+', _, _) => (Tk::Plus, 1),
        (b'-', _, _) => (Tk::Minus, 1),
        (b'*', _, _) => (Tk::Star, 1),
        (b'/', _, _) => (Tk::Slash, 1),
        (b'%', _, _) => (Tk::Percent, 1),
        (b'^', _, _) => (Tk::Caret, 1),
        (b'#', _, _) => (Tk::Hash, 1),
        (b'<', _, _) => (Tk::Lt, 1),
        (b'>', _, _) => (Tk::Gt, 1),
        (b'=', _, _) => (Tk::Assign, 1),
        (b'(', _, _) => (Tk::LParen, 1),
        (b')', _, _) => (Tk::RParen, 1),
        (b'{', _, _) => (Tk::LBrace, 1),
        (b'}', _, _) => (Tk::RBrace, 1),
        (b'[', _, _) => (Tk::LBracket, 1),
        (b']', _, _) => (Tk::RBracket, 1),
        (b';', _, _) => (Tk::Semicolon, 1),
        (b':', _, _) => (Tk::Colon, 1),
        (b',', _, _) => (Tk::Comma, 1),
        (b'.', _, _) => (Tk::Dot, 1),
        (b'?', _, _) => (Tk::Question, 1),
        (b'|', _, _) => (Tk::Pipe, 1),
        (b'&', _, _) => (Tk::Amp, 1),
        // Consume one char (or one UTF-8 codepoint) as Unknown.
        _ => (Tk::Unknown, utf8_len(b, i)),
    }
}

fn scan_quoted(src: &str, b: &[u8], start: usize, diags: &mut Diagnostics) -> usize {
    let n = b.len();
    let quote = b[start];
    let mut i = start + 1;
    while i < n {
        match b[i] {
            b'\\' => i += 2,
            b'\n' => {
                diags.error(Span::new(start, i), "unterminated string");
                return i;
            }
            c if c == quote => return i + 1,
            _ => i += utf8_len(b, i),
        }
    }
    diags.error(Span::new(start, n), "unterminated string");
    let _ = src;
    n
}

fn scan_number(src: &str, b: &[u8], start: usize, diags: &mut Diagnostics) -> usize {
    let n = b.len();
    let mut i = start;
    if b[i] == b'0' && matches!(b.get(i + 1), Some(b'x') | Some(b'X')) {
        i += 2;
        while i < n && (b[i].is_ascii_hexdigit() || b[i] == b'_') {
            i += 1;
        }
    } else {
        while i < n && (b[i].is_ascii_digit() || b[i] == b'_' || b[i] == b'.') {
            i += 1;
        }
        if i < n && (b[i] == b'e' || b[i] == b'E') {
            i += 1;
            if i < n && (b[i] == b'+' || b[i] == b'-') {
                i += 1;
            }
            while i < n && (b[i].is_ascii_digit() || b[i] == b'_') {
                i += 1;
            }
        }
    }
    // A letter/second dot stuck to the number is malformed (e.g. `1.2.3`, `5x`).
    let trailing = i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.');
    let text = &src[start..i];
    if trailing || !valid_number(text) {
        let mut end = i;
        while end < n && (b[end].is_ascii_alphanumeric() || b[end] == b'_' || b[end] == b'.') {
            end += 1;
        }
        diags.error(Span::new(start, end), "invalid number literal");
        return end;
    }
    i
}

fn valid_number(text: &str) -> bool {
    let t = text.replace('_', "");
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return !hex.is_empty() && hex.bytes().all(|c| c.is_ascii_hexdigit());
    }
    // At most one dot, at most one exponent, digits present.
    if t.matches('.').count() > 1 {
        return false;
    }
    let mantissa_exp: Vec<&str> = t.splitn(2, ['e', 'E']).collect();
    let mantissa = mantissa_exp[0];
    if mantissa.is_empty() || mantissa == "." {
        return false;
    }
    if !mantissa.bytes().all(|c| c.is_ascii_digit() || c == b'.') {
        return false;
    }
    if let Some(exp) = mantissa_exp.get(1) {
        let exp = exp.strip_prefix(['+', '-']).unwrap_or(exp);
        if exp.is_empty() || !exp.bytes().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    true
}

/// If `b[i]` begins a long bracket `[[`, `[=[`, ... return its level and the
/// offset just past the closing bracket (or `b.len()` if unterminated).
fn long_bracket(b: &[u8], i: usize) -> Option<(usize, usize)> {
    if i >= b.len() || b[i] != b'[' {
        return None;
    }
    let mut j = i + 1;
    let mut level = 0;
    while j < b.len() && b[j] == b'=' {
        level += 1;
        j += 1;
    }
    if j >= b.len() || b[j] != b'[' {
        return None;
    }
    j += 1;
    // Find the matching `]==]`.
    let mut k = j;
    while k < b.len() {
        if b[k] == b']' {
            let mut m = k + 1;
            let mut eq = 0;
            while m < b.len() && b[m] == b'=' {
                eq += 1;
                m += 1;
            }
            if eq == level && m < b.len() && b[m] == b']' {
                return Some((level, m + 1));
            }
        }
        k += 1;
    }
    Some((level, b.len()))
}

fn long_bracket_closed(b: &[u8], i: usize) -> bool {
    match long_bracket(b, i) {
        Some((_, end)) => end < b.len() || ends_with_close(b, i),
        None => false,
    }
}

fn ends_with_close(b: &[u8], i: usize) -> bool {
    // Cheap re-check that the reported end actually landed on a `]`.
    long_bracket(b, i).map(|(_, e)| e).is_some_and(|e| e >= 1 && b.get(e - 1) == Some(&b']'))
}

fn utf8_len(b: &[u8], i: usize) -> usize {
    match b[i] {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
}

fn memchr(b: &[u8], from: usize, needle: u8) -> Option<usize> {
    (from..b.len()).find(|&i| b[i] == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tk> {
        let mut d = Diagnostics::new(src);
        lex(src, &mut d).iter().map(|t| t.kind).collect()
    }

    #[test]
    fn multi_char_operators() {
        let ks = kinds("a == b ~= c .. d ... -> ::");
        assert!(ks.contains(&Tk::Eq));
        assert!(ks.contains(&Tk::Ne));
        assert!(ks.contains(&Tk::Concat));
        assert!(ks.contains(&Tk::Ellipsis));
        assert!(ks.contains(&Tk::Arrow));
        assert!(ks.contains(&Tk::DoubleColon));
    }

    #[test]
    fn keywords_vs_names() {
        let ks = kinds("local x function");
        assert_eq!(ks[0], Tk::Local);
        assert_eq!(ks[1], Tk::Name);
        assert_eq!(ks[2], Tk::Function);
        assert_eq!(*ks.last().unwrap(), Tk::Eof);
    }

    #[test]
    fn unterminated_string_reports() {
        let mut d = Diagnostics::new("x = \"oops\n");
        lex("x = \"oops\n", &mut d);
        assert!(d.finish().iter().any(|x| x.message.contains("unterminated string")));
    }

    #[test]
    fn invalid_number_reports() {
        let mut d = Diagnostics::new("local n = 1.2.3");
        lex("local n = 1.2.3", &mut d);
        assert!(d.finish().iter().any(|x| x.message.contains("invalid number")));
    }

    #[test]
    fn valid_numbers_ok() {
        for s in ["1", "1.5", "0xFF", "1e10", "1.5e-3", "100_000"] {
            let mut d = Diagnostics::new(s);
            lex(s, &mut d);
            assert!(d.finish().is_empty(), "{s} should be valid");
        }
    }
}
