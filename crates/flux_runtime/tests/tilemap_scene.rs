//! End-to-end check that a scene containing a `Tilemap` loads through a runtime
//! session and its grid is generated on launch. Engine-only: no game plugin, so
//! the scene is inline and colour tiles need no assets. Game-specific Lua-API and
//! simulation tests live in the `flux_game` crate.

use flux_runtime::Session;

const SCENE: &str = r#"{
  "version": 1,
  "root": {
    "class": "Game",
    "name": "Game",
    "children": [
      {
        "class": "Workspace",
        "name": "Workspace",
        "children": [
          {
            "class": "Tilemap",
            "name": "World",
            "props": {
              "MapWidth":  { "t": "Number", "v": 32.0 },
              "MapHeight": { "t": "Number", "v": 24.0 },
              "Seed":      { "t": "Number", "v": 1337.0 }
            }
          }
        ]
      },
      { "class": "Storage", "name": "Storage" },
      { "class": "Gui", "name": "Gui" },
      { "class": "Scripts", "name": "Scripts" }
    ]
  }
}"#;

#[test]
fn tilemap_scene_generates_a_grid_on_launch() {
    // Launching a session runs `tilemap::sync`, so the grid exists immediately.
    let session = Session::from_scene_json(SCENE, std::path::Path::new(".")).expect("scene loads");
    let world = session.world();
    let w = world.borrow();

    let tilemap = w
        .descendants(w.workspace())
        .into_iter()
        .find(|&id| w.class_name(id) == Some("Tilemap"))
        .expect("scene has a Tilemap");

    let grid = w.tile_grid(tilemap).expect("grid generated on launch");
    assert_eq!(grid.width(), 32);
    assert_eq!(grid.height(), 24);
    for row in 0..grid.height() as i32 {
        for col in 0..grid.width() as i32 {
            assert!(grid.get(col, row).is_some());
        }
    }
}
