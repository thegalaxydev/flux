//! Headless smoke test for the `projects/dino_run` sample: loads the scene,
//! starts a run with a Space press, then runs idle until the grounded player
//! collides with a scrolling cactus and the game-over state appears. Verifies
//! the whole loop (input → jump/gravity → spawning → collision → persistence)
//! runs without a single script error.
//!
//! Run with: `cargo run -p flux_runtime --example dino_smoke`

use std::path::Path;

use flux_core::{Value, World};
use flux_runtime::{DataBackend, InputFrame, LogLevel, Session, SessionOptions};
use glam::Vec2;

const VP: Vec2 = Vec2::new(960.0, 600.0);

fn frame(keys: &[&str]) -> InputFrame {
    InputFrame {
        keys: keys.iter().map(|s| s.to_string()).collect(),
        viewport: VP,
        ..Default::default()
    }
}

fn gui_child(w: &World, name: &str) -> Option<flux_core::InstanceId> {
    let gui = w.gui()?;
    w.children(gui).iter().copied().find(|&c| w.name(c) == Some(name))
}

fn label(w: &World, name: &str) -> String {
    match gui_child(w, name).and_then(|id| w.get_prop(id, "Text")) {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn game_over_visible(w: &World) -> bool {
    matches!(
        gui_child(w, "GameOver").and_then(|id| w.get_prop(id, "Visible")),
        Some(Value::Bool(true))
    )
}

fn count_class(w: &World, class: &str) -> usize {
    w.descendants(w.workspace())
        .into_iter()
        .filter(|&id| w.class_name(id) == Some(class))
        .count()
}

fn check_no_errors(session: &Session, label: &str) {
    for e in session.drain_logs() {
        println!("  [{label}] {:?} {}", e.level, e.message);
        assert_ne!(e.level, LogLevel::Error, "script error during {label}: {}", e.message);
    }
}

fn main() {
    let root = Path::new("projects/dino_run");
    let json = std::fs::read_to_string(root.join("main.scene.json")).expect("read scene");
    let mut session = Session::launch(&json, root, SessionOptions { data: DataBackend::SqliteMemory })
        .expect("load scene");
    check_no_errors(&session, "load");

    // A couple of idle frames in the "ready" state.
    for _ in 0..3 {
        session.step(0.016, &frame(&[]));
    }
    check_no_errors(&session, "ready");

    // Press Space (edge) then release to begin the run.
    session.step(0.016, &frame(&["Space"]));
    session.step(0.016, &frame(&[]));
    check_no_errors(&session, "start");

    // Run idle: the grounded player never jumps, so an incoming cactus must
    // eventually hit it. Track that obstacles spawn and that we score along the way.
    let mut max_obstacles = 0;
    let mut top_score = 0;
    let mut died_at = None;
    for i in 0..800 {
        session.step(0.016, &frame(&[]));
        check_no_errors(&session, "run");
        let w = session.world();
        let w = w.borrow();
        max_obstacles = max_obstacles.max(count_class(&w, "Sprite"));
        let score: i64 = label(&w, "Score")
            .trim_start_matches("Score  ")
            .parse()
            .unwrap_or(0);
        top_score = top_score.max(score);
        if game_over_visible(&w) {
            died_at = Some(i);
            break;
        }
    }

    println!("  peak sprite count = {max_obstacles}, top score = {top_score}");
    assert!(top_score > 0, "score should advance while running");
    assert!(max_obstacles > 5, "obstacles should have spawned into the workspace");
    let died_at = died_at.expect("a grounded player should eventually crash into a cactus");
    println!("  game over at frame {died_at}");

    let over = label(&session.world().borrow(), "GameOver");
    println!("  game-over label = {over:?}");
    assert!(over.contains("GAME OVER"), "game-over banner should show the result");

    // Retry: pressing Space again should restart cleanly.
    session.step(0.016, &frame(&["Space"]));
    session.step(0.016, &frame(&[]));
    session.step(0.016, &frame(&[]));
    check_no_errors(&session, "retry");
    assert!(!game_over_visible(&session.world().borrow()), "retry should clear game over");

    println!("dino_smoke: OK");
}
