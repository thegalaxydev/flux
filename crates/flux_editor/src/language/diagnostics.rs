//! Diagnostic model shared by every stage of the language pipeline (lexer,
//! parser, and each semantic pass). Any stage pushes into a [`Diagnostics`]
//! collector; the editor renders whatever comes out.

use std::ops::Range;

/// A byte span into the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    /// The smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn range(self) -> Range<usize> {
        self.start..self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// A 1-based (line, column) plus the byte offset, so the editor can map to
/// galley positions and humans can read the location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextPosition {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub start: TextPosition,
    pub end: TextPosition,
}

impl Diagnostic {
    pub fn range(&self) -> Range<usize> {
        self.start.offset..self.end.offset
    }
}

/// Precomputed line-start offsets for O(log n) offset → (line, column) lookup,
/// so converting many diagnostics doesn't rescan the whole source each time.
pub struct LineIndex {
    /// Byte offset of the start of each line.
    starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub fn new(src: &str) -> Self {
        let mut starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        LineIndex { starts, len: src.len() }
    }

    pub fn position(&self, offset: usize) -> TextPosition {
        let offset = offset.min(self.len);
        // Largest line-start <= offset.
        let line = self.starts.partition_point(|&s| s <= offset).max(1);
        let column = offset - self.starts[line - 1] + 1;
        TextPosition { offset, line, column }
    }
}

/// Collector every stage writes into. Owns the line index so callers push raw
/// byte spans and get resolved positions, with no lifetime plumbing.
pub struct Diagnostics {
    lines: LineIndex,
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new(src: &str) -> Self {
        Diagnostics { lines: LineIndex::new(src), items: Vec::new() }
    }

    pub fn push(&mut self, severity: DiagnosticSeverity, span: Span, message: impl Into<String>) {
        self.items.push(Diagnostic {
            severity,
            message: message.into(),
            start: self.lines.position(span.start),
            end: self.lines.position(span.end.max(span.start)),
        });
    }

    pub fn error(&mut self, span: Span, message: impl Into<String>) {
        self.push(DiagnosticSeverity::Error, span, message);
    }

    pub fn warning(&mut self, span: Span, message: impl Into<String>) {
        self.push(DiagnosticSeverity::Warning, span, message);
    }

    pub fn info(&mut self, span: Span, message: impl Into<String>) {
        self.push(DiagnosticSeverity::Information, span, message);
    }

    pub fn hint(&mut self, span: Span, message: impl Into<String>) {
        self.push(DiagnosticSeverity::Hint, span, message);
    }

    pub fn finish(mut self) -> Vec<Diagnostic> {
        self.items.sort_by(|a, b| {
            a.start.offset.cmp(&b.start.offset).then(a.end.offset.cmp(&b.end.offset))
        });
        self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_maps_positions() {
        let src = "abc\nde\nf";
        let idx = LineIndex::new(src);
        assert_eq!(idx.position(0), TextPosition { offset: 0, line: 1, column: 1 });
        assert_eq!(idx.position(4), TextPosition { offset: 4, line: 2, column: 1 });
        assert_eq!(idx.position(7), TextPosition { offset: 7, line: 3, column: 1 });
    }

    #[test]
    fn diagnostics_sorted_by_position() {
        let mut d = Diagnostics::new("hello world");
        d.error(Span::new(6, 11), "second");
        d.warning(Span::new(0, 5), "first");
        let out = d.finish();
        assert_eq!(out[0].message, "first");
        assert_eq!(out[1].message, "second");
    }
}
