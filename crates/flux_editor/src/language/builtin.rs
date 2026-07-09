//! Builtin symbol database for validation. Wraps the embedded [`ApiDb`] (the
//! documented Flux/Roblox surface) and adds an allowlist of always-in-scope
//! standard globals so we never warn "unknown global" for e.g. `select` or `os`.

use indexmap::IndexMap;

use super::api::{ApiDb, Entry};

/// Standard globals that always exist (superset of the documented ApiDb) — used
/// only to suppress "unknown global"/"undefined variable" for names we don't
/// necessarily have docs for.
const RESERVED: &[&str] = &[
    "_G", "_VERSION", "self", "select", "next", "unpack", "rawget", "rawset", "rawequal", "rawlen",
    "setmetatable", "getmetatable", "xpcall", "collectgarbage", "newproxy", "gcinfo", "tick",
    "os", "coroutine", "bit32", "utf8", "debug", "buffer", "Instance", "Random", "DateTime",
];

pub struct Builtins {
    db: ApiDb,
}

impl Builtins {
    pub fn load() -> Self {
        Builtins { db: ApiDb::load() }
    }

    pub fn db(&self) -> &ApiDb {
        &self.db
    }

    /// Is `name` a known global (documented or a reserved standard name)?
    pub fn is_global(&self, name: &str) -> bool {
        self.db.global(name).is_some() || RESERVED.contains(&name)
    }

    pub fn global(&self, name: &str) -> Option<&Entry> {
        self.db.global(name)
    }

    /// Members of a documented global namespace/library (e.g. `Vec2`, `math`).
    pub fn library_members(&self, name: &str) -> Option<&IndexMap<String, Entry>> {
        self.db.global(name).and_then(|g| g.members.as_ref())
    }

    /// Members of a documented value type (e.g. `Instance`, `DataStore`).
    pub fn type_members(&self, ty: &str) -> Option<&IndexMap<String, Entry>> {
        self.db.members_of_type(ty)
    }

    pub fn service_type(&self, service: &str) -> Option<String> {
        self.db.services.get(service).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_and_reserved_globals() {
        let b = Builtins::load();
        assert!(b.is_global("game"));
        assert!(b.is_global("math"));
        assert!(b.is_global("os")); // reserved, not necessarily documented
        assert!(!b.is_global("definitelyNotAGlobal"));
    }

    #[test]
    fn service_and_members() {
        let b = Builtins::load();
        assert_eq!(b.service_type("Input").as_deref(), Some("InputService"));
        assert!(b.type_members("Instance").unwrap().contains_key("GetService"));
        assert!(b.library_members("Vec2").unwrap().contains_key("new"));
    }
}
