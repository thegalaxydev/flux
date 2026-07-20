//! Completion suggestions: members after `.`/`:`, and bare identifiers
//! (keywords, globals, and symbols declared in the current file).

use super::api::{ApiDb, Entry};
use super::context::{Ctx, completion_context};
use super::lex::{KEYWORDS, Tok, tokenize};
use super::symbols::SymbolIndex;

/// Resolves scene-tree navigation for hierarchy-aware completion. Implemented by
/// the editor over the live `World`, so this language module stays engine-
/// agnostic.
pub trait SceneResolver {
    /// The `(name, class)` of each child of the instance reached by the base
    /// expression (e.g. `"script.Parent"`, `"game.Workspace"`), or `None` if the
    /// expression doesn't resolve to an instance.
    fn children(&self, base: &str) -> Option<Vec<(String, String)>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Keyword,
    Module,
    Function,
    Method,
    Property,
    Event,
    Variable,
    /// A child instance in the scene tree (e.g. `script.Parent.Module`).
    Instance,
}

impl CompletionKind {
    fn from_entry(kind: &str) -> Self {
        match kind {
            "library" => CompletionKind::Module,
            "function" => CompletionKind::Function,
            "method" => CompletionKind::Method,
            "event" => CompletionKind::Event,
            "variable" => CompletionKind::Variable,
            _ => CompletionKind::Property,
        }
    }

    /// A short glyph shown in the popup gutter.
    pub fn glyph(self) -> &'static str {
        match self {
            CompletionKind::Keyword => "key",
            CompletionKind::Module => "mod",
            CompletionKind::Function => "fn",
            CompletionKind::Method => "fn",
            CompletionKind::Property => "prop",
            CompletionKind::Event => "evt",
            CompletionKind::Variable => "var",
            CompletionKind::Instance => "obj",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub label: String,
    pub insert: String,
    pub kind: CompletionKind,
    pub detail: String,
    pub doc: String,
}

#[derive(Default)]
pub struct CompletionProvider;

impl CompletionProvider {
    pub fn completions(
        &self,
        db: &ApiDb,
        idx: &SymbolIndex,
        src: &str,
        cursor: usize,
        scene: Option<&dyn SceneResolver>,
    ) -> Vec<Completion> {
        // Don't offer suggestions while typing inside a string or comment.
        if in_literal(src, cursor) {
            return Vec::new();
        }
        match completion_context(src, cursor) {
            Ctx::Member { base, prefix, .. } => member_completions(db, idx, &base, &prefix, scene),
            Ctx::Ident { prefix, .. } => ident_completions(db, idx, &prefix),
            Ctx::None => Vec::new(),
        }
    }
}

/// Whether `byte` sits within a string or comment token (where completion is
/// unwanted).
fn in_literal(src: &str, byte: usize) -> bool {
    tokenize(src).iter().any(|t| {
        if !matches!(t.kind, Tok::Str { .. } | Tok::Comment) || byte <= t.start {
            return false;
        }
        // For open literals (comments, unterminated strings) the cursor may sit
        // right at the end and still be "inside".
        let open = matches!(t.kind, Tok::Comment | Tok::Str { terminated: false });
        byte < t.end || (open && byte == t.end)
    })
}

fn matches(name: &str, prefix: &str) -> bool {
    prefix.is_empty() || name.to_lowercase().starts_with(&prefix.to_lowercase())
}

fn entry_completion(name: &str, entry: &Entry) -> Completion {
    Completion {
        label: name.to_string(),
        insert: name.to_string(),
        kind: CompletionKind::from_entry(&entry.kind),
        detail: entry.detail(name),
        doc: entry.doc.clone(),
    }
}

fn sort(list: &mut [Completion], prefix: &str) {
    let pl = prefix.to_lowercase();
    list.sort_by(|a, b| {
        // Case-sensitive prefix matches rank first, then alphabetical.
        let a_exact = a.label.starts_with(prefix);
        let b_exact = b.label.starts_with(prefix);
        b_exact
            .cmp(&a_exact)
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
            .then_with(|| {
                // Tiny tiebreak so identical case-folded names are stable.
                (a.label.to_lowercase() == pl).cmp(&(b.label.to_lowercase() == pl))
            })
    });
}

fn member_completions(
    db: &ApiDb,
    idx: &SymbolIndex,
    base: &str,
    prefix: &str,
    scene: Option<&dyn SceneResolver>,
) -> Vec<Completion> {
    let mut list: Vec<Completion> = Vec::new();

    // Scene children first: navigating the hierarchy through `script`, `game`, or
    // `workspace` (e.g. `script.Parent.Module`) offers the resolved instance's
    // children by name.
    if let Some(children) = scene.and_then(|s| s.children(base)) {
        for (name, class) in children {
            if matches(&name, prefix) {
                list.push(Completion {
                    label: name.clone(),
                    insert: name,
                    kind: CompletionKind::Instance,
                    detail: class,
                    doc: String::new(),
                });
            }
        }
    }

    // API members of the base's resolved type (plus a generic-Instance fallback
    // for a bare local of unknown type, so `sprite.` still suggests Position...).
    let map = db.members_after(base, idx).or_else(|| {
        let simple = !base.contains(['.', ':', '(']) && idx.is_defined(base.trim());
        simple.then(|| db.members_of_type("Instance")).flatten()
    });
    if let Some(map) = map {
        for (name, entry) in map.iter().filter(|(name, _)| matches(name, prefix)) {
            list.push(entry_completion(name, entry));
        }
    }

    sort(&mut list, prefix);
    list.dedup_by(|a, b| a.label == b.label);
    list
}

fn ident_completions(db: &ApiDb, idx: &SymbolIndex, prefix: &str) -> Vec<Completion> {
    let mut list: Vec<Completion> = Vec::new();

    // Symbols declared in this file.
    let mut seen: Vec<&str> = Vec::new();
    for sym in &idx.symbols {
        if !matches(&sym.name, prefix) || seen.contains(&sym.name.as_str()) {
            continue;
        }
        seen.push(&sym.name);
        let kind = match sym.kind {
            super::symbols::SymKind::Function => CompletionKind::Function,
            _ => CompletionKind::Variable,
        };
        list.push(Completion {
            label: sym.name.clone(),
            insert: sym.name.clone(),
            kind,
            detail: sym.kind.label().to_string(),
            doc: String::new(),
        });
    }

    // Engine globals.
    for (name, entry) in &db.globals {
        if matches(name, prefix) {
            list.push(entry_completion(name, entry));
        }
    }

    // Keywords.
    for kw in KEYWORDS {
        if matches(kw, prefix) {
            list.push(Completion {
                label: kw.to_string(),
                insert: kw.to_string(),
                kind: CompletionKind::Keyword,
                detail: "keyword".to_string(),
                doc: String::new(),
            });
        }
    }

    sort(&mut list, prefix);
    list.dedup_by(|a, b| a.label == b.label);
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(list: &[Completion]) -> Vec<&str> {
        list.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn suggests_members_after_dot() {
        let db = ApiDb::load();
        let idx = SymbolIndex::default();
        let src = "Vec2.";
        let list = CompletionProvider.completions(&db, &idx, src, src.len(), None);
        assert!(labels(&list).contains(&"new"));
    }

    #[test]
    fn suggests_service_members_from_local() {
        let db = ApiDb::load();
        let src = "local Input = game:GetService(\"Input\")\nInput.";
        let idx = SymbolIndex::build(src);
        let list = CompletionProvider.completions(&db, &idx, src, src.len(), None);
        assert!(labels(&list).contains(&"IsKeyDown"));
    }

    #[test]
    fn suggests_locals_and_globals_for_prefix() {
        let db = ApiDb::load();
        let src = "local speed = 5\nsp";
        let idx = SymbolIndex::build(src);
        let list = CompletionProvider.completions(&db, &idx, src, src.len(), None);
        assert!(labels(&list).contains(&"speed"));
    }

    #[test]
    fn no_completions_inside_strings() {
        let db = ApiDb::load();
        let idx = SymbolIndex::default();
        let src = "local s = \"Vec2.ne";
        let list = CompletionProvider.completions(&db, &idx, src, src.len(), None);
        assert!(list.is_empty(), "should not complete inside a string: {list:?}");
    }

    #[test]
    fn falls_back_to_instance_members_for_unknown_local() {
        let db = ApiDb::load();
        let src = "local sprite = workspace.Thing\nsprite.";
        let idx = SymbolIndex::build(src);
        let list = CompletionProvider.completions(&db, &idx, src, src.len(), None);
        assert!(labels(&list).contains(&"Position"));
    }

    /// A stub resolver: `script.Parent` has a "Module" and a "Health" child.
    struct Stub;
    impl SceneResolver for Stub {
        fn children(&self, base: &str) -> Option<Vec<(String, String)>> {
            (base == "script.Parent").then(|| {
                vec![
                    ("Module".to_string(), "Module".to_string()),
                    ("Health".to_string(), "Script".to_string()),
                ]
            })
        }
    }

    #[test]
    fn suggests_scene_children_through_hierarchy() {
        let db = ApiDb::load();
        let src = "print(script.Parent.)";
        let cursor = src.find('.').unwrap() + ".Parent.".len();
        let idx = SymbolIndex::build(src);
        let list = CompletionProvider.completions(&db, &idx, src, cursor, Some(&Stub));
        // Children of the resolved instance are offered by name...
        assert!(labels(&list).contains(&"Module"));
        assert!(labels(&list).contains(&"Health"));
        // ...alongside Instance members from the API.
        assert!(labels(&list).contains(&"Parent"));

        // With a prefix, they filter.
        let src2 = "print(script.Parent.Mo)";
        let cursor2 = src2.rfind("Mo").unwrap() + 2;
        let idx2 = SymbolIndex::build(src2);
        let list2 = CompletionProvider.completions(&db, &idx2, src2, cursor2, Some(&Stub));
        assert!(labels(&list2).contains(&"Module"));
        assert!(!labels(&list2).contains(&"Health"));
    }
}
