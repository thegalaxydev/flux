use flux_data::{DataError, PersistenceProvider, SqliteProvider};
use serde_json::json;

fn provider() -> SqliteProvider {
    SqliteProvider::open_in_memory().unwrap()
}

#[test]
fn set_then_get() {
    let p = provider();
    assert_eq!(p.get("global", "s", "k").unwrap(), None);
    p.set("global", "s", "k", &json!({"hp": 10, "name": "x"})).unwrap();
    assert_eq!(p.get("global", "s", "k").unwrap(), Some(json!({"hp": 10, "name": "x"})));
}

#[test]
fn remove_returns_old_and_clears() {
    let p = provider();
    p.set("global", "s", "k", &json!(7)).unwrap();
    assert_eq!(p.remove("global", "s", "k").unwrap(), Some(json!(7)));
    assert_eq!(p.get("global", "s", "k").unwrap(), None);
    assert_eq!(p.remove("global", "s", "k").unwrap(), None);
}

#[test]
fn increment_missing_and_existing() {
    let p = provider();
    assert_eq!(p.increment("global", "s", "coins", 5.0).unwrap(), 5.0);
    assert_eq!(p.increment("global", "s", "coins", 3.0).unwrap(), 8.0);
    assert_eq!(p.get("global", "s", "coins").unwrap(), Some(json!(8)));
}

#[test]
fn increment_rejects_non_number() {
    let p = provider();
    p.set("global", "s", "k", &json!("hello")).unwrap();
    assert!(matches!(
        p.increment("global", "s", "k", 1.0),
        Err(DataError::NotANumber(_))
    ));
}

#[test]
fn update_increments_existing_value() {
    let p = provider();
    p.set("global", "s", "score", &json!(10)).unwrap();
    let out = p
        .update("global", "s", "score", &mut |cur| {
            let n = cur.and_then(|v| v.as_i64()).unwrap_or(0);
            Ok(Some(json!(n + 1)))
        })
        .unwrap();
    assert_eq!(out, Some(json!(11)));
    assert_eq!(p.get("global", "s", "score").unwrap(), Some(json!(11)));
}

#[test]
fn update_creates_missing_value() {
    let p = provider();
    let out = p
        .update("global", "s", "fresh", &mut |cur| {
            assert_eq!(cur, None);
            Ok(Some(json!(1)))
        })
        .unwrap();
    assert_eq!(out, Some(json!(1)));
    assert_eq!(p.get("global", "s", "fresh").unwrap(), Some(json!(1)));
}

#[test]
fn update_returning_none_removes_value() {
    let p = provider();
    p.set("global", "s", "k", &json!(99)).unwrap();
    let out = p.update("global", "s", "k", &mut |_| Ok(None)).unwrap();
    assert_eq!(out, None);
    assert_eq!(p.get("global", "s", "k").unwrap(), None);
}

#[test]
fn update_bumps_version() {
    let p = provider();
    p.set("global", "s", "k", &json!(1)).unwrap();
    for _ in 0..3 {
        p.update("global", "s", "k", &mut |c| {
            Ok(Some(json!(c.and_then(|v| v.as_i64()).unwrap_or(0) + 1)))
        })
        .unwrap();
    }
    assert_eq!(p.get("global", "s", "k").unwrap(), Some(json!(4)));
}

#[test]
fn separate_stores_do_not_collide() {
    let p = provider();
    p.set("global", "store_a", "k", &json!("a")).unwrap();
    p.set("global", "store_b", "k", &json!("b")).unwrap();
    assert_eq!(p.get("global", "store_a", "k").unwrap(), Some(json!("a")));
    assert_eq!(p.get("global", "store_b", "k").unwrap(), Some(json!("b")));
}

#[test]
fn separate_scopes_do_not_collide() {
    let p = provider();
    p.set("player_1", "s", "k", &json!(1)).unwrap();
    p.set("player_2", "s", "k", &json!(2)).unwrap();
    assert_eq!(p.get("player_1", "s", "k").unwrap(), Some(json!(1)));
    assert_eq!(p.get("player_2", "s", "k").unwrap(), Some(json!(2)));
}

#[test]
fn list_keys_scoped_to_store() {
    let p = provider();
    p.set("global", "s", "b", &json!(1)).unwrap();
    p.set("global", "s", "a", &json!(1)).unwrap();
    p.set("global", "other", "z", &json!(1)).unwrap();
    assert_eq!(p.list_keys("global", "s").unwrap(), vec!["a", "b"]);
}
