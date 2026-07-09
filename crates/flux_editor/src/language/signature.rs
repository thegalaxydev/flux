//! Signature help: while the cursor is inside a call's argument list, describe
//! the callee and highlight the active parameter.

use super::api::ApiDb;
use super::context::enclosing_call;
use super::symbols::SymbolIndex;

#[derive(Debug, Clone, PartialEq)]
pub struct SignatureHelp {
    pub name: String,
    /// Each parameter rendered as `name` or `name: type`.
    pub params: Vec<String>,
    /// Index into `params` of the argument being typed (clamped; usize::MAX if
    /// there are no parameters).
    pub active: usize,
    pub returns: Option<String>,
    pub doc: String,
}

#[derive(Default)]
pub struct SignatureHelpProvider;

impl SignatureHelpProvider {
    pub fn signature(
        &self,
        db: &ApiDb,
        idx: &SymbolIndex,
        src: &str,
        cursor: usize,
    ) -> Option<SignatureHelp> {
        let (func_expr, arg) = enclosing_call(src, cursor)?;
        let entry = db.resolve_member(&func_expr, idx)?;
        if !entry.is_callable() {
            return None;
        }
        let name = func_expr
            .rsplit(['.', ':'])
            .next()
            .unwrap_or(&func_expr)
            .to_string();
        let params: Vec<String> = entry
            .params
            .iter()
            .map(|p| match &p.ty {
                Some(t) => format!("{}: {}", p.name, t),
                None => p.name.clone(),
            })
            .collect();

        let active = if params.is_empty() {
            usize::MAX
        } else if entry.params.last().is_some_and(|p| p.is_variadic()) && arg >= params.len() {
            params.len() - 1
        } else {
            arg.min(params.len() - 1)
        };

        Some(SignatureHelp {
            name,
            params,
            active,
            returns: entry.returns.clone().filter(|r| r != "()"),
            doc: entry.doc.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_active_parameter() {
        let db = ApiDb::load();
        let idx = SymbolIndex::default();
        let src = "Vec2.new(1, ";
        let sig = SignatureHelpProvider.signature(&db, &idx, src, src.len()).unwrap();
        assert_eq!(sig.name, "new");
        assert_eq!(sig.active, 1);
        assert_eq!(sig.params.len(), 2);
    }

    #[test]
    fn method_call_signature() {
        let db = ApiDb::load();
        let src = "local dss = game:GetService(\"DataStoreService\")\nlocal store = dss:GetDataStore(\"s\")\nstore:SetAsync(\"k\", ";
        let idx = SymbolIndex::build(src);
        let sig = SignatureHelpProvider.signature(&db, &idx, src, src.len()).unwrap();
        assert_eq!(sig.name, "SetAsync");
        assert_eq!(sig.active, 1);
    }

    #[test]
    fn variadic_clamps_to_last() {
        let db = ApiDb::load();
        let idx = SymbolIndex::default();
        let src = "print(a, b, c, ";
        let sig = SignatureHelpProvider.signature(&db, &idx, src, src.len()).unwrap();
        assert_eq!(sig.active, 0); // print has one variadic parameter "..."
    }
}
