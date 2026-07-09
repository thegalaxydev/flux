//! Script language service for the Flux editor.
//!
//! A small, self-contained "IDE brain" for Luau: completion, hover, signature
//! help, and diagnostics, backed by embedded API metadata and a token-based
//! symbol index. The editor UI (`script_editor.rs`) only talks to
//! [`ScriptLanguageService`]; all parsing lives behind it so it can later be
//! replaced by a real Luau parser / LSP without touching the UI.

mod api;
mod completion;
mod context;
mod diagnostics;
mod hover;
mod lex;
mod signature;
mod symbols;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub use completion::{Completion, CompletionKind, CompletionProvider};
pub use diagnostics::{Diagnostic, DiagnosticsProvider, Severity};
pub use hover::{Hover, HoverProvider};
pub use lex::{byte_of_char, char_of_byte, line_col};
pub use signature::{SignatureHelp, SignatureHelpProvider};

use api::ApiDb;
use symbols::SymbolIndex;

/// Per-buffer analysis, rebuilt only when the text changes.
struct Analysis {
    hash: u64,
    index: SymbolIndex,
    diagnostics: Vec<Diagnostic>,
}

pub struct ScriptLanguageService {
    db: ApiDb,
    completion: CompletionProvider,
    hover: HoverProvider,
    signature: SignatureHelpProvider,
    diagnostics: DiagnosticsProvider,
    analysis: Option<Analysis>,
}

impl Default for ScriptLanguageService {
    fn default() -> Self {
        Self {
            db: ApiDb::load(),
            completion: CompletionProvider,
            hover: HoverProvider,
            signature: SignatureHelpProvider,
            diagnostics: DiagnosticsProvider,
            analysis: None,
        }
    }
}

fn hash(src: &str) -> u64 {
    let mut h = DefaultHasher::new();
    src.hash(&mut h);
    h.finish()
}

impl ScriptLanguageService {
    /// Rebuild the cached symbol index + diagnostics if `src` changed. Kept as
    /// a `()`-returning method so callers can then borrow `self.db` and the
    /// cached index as disjoint fields.
    fn analyze(&mut self, src: &str) {
        let h = hash(src);
        if self.analysis.as_ref().map(|a| a.hash) != Some(h) {
            let index = SymbolIndex::build(src);
            let diagnostics = self.diagnostics.diagnostics(&self.db, &index, src);
            self.analysis = Some(Analysis { hash: h, index, diagnostics });
        }
    }

    /// Completion suggestions for the cursor at char index `char_cursor`.
    pub fn completions(&mut self, src: &str, char_cursor: usize) -> Vec<Completion> {
        let cursor = byte_of_char(src, char_cursor);
        self.analyze(src);
        let a = self.analysis.as_ref().unwrap();
        self.completion.completions(&self.db, &a.index, src, cursor)
    }

    /// Hover info for the char index `char_pos` (e.g. under the pointer).
    pub fn hover(&mut self, src: &str, char_pos: usize) -> Option<Hover> {
        let byte = byte_of_char(src, char_pos);
        self.analyze(src);
        let a = self.analysis.as_ref().unwrap();
        self.hover.hover(&self.db, &a.index, src, byte)
    }

    /// Signature help for the cursor at char index `char_cursor`.
    pub fn signature(&mut self, src: &str, char_cursor: usize) -> Option<SignatureHelp> {
        let cursor = byte_of_char(src, char_cursor);
        self.analyze(src);
        let a = self.analysis.as_ref().unwrap();
        self.signature.signature(&self.db, &a.index, src, cursor)
    }

    /// Diagnostics for `src` (cached until the text changes).
    pub fn diagnostics(&mut self, src: &str) -> &[Diagnostic] {
        self.analyze(src);
        &self.analysis.as_ref().unwrap().diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_end_to_end() {
        let mut svc = ScriptLanguageService::default();
        let src = "local Input = game:GetService(\"Input\")\nInput.";
        let comps = svc.completions(src, src.chars().count());
        assert!(comps.iter().any(|c| c.label == "IsKeyDown"));

        // Hover on the "Input" service variable resolves.
        let byte_input = src.rfind("Input").unwrap();
        let h = svc.hover(src, byte_input + 1);
        assert!(h.is_some());

        // Diagnostics are cached and stable.
        let n = svc.diagnostics(src).len();
        assert_eq!(n, svc.diagnostics(src).len());
    }
}
