//! End-to-end: the built-in camera controller moves the `Camera2D` in response
//! to input threaded through a runtime `Session` (keyboard pan + wheel zoom),
//! and stays put when `Controls` is off.

use std::path::Path;

use flux_core::{InstanceId, Value, World};
use flux_runtime::{InputFrame, Session};
use glam::Vec2;

fn scene(controls: bool, pan_speed: f64) -> String {
    let mut w = World::new();
    let ws = w.workspace();
    let cam = match w.get_prop(ws, "CurrentCamera") {
        Some(Value::InstanceRef(Some(c))) => *c,
        _ => panic!("no current camera"),
    };
    w.set_prop(cam, "Controls", Value::Bool(controls)).unwrap();
    w.set_prop(cam, "PanSpeed", Value::Number(pan_speed)).unwrap();
    w.to_json()
}

fn camera_of(session: &Session) -> InstanceId {
    let rc = session.world();
    let w = rc.borrow();
    match w.get_prop(w.workspace(), "CurrentCamera") {
        Some(Value::InstanceRef(Some(c))) => *c,
        _ => panic!("no current camera"),
    }
}

fn read(session: &Session, cam: InstanceId, prop: &str) -> Value {
    session.world().borrow().get_prop(cam, prop).unwrap().clone()
}

fn frame(keys: &[&str], scroll: f32) -> InputFrame {
    InputFrame {
        keys: keys.iter().map(|k| k.to_string()).collect(),
        viewport: Vec2::new(800.0, 600.0),
        mouse_pos: Vec2::new(400.0, 300.0),
        pointer_over: true,
        scroll,
        ..Default::default()
    }
}

#[test]
fn keyboard_pan_moves_the_camera() {
    let mut session = Session::from_scene_json(&scene(true, 100.0), Path::new(".")).unwrap();
    let cam = camera_of(&session);
    session.step(1.0, &frame(&["D"], 0.0));
    match read(&session, cam, "Position") {
        Value::Vec2(p) => assert!((p.x - 100.0).abs() < 1e-3, "pan x = {}", p.x),
        v => panic!("unexpected {v:?}"),
    }
}

#[test]
fn wheel_zooms_in() {
    let mut session = Session::from_scene_json(&scene(true, 100.0), Path::new(".")).unwrap();
    let cam = camera_of(&session);
    for _ in 0..10 {
        session.step(0.1, &frame(&[], 120.0));
    }
    match read(&session, cam, "Zoom") {
        Value::Number(z) => assert!(z > 1.05, "zoom did not increase: {z}"),
        v => panic!("unexpected {v:?}"),
    }
}

#[test]
fn disabled_controls_leave_camera_still() {
    let mut session = Session::from_scene_json(&scene(false, 100.0), Path::new(".")).unwrap();
    let cam = camera_of(&session);
    session.step(1.0, &frame(&["D"], 5.0));
    assert_eq!(read(&session, cam, "Position"), Value::Vec2(Vec2::ZERO));
    assert_eq!(read(&session, cam, "Zoom"), Value::Number(1.0));
}
