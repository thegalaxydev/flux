//! Script language service for the Flux editor.
//!
//! A self-contained "IDE brain" for Luau. Two cooperating halves:
//!
//! * **Interactive** (completion, hover, signature help) — fast token-based
//!   resolution over [`symbols`]/[`context`]/[`api`].
//! * **Diagnostics** — a real front-end pipeline: [`token`] → [`parser`] →
//!   [`ast`] → [`semantic`] (scope / types / builtin / flow), all writing into a
//!   shared [`diagnostics::Diagnostics`] collector.
//!
//! The editor UI (`script_editor.rs`) only talks to [`ScriptLanguageService`];
//! every parsing detail lives behind it, so it can grow into a full language
//! server (go-to-definition, rename, find-references, code actions) without the
//! UI changing.

mod api;
mod ast;
mod builtin;
mod completion;
mod context;
mod diagnostics;
mod flow;
mod hover;
mod lex;
mod parser;
mod scope;
mod semantic;
mod signature;
mod suggestions;
mod symbols;
mod token;
mod types;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub use completion::{Completion, CompletionKind, CompletionProvider};
pub use diagnostics::{Diagnostic, DiagnosticSeverity};
pub use hover::{Hover, HoverProvider};
pub use lex::{byte_of_char, char_of_byte, line_col};
pub use signature::{SignatureHelp, SignatureHelpProvider};

use builtin::Builtins;
use diagnostics::Diagnostics;
use symbols::SymbolIndex;

/// Per-buffer analysis, rebuilt only when the text changes (incremental at the
/// document level: no work and no reallocation while the buffer is unchanged).
struct Analysis {
    hash: u64,
    index: SymbolIndex,
    diagnostics: Vec<Diagnostic>,
}

pub struct ScriptLanguageService {
    builtins: Builtins,
    completion: CompletionProvider,
    hover: HoverProvider,
    signature: SignatureHelpProvider,
    analysis: Option<Analysis>,
}

impl Default for ScriptLanguageService {
    fn default() -> Self {
        Self {
            builtins: Builtins::load(),
            completion: CompletionProvider,
            hover: HoverProvider,
            signature: SignatureHelpProvider,
            analysis: None,
        }
    }
}

fn hash(src: &str) -> u64 {
    let mut h = DefaultHasher::new();
    src.hash(&mut h);
    h.finish()
}

/// Run the full diagnostics pipeline over `src`.
fn run_diagnostics(builtins: &Builtins, src: &str) -> Vec<Diagnostic> {
    let mut diags = Diagnostics::new(src);
    let block = parser::parse(src, &mut diags);
    semantic::analyze(src, &block, builtins, &mut diags);
    diags.finish()
}

impl ScriptLanguageService {
    /// Rebuild the cached symbol index + diagnostics if `src` changed. Kept as a
    /// `()`-returning method so callers can then borrow disjoint fields.
    fn analyze(&mut self, src: &str) {
        let h = hash(src);
        if self.analysis.as_ref().map(|a| a.hash) != Some(h) {
            let index = SymbolIndex::build(src);
            let diagnostics = run_diagnostics(&self.builtins, src);
            self.analysis = Some(Analysis { hash: h, index, diagnostics });
        }
    }

    /// Completion suggestions for the cursor at char index `char_cursor`.
    pub fn completions(&mut self, src: &str, char_cursor: usize) -> Vec<Completion> {
        let cursor = byte_of_char(src, char_cursor);
        self.analyze(src);
        let a = self.analysis.as_ref().unwrap();
        self.completion.completions(self.builtins.db(), &a.index, src, cursor)
    }

    /// Hover info for the char index `char_pos` (e.g. under the pointer).
    pub fn hover(&mut self, src: &str, char_pos: usize) -> Option<Hover> {
        let byte = byte_of_char(src, char_pos);
        self.analyze(src);
        let a = self.analysis.as_ref().unwrap();
        self.hover.hover(self.builtins.db(), &a.index, src, byte)
    }

    /// Signature help for the cursor at char index `char_cursor`.
    pub fn signature(&mut self, src: &str, char_cursor: usize) -> Option<SignatureHelp> {
        let cursor = byte_of_char(src, char_cursor);
        self.analyze(src);
        let a = self.analysis.as_ref().unwrap();
        self.signature.signature(self.builtins.db(), &a.index, src, cursor)
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

        let byte_input = src.rfind("Input").unwrap();
        assert!(svc.hover(src, byte_input + 1).is_some());

        // Diagnostics are cached and stable across identical calls.
        let n = svc.diagnostics(src).len();
        assert_eq!(n, svc.diagnostics(src).len());
    }

    #[test]
    fn diagnostics_flow_through_service() {
        let mut svc = ScriptLanguageService::default();
        let d = svc.diagnostics("local x = 5\nx()\n").to_vec();
        assert!(d.iter().any(|x| x.message.contains("attempt to call a number")));
    }
}
