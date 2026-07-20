use std::path::Path;

use flux_core::{Value, World};
use flux_runtime::{DataBackend, InputFrame, Session, SessionOptions};
use glam::Vec2;

const VP: Vec2 = Vec2::new(960.0, 600.0);

fn idle() -> InputFrame {
    InputFrame {
        viewport: VP,
        ..Default::default()
    }
}

fn click(pos: Vec2) -> InputFrame {
    InputFrame {
        mouse_pos: pos,
        mouse_buttons: ["Left".to_string()].into(),
        viewport: VP,
        ..Default::default()
    }
}

fn gui_child(w: &World, name: &str) -> Option<flux_core::InstanceId> {
    let gui = w.gui()?;
    w.children(gui).iter().copied().find(|&c| w.name(c) == Some(name))
}

fn text_of(w: &World, name: &str) -> String {
    match gui_child(w, name).and_then(|id| w.get_prop(id, "Text")) {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn dump(session: &Session, label: &str) {
    for e in session.drain_logs() {
        println!("  [{label}] {:?} {}", e.level, e.message);
    }
}

fn main() {
    let root = Path::new("projects/flux_sample_clicker");
    let db_path = root.join(".flux/data/playtest.sqlite");
    let _ = std::fs::remove_file(&db_path);

    let json = std::fs::read_to_string(root.join("main.scene.json")).expect("read scene");
    let options = SessionOptions {
        data: DataBackend::SqliteFile(db_path.clone()),
        ..Default::default()
    };
    let mut session = Session::launch(&json, root, options).expect("load scene");
    dump(&session, "load");

    // Click the Start button (pos 410,290 size 160,40 -> center 490,310).
    session.step(0.016, &idle());
    session.step(0.016, &click(Vec2::new(490.0, 310.0)));
    session.step(0.016, &idle());
    dump(&session, "start");

    let world = session.world();
    let center = {
        let w = world.borrow();
        let target = gui_child(&w, "TargetTemplate").expect("no target spawned after Start");
        let rect = flux_core::gui::absolute_rect(&w, target, flux_core::Rect2::from_screen(VP))
            .expect("target has no absolute rect");
        println!("  target spawned at {:?} size {:?}", rect.min, rect.size);
        rect.center()
    };

    session.step(0.016, &click(Vec2::new(center.x, center.y)));
    session.step(0.016, &idle());
    dump(&session, "hit");

    let score = text_of(&world.borrow(), "Score");
    println!("  score label = {score:?}");
    assert_eq!(score, "Score: 1", "clicking a target should score a point");

    // Fast-forward past the 30s round so it ends and persists Best + total clicks.
    for _ in 0..70 {
        session.step(0.5, &idle());
    }
    dump(&session, "end");
    drop(session);

    // Reopen the same database in a fresh session; the clicker reads Best/Clicks
    // back on startup and prints them.
    let reopen = Session::launch(
        &json,
        root,
        SessionOptions {
            data: DataBackend::SqliteFile(db_path.clone()),
            ..Default::default()
        },
    )
    .expect("reopen");
    let logs = reopen.drain_logs();
    for e in &logs {
        println!("  [reopen] {:?} {}", e.level, e.message);
    }
    assert!(
        logs.iter()
            .any(|l| l.message.contains("best is 1") && l.message.contains("total clicks 1")),
        "clicker did not read persisted Best/Clicks on reload"
    );
    drop(reopen);
    let _ = std::fs::remove_dir_all(root.join(".flux"));

    println!("CLICKER SMOKE OK");
}
