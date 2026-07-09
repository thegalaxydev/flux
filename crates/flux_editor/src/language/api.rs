//! Built-in API metadata (loaded from `assets/api/flux_luau_api.json`) plus the
//! type-resolution used to answer "what are the members of `expr`?" and
//! "what does `expr` resolve to?".
//!
//! This is the only module that knows the shape of the API JSON. Providers ask
//! it for entries and member lists; swapping in a richer schema (or a real Luau
//! type engine) later means changing only this file.

use indexmap::IndexMap;
use serde::Deserialize;

use super::symbols::SymbolIndex;

/// Raw JSON, embedded so the editor always has API docs regardless of the open
/// project.
const API_JSON: &str = include_str!("../../../../assets/api/flux_luau_api.json");

#[derive(Debug, Default, Deserialize)]
pub struct ApiDb {
    #[serde(default)]
    pub globals: IndexMap<String, Entry>,
    #[serde(default)]
    pub types: IndexMap<String, TypeDef>,
    /// Service name (as passed to `GetService`) -> API type name.
    #[serde(default)]
    pub services: IndexMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct TypeDef {
    /// Description of the type (reserved for future "hover on a type name").
    #[serde(default)]
    #[allow(dead_code)]
    pub doc: String,
    #[serde(default)]
    pub members: IndexMap<String, Entry>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct Entry {
    /// One of: library | variable | function | method | property | event.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub doc: String,
    /// Value type (for property/variable/event).
    #[serde(default, rename = "type")]
    pub ty: Option<String>,
    #[serde(default)]
    pub params: Vec<Param>,
    /// Result type (for function/method).
    #[serde(default)]
    pub returns: Option<String>,
    /// Inline members (for a library global such as `Vec2` or `math`).
    #[serde(default)]
    pub members: Option<IndexMap<String, Entry>>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct Param {
    pub name: String,
    #[serde(default, rename = "type")]
    pub ty: Option<String>,
}

impl Param {
    pub fn is_variadic(&self) -> bool {
        self.name == "..." || self.ty.as_deref().is_some_and(|t| t.contains("..."))
    }

    pub fn is_optional(&self) -> bool {
        self.ty.as_deref().is_some_and(|t| t.trim_end().ends_with('?'))
    }
}

impl Entry {
    pub fn is_callable(&self) -> bool {
        matches!(self.kind.as_str(), "function" | "method")
    }

    /// A one-line signature/type description for popups, e.g.
    /// `new(x, y) -> Vec2` or `Position: Vec2`.
    pub fn detail(&self, name: &str) -> String {
        if self.is_callable() {
            let params: Vec<String> = self
                .params
                .iter()
                .map(|p| match &p.ty {
                    Some(t) => format!("{}: {}", p.name, t),
                    None => p.name.clone(),
                })
                .collect();
            let ret = self.returns.as_deref().filter(|r| *r != "()");
            match ret {
                Some(r) => format!("{name}({}) -> {r}", params.join(", ")),
                None => format!("{name}({})", params.join(", ")),
            }
        } else if let Some(t) = &self.ty {
            format!("{name}: {t}")
        } else {
            name.to_string()
        }
    }
}

impl ApiDb {
    pub fn load() -> Self {
        serde_json::from_str(API_JSON).unwrap_or_else(|e| {
            eprintln!("flux: failed to parse embedded Luau API metadata: {e}");
            ApiDb::default()
        })
    }

    pub fn global(&self, name: &str) -> Option<&Entry> {
        self.globals.get(name)
    }

    pub fn members_of_type(&self, name: &str) -> Option<&IndexMap<String, Entry>> {
        self.types.get(name).map(|t| &t.members)
    }

    fn resolve_hint(&self, hint: &str) -> String {
        if let Some(service) = hint.strip_prefix("service:") {
            self.services
                .get(service)
                .cloned()
                .unwrap_or_else(|| "Instance".to_string())
        } else {
            hint.to_string()
        }
    }

    /// Members available after `base` — the expression left of a `.`/`:`.
    /// Examples: `"Enum.KeyCode"`, `"game:GetService(\"Input\")"`, a local var.
    pub fn members_after(
        &self,
        base: &str,
        idx: &SymbolIndex,
    ) -> Option<&IndexMap<String, Entry>> {
        let segs = split_segments(base)?;
        self.walk(&segs, idx)
    }

    fn walk(&self, segs: &[Seg], idx: &SymbolIndex) -> Option<&IndexMap<String, Entry>> {
        let head = segs.first()?;
        let mut current: &IndexMap<String, Entry> = if let Some(hint) = idx.var_type(&head.name) {
            let ty = self.resolve_hint(&hint);
            self.members_of_type(&ty)?
        } else if let Some(g) = self.globals.get(&head.name) {
            if let Some(m) = &g.members {
                m
            } else if let Some(t) = &g.ty {
                self.members_of_type(t)?
            } else {
                return None;
            }
        } else {
            return None;
        };

        for seg in &segs[1..] {
            let entry = current.get(&seg.name)?;
            match self.result_type(seg, entry) {
                Some(t) => current = self.members_of_type(&t)?,
                None => current = entry.members.as_ref()?,
            }
        }
        Some(current)
    }

    fn result_type(&self, seg: &Seg, entry: &Entry) -> Option<String> {
        if seg.name == "GetService" {
            if let Some(t) = seg.arg.as_ref().and_then(|a| self.services.get(a)) {
                return Some(t.clone());
            }
        }
        match entry.kind.as_str() {
            "property" | "variable" | "event" => entry.ty.clone(),
            _ => entry.returns.clone(),
        }
    }

    /// Resolve a full member expression (e.g. `"Vec2.new"`, `"store:GetAsync"`,
    /// or a bare global `"print"`) to its [`Entry`].
    pub fn resolve_member(&self, expr: &str, idx: &SymbolIndex) -> Option<&Entry> {
        let expr = expr.trim();
        match expr.rfind(['.', ':']) {
            None => self.globals.get(expr),
            Some(cut) => {
                let base = &expr[..cut];
                let member = expr[cut + 1..].trim();
                self.members_after(base, idx)?.get(member)
            }
        }
    }
}

/// A segment of a dotted/method chain: an identifier and, if it is a call, the
/// first string-literal argument (for `GetService("X")` service resolution).
#[derive(Debug, Clone)]
pub struct Seg {
    pub name: String,
    pub arg: Option<String>,
}

/// Parse a base expression like `Enum.KeyCode` or `game:GetService("Input")`
/// into its segments. Returns `None` for anything that isn't a simple chain.
pub fn split_segments(base: &str) -> Option<Vec<Seg>> {
    let ch: Vec<char> = base.trim().chars().collect();
    let n = ch.len();
    if n == 0 {
        return None;
    }
    let mut i = 0;
    let mut segs = Vec::new();
    loop {
        if i >= n || !(ch[i].is_alphabetic() || ch[i] == '_') {
            return None;
        }
        let start = i;
        while i < n && (ch[i].is_alphanumeric() || ch[i] == '_') {
            i += 1;
        }
        let name: String = ch[start..i].iter().collect();
        let mut arg = None;
        if i < n && ch[i] == '(' {
            let (a, j) = scan_call(&ch, i)?;
            arg = a;
            i = j;
        }
        segs.push(Seg { name, arg });
        if i >= n {
            break;
        }
        if ch[i] == '.' || ch[i] == ':' {
            i += 1;
            continue;
        }
        return None;
    }
    Some(segs)
}

/// Given `ch[open] == '('`, return the first string-literal argument (if any)
/// and the index just past the matching `)`.
fn scan_call(ch: &[char], open: usize) -> Option<(Option<String>, usize)> {
    let n = ch.len();
    let mut depth = 0i32;
    let mut i = open;
    let mut arg: Option<String> = None;
    while i < n {
        let c = ch[i];
        if c == '"' || c == '\'' {
            let q = c;
            i += 1;
            let mut buf = String::new();
            while i < n && ch[i] != q {
                if ch[i] == '\\' && i + 1 < n {
                    i += 1;
                }
                buf.push(ch[i]);
                i += 1;
            }
            if depth == 1 && arg.is_none() {
                arg = Some(buf);
            }
            i += 1; // past closing quote
            continue;
        }
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            if depth == 0 {
                return Some((arg, i + 1));
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_json_parses() {
        let db = ApiDb::load();
        assert!(db.globals.contains_key("game"));
        assert!(db.globals.contains_key("Vec2"));
        assert!(db.types.contains_key("Instance"));
        assert!(db.types.contains_key("DataStore"));
    }

    #[test]
    fn resolves_library_and_type_members() {
        let db = ApiDb::load();
        let idx = SymbolIndex::default();
        let m = db.members_after("Vec2", &idx).unwrap();
        assert!(m.contains_key("new"));
        let m = db.members_after("Enum.KeyCode", &idx).unwrap();
        assert!(m.contains_key("Space"));
    }

    #[test]
    fn resolves_service_variable_members() {
        let db = ApiDb::load();
        let idx = SymbolIndex::build("local Input = game:GetService(\"Input\")\n");
        let m = db.members_after("Input", &idx).unwrap();
        assert!(m.contains_key("IsKeyDown"));
    }

    #[test]
    fn resolves_inline_getservice_chain() {
        let db = ApiDb::load();
        let idx = SymbolIndex::default();
        let m = db.members_after("game:GetService(\"DataStoreService\")", &idx).unwrap();
        assert!(m.contains_key("GetDataStore"));
    }

    #[test]
    fn resolve_member_finds_entries() {
        let db = ApiDb::load();
        let idx = SymbolIndex::default();
        assert!(db.resolve_member("Vec2.new", &idx).unwrap().is_callable());
        assert!(db.resolve_member("print", &idx).unwrap().is_callable());
    }
}
