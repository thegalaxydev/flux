use std::path::Path;

use flux_core::{InstanceId, UDim2, Value, World};
use flux_runtime::{InputFrame, LogLevel, Session};
use glam::Vec2;

fn fixtures() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"))
}

fn idle() -> InputFrame {
    InputFrame::default()
}

fn with_keys(keys: &[&str]) -> InputFrame {
    InputFrame {
        keys: keys.iter().map(|k| k.to_string()).collect(),
        ..Default::default()
    }
}

fn scene_with_script(script_path: &str) -> String {
    let mut w = World::new();
    let ws = w.workspace();
    let storage = w.service("Storage").unwrap();

    let hero = w.create("Sprite", ws).unwrap();
    w.set_name(hero, "Hero").unwrap();

    let script = w.create("Script", hero).unwrap();
    w.set_prop(script, "SourcePath", Value::Asset(script_path.to_string()))
        .unwrap();

    let template = w.create("Sprite", storage).unwrap();
    w.set_name(template, "Template").unwrap();

    w.to_json()
}

fn hero_of(session: &Session) -> InstanceId {
    let rc = session.world();
    let w = rc.borrow();
    let ws = w.workspace();
    w.find_first_child(ws, "Hero")
        .or_else(|| w.find_first_child(ws, "Waited"))
        .or_else(|| w.find_first_child(ws, "KeyDown"))
        .expect("hero sprite missing")
}

fn pos_of(session: &Session, id: InstanceId) -> Vec2 {
    let rc = session.world();
    let w = rc.borrow();
    match w.get_prop(id, "Position") {
        Some(Value::Vec2(v)) => *v,
        other => panic!("unexpected position {other:?}"),
    }
}

#[test]
fn scripts_run_and_drive_the_world() {
    let json = scene_with_script("scripts/test_basic.luau");
    let mut session = Session::from_scene_json(&json, fixtures()).unwrap();
    let hero = hero_of(&session);

    let logs = session.drain_logs();
    assert!(
        logs.iter()
            .any(|l| l.level == LogLevel::Info && l.message == "hello Hero"),
        "print not captured: {logs:?}"
    );

    assert_eq!(pos_of(&session, hero), Vec2::new(1.0, 2.0));
    {
        let rc = session.world();
        let w = rc.borrow();
        let cloned = w.find_first_child(w.workspace(), "ClonedThing");
        assert!(cloned.is_some(), "Clone + Parent assignment failed");
        assert_eq!(w.class_name(cloned.unwrap()), Some("Sprite"));
    }

    session.step(0.1, &idle());
    let pos = pos_of(&session, hero);
    assert!((pos.x - 2.0).abs() < 1e-4, "heartbeat did not move hero: {pos}");
    {
        let rc = session.world();
        let w = rc.borrow();
        assert_eq!(w.name(hero), Some("Waited"), "task.wait did not resume");
    }
}

#[test]
fn input_reaches_scripts() {
    let json = scene_with_script("scripts/test_input.luau");
    let mut session = Session::from_scene_json(&json, fixtures()).unwrap();
    let hero = hero_of(&session);

    session.step(0.016, &idle());
    {
        let rc = session.world();
        assert_eq!(rc.borrow().name(hero), Some("Hero"));
    }

    session.step(0.016, &with_keys(&["A"]));
    let rc = session.world();
    assert_eq!(rc.borrow().name(hero), Some("KeyDown"));
}

#[test]
fn enum_keycodes_resolve_to_input_tokens() {
    let json = scene_with_script("scripts/test_enum.luau");
    let mut session = Session::from_scene_json(&json, fixtures()).unwrap();
    let hero = hero_of(&session);

    // The top-level `assert`s in the fixture must not have logged an error.
    let logs = session.drain_logs();
    assert!(
        !logs.iter().any(|l| l.level == LogLevel::Error),
        "enum fixture reported errors: {logs:?}"
    );

    // `Enum.KeyCode.Left` maps to the engine's "ArrowLeft" token.
    session.step(0.016, &with_keys(&["ArrowLeft"]));
    let rc = session.world();
    assert_eq!(rc.borrow().name(hero), Some("MovedLeft"));
}

#[test]
fn gui_absolute_position_uses_scale_and_viewport() {
    let mut w = World::new();
    let gui = w.gui().unwrap();
    let frame = w.create("Frame", gui).unwrap();
    w.set_name(frame, "Panel").unwrap();
    let script = w.create("Script", frame).unwrap();
    w.set_prop(script, "SourcePath", Value::Asset("scripts/test_gui.luau".into()))
        .unwrap();
    let json = w.to_json();

    let mut session = Session::from_scene_json(&json, fixtures()).unwrap();
    let rc = session.world();
    let panel = rc
        .borrow()
        .gui()
        .and_then(|g| rc.borrow().find_first_child(g, "Panel"))
        .unwrap();

    let frame_input = InputFrame {
        viewport: Vec2::new(200.0, 100.0),
        ..Default::default()
    };
    session.step(0.016, &frame_input);
    assert_eq!(
        rc.borrow().name(panel),
        Some("AbsOk"),
        "AbsolutePosition/AbsoluteSize did not resolve scale against the viewport"
    );
}

#[test]
fn script_errors_are_logged_not_fatal() {
    let json = scene_with_script("scripts/test_error.luau");
    let mut session = Session::from_scene_json(&json, fixtures()).unwrap();
    let logs = session.drain_logs();
    assert!(
        logs.iter().any(|l| l.level == LogLevel::Error),
        "expected an error log entry: {logs:?}"
    );
    session.step(0.016, &idle());
}

#[test]
fn button_activated_fires_on_click() {
    let mut w = World::new();
    let gui = w.gui().unwrap();
    let button = w.create("Button", gui).unwrap();
    w.set_name(button, "Btn").unwrap();
    w.set_prop(button, "Position", Value::UDim2(UDim2::new(0.0, 50.0, 0.0, 40.0)))
        .unwrap();
    w.set_prop(button, "Size", Value::UDim2(UDim2::new(0.0, 120.0, 0.0, 30.0)))
        .unwrap();
    let script = w.create("Script", button).unwrap();
    w.set_prop(script, "SourcePath", Value::Asset("scripts/button.luau".into()))
        .unwrap();
    let json = w.to_json();

    let mut session = Session::from_scene_json(&json, fixtures()).unwrap();
    let rc = session.world();
    let btn = rc.borrow().gui().and_then(|g| rc.borrow().find_first_child(g, "Btn")).unwrap();

    let viewport = Vec2::new(800.0, 600.0);
    let press_inside = InputFrame {
        mouse_pos: Vec2::new(90.0, 55.0),
        mouse_buttons: ["Left".to_string()].into(),
        viewport,
        ..Default::default()
    };
    let release = InputFrame {
        mouse_pos: Vec2::new(90.0, 55.0),
        viewport,
        ..Default::default()
    };
    session.step(0.016, &release);
    assert_eq!(rc.borrow().name(btn), Some("Btn"));
    session.step(0.016, &press_inside);
    assert_eq!(rc.borrow().name(btn), Some("Clicked"), "Activated did not fire");

    session.step(0.016, &release);
    rc.borrow_mut().set_name(btn, "Btn").ok();
    let press_outside = InputFrame {
        mouse_pos: Vec2::new(400.0, 400.0),
        mouse_buttons: ["Left".to_string()].into(),
        viewport,
        ..Default::default()
    };
    session.step(0.016, &press_outside);
    assert_ne!(
        rc.borrow().name(btn),
        Some("Clicked"),
        "click outside should not fire Activated"
    );
}

fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "flux_ds_{tag}_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    dir
}

fn launch_inline(root: &Path, src: &str, backend: flux_runtime::DataBackend) -> Session {
    std::fs::write(root.join("scripts/main.luau"), src).unwrap();
    let mut w = World::new();
    let s = w.create("Script", w.workspace()).unwrap();
    w.set_prop(s, "SourcePath", Value::Asset("scripts/main.luau".into()))
        .unwrap();
    Session::launch(&w.to_json(), root, flux_runtime::SessionOptions { data: backend }).unwrap()
}

#[test]
fn datastore_service_persists_across_sessions() {
    let dir = temp_root("persist");
    let db = flux_runtime::DataBackend::SqliteFile(dir.join(".flux/data/playtest.sqlite"));

    let writer = "game:GetService(\"DataStoreService\"):GetDataStore(\"scores\"):SetAsync(\"best\", 42)";
    drop(launch_inline(&dir, writer, db.clone()));

    let reader = "print(\"best is \" .. game:GetService(\"DataStoreService\"):GetDataStore(\"scores\"):GetAsync(\"best\"))";
    let session = launch_inline(&dir, reader, db);
    let logs = session.drain_logs();
    assert!(
        logs.iter().any(|l| l.message == "best is 42"),
        "value did not persist across sessions: {logs:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn datastore_update_and_increment_via_lua() {
    let dir = temp_root("update");
    let src = r#"
        local store = game:GetService("DataStoreService"):GetDataStore("s")
        local n = store:UpdateAsync("v", function(old) return (old or 0) + 5 end)
        print("update " .. n)
        local c = store:IncrementAsync("clicks", 3)
        print("increment " .. c)
        store:SetAsync("gone", 1)
        store:UpdateAsync("gone", function(old) return nil end)
        print("gone is " .. tostring(store:GetAsync("gone")))
    "#;
    let session = launch_inline(&dir, src, flux_runtime::DataBackend::SqliteMemory);
    let logs = session.drain_logs();
    let has = |msg: &str| logs.iter().any(|l| l.message == msg);
    assert!(has("update 5"), "UpdateAsync on missing key: {logs:?}");
    assert!(has("increment 3"), "IncrementAsync: {logs:?}");
    assert!(has("gone is nil"), "UpdateAsync returning nil should remove: {logs:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn datastore_rejects_unsupported_value() {
    let dir = temp_root("unsupported");
    let src = "game:GetService(\"DataStoreService\"):GetDataStore(\"s\"):SetAsync(\"k\", function() end)";
    let session = launch_inline(&dir, src, flux_runtime::DataBackend::SqliteMemory);
    let logs = session.drain_logs();
    assert!(
        logs.iter()
            .any(|l| l.level == LogLevel::Error && l.message.contains("cannot store a function")),
        "expected an unsupported-value error: {logs:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn script_error_reports_file_and_line() {
    let dir = temp_root("errloc");
    let src = "local a\nlocal b = a.x\n";
    let session = launch_inline(&dir, src, flux_runtime::DataBackend::SqliteMemory);
    let logs = session.drain_logs();
    assert!(
        logs.iter()
            .any(|l| l.level == LogLevel::Error && l.message.contains("main.luau:2")),
        "error should report file:line: {logs:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn datastore_rejects_cyclic_table() {
    let dir = temp_root("cyclic");
    let src = r#"
        local t = {}
        t.self = t
        game:GetService("DataStoreService"):GetDataStore("s"):SetAsync("k", t)
    "#;
    let session = launch_inline(&dir, src, flux_runtime::DataBackend::SqliteMemory);
    let logs = session.drain_logs();
    assert!(
        logs.iter()
            .any(|l| l.level == LogLevel::Error && l.message.contains("cyclic")),
        "expected a cyclic-table error: {logs:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn get_touching_sprites_detects_overlap() {
    let mut w = World::new();
    let ws = w.workspace();
    let a = w.create("Sprite", ws).unwrap();
    w.set_name(a, "A").unwrap();
    w.set_prop(a, "Position", Value::Vec2(Vec2::ZERO)).unwrap();
    w.set_prop(a, "Size", Value::Vec2(Vec2::new(50.0, 50.0))).unwrap();
    let b = w.create("Sprite", ws).unwrap();
    w.set_name(b, "B").unwrap();
    w.set_prop(b, "Position", Value::Vec2(Vec2::new(30.0, 0.0))).unwrap();
    w.set_prop(b, "Size", Value::Vec2(Vec2::new(50.0, 50.0))).unwrap();
    let far = w.create("Sprite", ws).unwrap();
    w.set_name(far, "Far").unwrap();
    w.set_prop(far, "Position", Value::Vec2(Vec2::new(500.0, 0.0))).unwrap();
    w.set_prop(far, "Size", Value::Vec2(Vec2::new(50.0, 50.0))).unwrap();

    let script = w.create("Script", a).unwrap();
    w.set_prop(script, "SourcePath", Value::Asset("scripts/touching.luau".into()))
        .unwrap();
    let json = w.to_json();

    let session = Session::from_scene_json(&json, fixtures()).unwrap();
    let logs = session.drain_logs();
    assert!(
        logs.iter().any(|l| l.message == "touching 1"),
        "expected exactly one touching sprite: {logs:?}"
    );
}

#[test]
fn missing_script_file_logs_error() {
    let json = scene_with_script("scripts/does_not_exist.luau");
    let session = Session::from_scene_json(&json, fixtures()).unwrap();
    let logs = session.drain_logs();
    assert!(logs.iter().any(|l| l.level == LogLevel::Error));
}

#[test]
fn scripts_in_the_scripts_container_run() {
    // A Script parented under the top-level Scripts service should run without
    // being attached to Workspace/Gui/Storage.
    let mut w = World::new();
    let scripts = w.scripts().expect("Scripts service");
    let s = w.create("Script", scripts).unwrap();
    w.set_prop(s, "SourcePath", Value::Asset("scripts/top_level.luau".to_string()))
        .unwrap();
    let json = w.to_json();

    let session = Session::from_scene_json(&json, fixtures()).unwrap();
    let logs = session.drain_logs();
    assert!(
        logs.iter()
            .any(|l| l.level == LogLevel::Info && l.message == "top-level ran under Scripts"),
        "top-level script did not run: {logs:?}"
    );
}

// --- Modules (require) -----------------------------------------------------

/// Build a world with the standard services plus the given (class, name, path)
/// instances under the Scripts container.
fn scene_with(items: &[(&str, &str, &str)]) -> String {
    let mut w = World::new();
    let scripts = w.scripts().expect("Scripts service");
    for (class, name, path) in items {
        let id = w.create(class, scripts).unwrap();
        w.set_name(id, *name).unwrap();
        w.set_prop(id, "SourcePath", Value::Asset(path.to_string())).unwrap();
    }
    w.to_json()
}

#[test]
fn require_loads_a_module_once_and_caches_it() {
    let json = scene_with(&[
        ("Module", "Balance", "scripts/balance_module.luau"),
        ("Script", "User", "scripts/use_module.luau"),
    ]);
    let session = Session::from_scene_json(&json, fixtures()).unwrap();
    let logs = session.drain_logs();
    let msgs: Vec<&str> = logs.iter().map(|l| l.message.as_str()).collect();

    assert!(logs.iter().all(|l| l.level != LogLevel::Error), "unexpected error: {logs:?}");
    // Module body ran exactly once despite two requires.
    assert_eq!(msgs.iter().filter(|m| **m == "module loaded").count(), 1, "{msgs:?}");
    assert!(msgs.contains(&"speed 240"), "module data not returned: {msgs:?}");
    assert!(msgs.contains(&"same true"), "require should return the cached value: {msgs:?}");
}

#[test]
fn modules_do_not_run_on_their_own() {
    // A Module with no `require` must never execute at startup.
    let json = scene_with(&[("Module", "Noisy", "scripts/noisy_module.luau")]);
    let session = Session::from_scene_json(&json, fixtures()).unwrap();
    let logs = session.drain_logs();
    assert!(
        logs.iter().all(|l| l.message != "SHOULD NOT RUN"),
        "a Module was auto-run: {logs:?}"
    );
}

#[test]
fn cyclic_require_is_reported() {
    let json = scene_with(&[
        ("Module", "CycleA", "scripts/cycle_a.luau"),
        ("Module", "CycleB", "scripts/cycle_b.luau"),
        ("Script", "Start", "scripts/require_cycle.luau"),
    ]);
    let session = Session::from_scene_json(&json, fixtures()).unwrap();
    let logs = session.drain_logs();
    assert!(
        logs.iter().any(|l| l.level == LogLevel::Error && l.message.contains("cyclic")),
        "expected a cyclic-require error: {logs:?}"
    );
}
