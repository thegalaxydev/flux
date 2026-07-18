use flux_core::{Color, CoreError, UDim2, Value, World};
use glam::Vec2;

#[test]
fn new_world_has_services() {
    let w = World::new();
    let ws = w.workspace();
    assert_eq!(w.class_name(ws), Some("Workspace"));
    assert!(w.service("Storage").is_some());
    let Some(Value::InstanceRef(Some(cam))) = w.get_prop(ws, "CurrentCamera") else {
        panic!("CurrentCamera not set");
    };
    assert_eq!(w.class_name(*cam), Some("Camera2D"));
}

#[test]
fn create_applies_defaults_and_set_prop_type_checks() {
    let mut w = World::new();
    let ws = w.workspace();
    let sprite = w.create("Sprite", ws).unwrap();
    assert_eq!(w.name(sprite), Some("Sprite"));
    assert_eq!(
        w.get_prop(sprite, "Position"),
        Some(&Value::Vec2(Vec2::ZERO))
    );
    assert_eq!(w.get_prop(sprite, "Visible"), Some(&Value::Bool(true)));

    w.set_prop(sprite, "Position", Value::Vec2(Vec2::new(10.0, 20.0)))
        .unwrap();
    assert_eq!(
        w.get_prop(sprite, "Position"),
        Some(&Value::Vec2(Vec2::new(10.0, 20.0)))
    );

    assert!(matches!(
        w.set_prop(sprite, "Position", Value::Bool(true)),
        Err(CoreError::TypeMismatch { .. })
    ));
    assert!(matches!(
        w.set_prop(sprite, "Nope", Value::Bool(true)),
        Err(CoreError::UnknownProperty(_))
    ));
}

#[test]
fn services_cannot_be_created_destroyed_or_renamed() {
    let mut w = World::new();
    let ws = w.workspace();
    assert!(matches!(
        w.create("Game", ws),
        Err(CoreError::NotCreatable(_))
    ));
    assert!(matches!(
        w.create("Instance", ws),
        Err(CoreError::NotCreatable(_))
    ));
    assert!(matches!(w.destroy(ws), Err(CoreError::CannotModifyService)));
    assert!(matches!(
        w.destroy(w.root()),
        Err(CoreError::CannotModifyService)
    ));
    assert!(matches!(
        w.set_name(ws, "NotWorkspace"),
        Err(CoreError::CannotModifyService)
    ));
}

#[test]
fn destroy_removes_subtree() {
    let mut w = World::new();
    let ws = w.workspace();
    let folder = w.create("Folder", ws).unwrap();
    let sprite = w.create("Sprite", folder).unwrap();
    let script = w.create("Script", sprite).unwrap();

    w.destroy(folder).unwrap();
    assert!(!w.contains(folder));
    assert!(!w.contains(sprite));
    assert!(!w.contains(script));
    assert!(w.find_first_child(ws, "Folder").is_none());
    assert!(matches!(
        w.destroy(folder),
        Err(CoreError::InstanceNotFound)
    ));
}

#[test]
fn reparent_moves_and_rejects_cycles() {
    let mut w = World::new();
    let ws = w.workspace();
    let a = w.create("Folder", ws).unwrap();
    let b = w.create("Folder", a).unwrap();
    let sprite = w.create("Sprite", ws).unwrap();

    w.reparent(sprite, b).unwrap();
    assert_eq!(w.parent(sprite), Some(b));
    assert_eq!(w.children(b), &[sprite]);

    assert!(matches!(w.reparent(a, b), Err(CoreError::WouldCreateCycle)));
    assert!(matches!(w.reparent(a, a), Err(CoreError::WouldCreateCycle)));
    assert!(matches!(
        w.reparent(ws, a),
        Err(CoreError::CannotModifyService)
    ));
}

#[test]
fn reparent_at_orders_children() {
    let mut w = World::new();
    let ws = w.workspace();
    let a = w.create("Folder", ws).unwrap();
    let b = w.create("Folder", ws).unwrap();
    let c = w.create("Folder", ws).unwrap();
    assert_eq!(w.child_index(a), Some(1));

    w.reparent_at(c, ws, 1).unwrap();
    let kids: Vec<_> = w.children(ws).to_vec();
    let pos = |id| kids.iter().position(|&k| k == id).unwrap();
    assert!(pos(c) < pos(a));
    assert!(pos(a) < pos(b));
}

#[test]
fn snapshot_restore_preserves_structure_and_remaps_refs() {
    let mut w = World::new();
    let ws = w.workspace();
    let storage = w.service("Storage").unwrap();

    let folder = w.create("Folder", ws).unwrap();
    w.set_name(folder, "Enemies").unwrap();
    let s1 = w.create("Sprite", folder).unwrap();
    w.set_name(s1, "Grunt").unwrap();
    w.set_prop(s1, "Position", Value::Vec2(Vec2::new(5.0, 6.0)))
        .unwrap();
    let s2 = w.create("Sprite", folder).unwrap();
    w.set_name(s2, "Boss").unwrap();

    let snap = w.snapshot_subtree(folder).unwrap();
    w.destroy(folder).unwrap();
    assert!(!w.contains(s1));

    let map = w.restore_subtree(ws, 0, &snap).unwrap();
    let new_folder = map[&folder];
    assert_eq!(w.children(ws)[0], new_folder);
    assert_eq!(w.name(new_folder), Some("Enemies"));
    let kids = w.children(new_folder);
    assert_eq!(w.name(kids[0]), Some("Grunt"));
    assert_eq!(w.name(kids[1]), Some("Boss"));
    assert_eq!(
        w.get_prop(kids[0], "Position"),
        Some(&Value::Vec2(Vec2::new(5.0, 6.0)))
    );

    let ws_snap = w.snapshot_subtree(ws).unwrap();
    let ws_map = w.restore_subtree(storage, 0, &ws_snap).unwrap();
    let ws_copy = ws_map[&ws];
    let Some(Value::InstanceRef(Some(cam_copy))) = w.get_prop(ws_copy, "CurrentCamera") else {
        panic!("CurrentCamera missing on restored copy");
    };
    let Some(Value::InstanceRef(Some(cam_orig))) = w.get_prop(ws, "CurrentCamera") else {
        panic!("CurrentCamera missing on original");
    };
    assert_ne!(cam_copy, cam_orig);
    assert_eq!(*cam_copy, ws_map[cam_orig]);
}

#[test]
fn json_roundtrip_is_stable() {
    let mut w = World::new();
    let ws = w.workspace();
    let storage = w.service("Storage").unwrap();

    let player = w.create("Sprite", ws).unwrap();
    w.set_name(player, "Player").unwrap();
    w.set_prop(player, "Position", Value::Vec2(Vec2::new(-40.0, 12.5)))
        .unwrap();
    w.set_prop(
        player,
        "Tint",
        Value::Color(flux_core::Color::new(0.2, 0.5, 1.0, 1.0)),
    )
    .unwrap();

    let script = w.create("Script", player).unwrap();
    w.set_name(script, "Movement").unwrap();
    w.set_prop(
        script,
        "SourcePath",
        Value::Asset("scripts/movement.luau".into()),
    )
    .unwrap();

    let env = w.create("Folder", ws).unwrap();
    w.set_name(env, "Environment").unwrap();
    let ground = w.create("Sprite", env).unwrap();
    w.set_name(ground, "Ground").unwrap();
    w.set_prop(ground, "Size", Value::Vec2(Vec2::new(800.0, 40.0)))
        .unwrap();

    let template = w.create("Sprite", storage).unwrap();
    w.set_name(template, "BulletTemplate").unwrap();

    let json1 = w.to_json();
    let w2 = World::from_json(&json1).unwrap();
    let json2 = w2.to_json();
    assert_eq!(json1, json2);

    let ws2 = w2.workspace();
    let player2 = w2.find_first_child(ws2, "Player").unwrap();
    assert_eq!(
        w2.get_prop(player2, "Position"),
        Some(&Value::Vec2(Vec2::new(-40.0, 12.5)))
    );
    assert!(w2.find_first_child(player2, "Movement").is_some());

    let Some(Value::InstanceRef(Some(cam2))) = w2.get_prop(ws2, "CurrentCamera") else {
        panic!("CurrentCamera lost in roundtrip");
    };
    assert_eq!(w2.class_name(*cam2), Some("Camera2D"));
}

#[test]
fn gui_properties_roundtrip() {
    let mut w = World::new();
    let gui = w.gui().unwrap();
    let frame = w.create("Frame", gui).unwrap();
    w.set_name(frame, "Panel").unwrap();
    w.set_prop(
        frame,
        "Position",
        Value::UDim2(UDim2::new(0.5, -20.0, 0.25, 8.0)),
    )
    .unwrap();
    w.set_prop(
        frame,
        "Size",
        Value::UDim2(UDim2::new(0.0, 300.0, 1.0, -40.0)),
    )
    .unwrap();
    w.set_prop(frame, "AnchorPoint", Value::Vec2(Vec2::new(0.5, 0.5)))
        .unwrap();
    w.set_prop(frame, "BackgroundTransparency", Value::Number(0.25))
        .unwrap();
    w.set_prop(frame, "ClipsDescendants", Value::Bool(true))
        .unwrap();
    w.set_prop(frame, "Visible", Value::Bool(false)).unwrap();
    w.set_prop(frame, "ZIndex", Value::Number(7.0)).unwrap();

    let json = w.to_json();
    let w2 = World::from_json(&json).unwrap();
    // Stable across a second serialization.
    assert_eq!(json, w2.to_json());

    let gui2 = w2.gui().unwrap();
    let panel = w2.find_first_child(gui2, "Panel").unwrap();
    assert_eq!(
        w2.get_prop(panel, "Position"),
        Some(&Value::UDim2(UDim2::new(0.5, -20.0, 0.25, 8.0)))
    );
    assert_eq!(
        w2.get_prop(panel, "Size"),
        Some(&Value::UDim2(UDim2::new(0.0, 300.0, 1.0, -40.0)))
    );
    assert_eq!(
        w2.get_prop(panel, "AnchorPoint"),
        Some(&Value::Vec2(Vec2::new(0.5, 0.5)))
    );
    assert_eq!(
        w2.get_prop(panel, "BackgroundTransparency"),
        Some(&Value::Number(0.25))
    );
    assert_eq!(
        w2.get_prop(panel, "ClipsDescendants"),
        Some(&Value::Bool(true))
    );
    assert_eq!(w2.get_prop(panel, "Visible"), Some(&Value::Bool(false)));
}

#[test]
fn sprite_transform_props_roundtrip() {
    let mut w = World::new();
    let ws = w.workspace();
    let sprite = w.create("Sprite", ws).unwrap();
    w.set_name(sprite, "Hero").unwrap();
    w.set_prop(sprite, "Position", Value::Vec2(Vec2::new(12.0, -34.0)))
        .unwrap();
    w.set_prop(sprite, "Size", Value::Vec2(Vec2::new(80.0, 40.0)))
        .unwrap();
    w.set_prop(sprite, "Scale", Value::Vec2(Vec2::new(1.5, 2.0)))
        .unwrap();
    w.set_prop(sprite, "Rotation", Value::Number(37.5)).unwrap();
    w.set_prop(sprite, "Pivot", Value::Vec2(Vec2::new(0.0, 1.0)))
        .unwrap();
    w.set_prop(sprite, "ZIndex", Value::Number(4.0)).unwrap();
    w.set_prop(sprite, "Locked", Value::Bool(true)).unwrap();
    w.set_prop(sprite, "Visible", Value::Bool(false)).unwrap();

    let json = w.to_json();
    let w2 = World::from_json(&json).unwrap();
    assert_eq!(json, w2.to_json());

    let ws2 = w2.workspace();
    let h = w2.find_first_child(ws2, "Hero").unwrap();
    assert_eq!(w2.get_prop(h, "Rotation"), Some(&Value::Number(37.5)));
    assert_eq!(
        w2.get_prop(h, "Pivot"),
        Some(&Value::Vec2(Vec2::new(0.0, 1.0)))
    );
    assert_eq!(
        w2.get_prop(h, "Scale"),
        Some(&Value::Vec2(Vec2::new(1.5, 2.0)))
    );
    assert_eq!(w2.get_prop(h, "Locked"), Some(&Value::Bool(true)));
}

#[test]
fn sample_sprite_scene_loads() {
    // The hand-authored sample scene must stay loadable as the schema evolves.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../projects/sprite_demo/main.scene.json"
    );
    let json = std::fs::read_to_string(path).expect("read sample scene");
    let w = World::from_json(&json).expect("load sample scene");
    let ws = w.workspace();
    let spinner = w.find_first_child(ws, "Spinner").unwrap();
    assert_eq!(w.get_prop(spinner, "Rotation"), Some(&Value::Number(30.0)));
    let locked = w.find_first_child(ws, "LockedRock").unwrap();
    assert_eq!(w.get_prop(locked, "Locked"), Some(&Value::Bool(true)));
    // Oriented transform resolves for a rotated sprite.
    let xf = flux_core::SpriteXform::read(&w, spinner).unwrap();
    assert!((xf.rotation - 30.0).abs() < 1e-3);
}

#[test]
fn new_world_has_scripts_service() {
    let w = World::new();
    let scripts = w.scripts().expect("Scripts service should exist");
    assert_eq!(w.class_name(scripts), Some("Scripts"));
    // GetService-style lookup resolves it by class name.
    assert_eq!(w.service("Scripts"), Some(scripts));
}

#[test]
fn loading_legacy_scene_gains_scripts_service() {
    // A scene authored before the Scripts service existed (Workspace + Gui only)
    // should gain one on load, so top-level scripts have a home everywhere.
    let json = r#"{
        "version": 1,
        "root": {
            "class": "Game",
            "name": "Game",
            "children": [
                { "class": "Workspace", "name": "Workspace" },
                { "class": "Gui", "name": "Gui" }
            ]
        }
    }"#;
    let w = World::from_json(json).expect("load legacy scene");
    assert!(
        w.scripts().is_some(),
        "Scripts service should be auto-added on load"
    );
    assert!(
        w.service("Storage").is_some(),
        "Storage service should be auto-added on load"
    );
}

#[test]
fn animated_sprite_serializes_config_not_runtime_state() {
    let mut w = World::new();
    let ws = w.workspace();
    let s = w.create("AnimatedSprite", ws).unwrap();
    w.set_prop(s, "Animation", Value::String("Run".into()))
        .unwrap();
    // Runtime state as if playback advanced.
    w.set_prop(s, "CurrentFrame", Value::Number(5.0)).unwrap();
    w.set_prop(s, "TimePosition", Value::Number(0.683)).unwrap();
    w.set_prop(s, "Playing", Value::Bool(true)).unwrap();

    let json = w.to_json();
    assert!(
        json.contains("Animation") && json.contains("Run"),
        "authored Animation must serialize"
    );
    assert!(
        !json.contains("CurrentFrame"),
        "transient CurrentFrame must not serialize"
    );
    assert!(
        !json.contains("TimePosition"),
        "transient TimePosition must not serialize"
    );

    // Reload: authored Animation preserved, transient state back at defaults.
    let w2 = World::from_json(&json).unwrap();
    let s2 = w2
        .find_first_child(w2.workspace(), "AnimatedSprite")
        .unwrap();
    assert_eq!(
        w2.get_prop(s2, "Animation"),
        Some(&Value::String("Run".into()))
    );
    assert_eq!(w2.get_prop(s2, "CurrentFrame"), Some(&Value::Number(0.0)));
    assert!(matches!(
        w2.get_prop(s2, "Playing"),
        Some(Value::Bool(false))
    ));
}

#[test]
fn legacy_sprite_animation_player_migrates_to_animated_sprite() {
    // A hand-written legacy scene: a Sprite "Hero" with an AnimationPlayer child.
    let json = r#"{
        "version": 1,
        "root": { "class": "Game", "name": "Game", "children": [
            { "class": "Workspace", "name": "Workspace", "children": [
                { "class": "Sprite", "name": "Hero",
                  "props": { "Tint": { "t": "Color", "v": [1.0, 0.0, 0.0, 1.0] } },
                  "children": [
                    { "class": "AnimationPlayer", "name": "Anim", "props": {
                        "Frames": { "t": "Asset", "v": "hero.spriteframes" },
                        "AutoPlay": { "t": "String", "v": "Run" }
                    } }
                  ] }
            ] }
        ] }
    }"#;

    let w = World::from_json(json).expect("legacy scene should load");
    let ws = w.workspace();
    let hero = w
        .find_first_child(ws, "Hero")
        .expect("migrated node present");
    assert_eq!(w.class_name(hero), Some("AnimatedSprite"));
    assert_eq!(
        w.get_prop(hero, "Frames"),
        Some(&Value::Asset("hero.spriteframes".into()))
    );
    assert_eq!(
        w.get_prop(hero, "Animation"),
        Some(&Value::String("Run".into()))
    );
    assert!(matches!(
        w.get_prop(hero, "AutoPlay"),
        Some(Value::Bool(true))
    ));
    // Visual config carried over from the Sprite.
    assert_eq!(
        w.get_prop(hero, "Tint"),
        Some(&Value::Color(Color::new(1.0, 0.0, 0.0, 1.0)))
    );
    // The old Sprite and player are gone.
    assert!(w.find_first_child(ws, "Anim").is_none());
    assert_eq!(
        w.children(ws)
            .iter()
            .filter(|&&c| w.class_name(c) == Some("Sprite"))
            .count(),
        0,
        "legacy Sprite should be replaced"
    );
}
