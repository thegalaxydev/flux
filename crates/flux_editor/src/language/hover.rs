//! Hover tooltips: describe the symbol under the cursor (globals, members, and
//! file-local declarations).

use super::api::ApiDb;
use super::context::word_at;
use super::symbols::SymbolIndex;

#[derive(Debug, Clone, PartialEq)]
pub struct Hover {
    /// A one-line signature/type, shown emphasised.
    pub title: String,
    /// Longer human description (may be empty).
    pub doc: String,
}

#[derive(Default)]
pub struct HoverProvider;

impl HoverProvider {
    pub fn hover(&self, db: &ApiDb, idx: &SymbolIndex, src: &str, byte: usize) -> Option<Hover> {
        let target = word_at(src, byte)?;

        if target.is_member {
            // `base.word` — resolve the member against base's type, then fall
            // back to generic Instance members (so `sprite.Position` still docs).
            let entry = db
                .members_after(&target.base, idx)
                .and_then(|m| m.get(&target.word))
                .or_else(|| {
                    db.members_of_type("Instance").and_then(|m| m.get(&target.word))
                })?;
            return Some(Hover {
                title: entry.detail(&target.word),
                doc: entry.doc.clone(),
            });
        }

        // A bare word: engine global first, then a file-local symbol.
        if let Some(entry) = db.global(&target.word) {
            return Some(Hover {
                title: entry.detail(&target.word),
                doc: entry.doc.clone(),
            });
        }
        if let Some(sym) = idx.symbols.iter().find(|s| s.name == target.word) {
            let ty = sym
                .type_hint
                .as_deref()
                .map(|t| t.strip_prefix("service:").unwrap_or(t))
                .map(|t| format!(": {t}"))
                .unwrap_or_default();
            return Some(Hover {
                title: format!("{} {}{}", sym.kind.label(), sym.name, ty),
                doc: format!("Declared on line {}.", sym.line),
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hovers_library_member() {
        let db = ApiDb::load();
        let idx = SymbolIndex::default();
        let src = "Vec2.new(1, 2)";
        // byte index inside "new"
        let h = HoverProvider.hover(&db, &idx, src, 6).unwrap();
        assert!(h.title.contains("new"));
        assert!(h.doc.contains("2D vector"));
    }

    #[test]
    fn hovers_instance_member_fallback() {
        let db = ApiDb::load();
        let src = "local sprite = workspace.Thing\nsprite.Position = nil";
        let idx = SymbolIndex::build(src);
        let byte = src.find("Position").unwrap() + 2;
        let h = HoverProvider.hover(&db, &idx, src, byte).unwrap();
        assert!(h.doc.contains("2D position"));
    }

    #[test]
    fn hovers_local_symbol() {
        let db = ApiDb::load();
        let src = "local speed = 5\nprint(speed)";
        let idx = SymbolIndex::build(src);
        let byte = src.rfind("speed").unwrap() + 1;
        let h = HoverProvider.hover(&db, &idx, src, byte).unwrap();
        assert!(h.title.contains("speed"));
    }
}
