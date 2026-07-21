//! The generator is deterministic (same bytes every run) and its emitted
//! `*.frames.json` parse with the engine's animation loader, with every clip
//! the game's state machine needs present.

use std::fs;
use std::path::PathBuf;

use flux_core::animation::SpriteFrames;

fn out_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("flux_assetgen_test_{tag}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn generation_is_deterministic() {
    let (a, b) = (out_dir("a"), out_dir("b"));
    flux_assetgen::generate_all(&a).unwrap();
    flux_assetgen::generate_all(&b).unwrap();
    for file in ["art/terrain.png", "art/reactor.png", "art/miner.frames.json", "world.tileset.json"] {
        let (fa, fb) = (fs::read(a.join(file)).unwrap(), fs::read(b.join(file)).unwrap());
        assert_eq!(fa, fb, "{file} differs between runs");
    }
    let _ = fs::remove_dir_all(&a);
    let _ = fs::remove_dir_all(&b);
}

#[test]
fn frames_json_parses_with_required_clips() {
    let dir = out_dir("clips");
    let summary = flux_assetgen::generate_all(&dir).unwrap();
    assert!(!summary.buildings.is_empty());

    for meta in &summary.buildings {
        let json = fs::read_to_string(dir.join(&meta.frames_asset)).unwrap();
        let frames = SpriteFrames::parse(&json).unwrap_or_else(|e| panic!("{}: {e}", meta.id));
        // Every building must cover the factory states...
        for clip in ["idle", "working", "starved"] {
            assert!(frames.clip(clip).is_some(), "{} missing clip '{clip}'", meta.id);
        }
        assert!(frames.default_texture().is_some(), "{} missing texture", meta.id);
    }
    // ...and the reactor additionally its own state machine.
    let json = fs::read_to_string(dir.join("art/reactor.frames.json")).unwrap();
    let frames = SpriteFrames::parse(&json).unwrap();
    for clip in ["off", "running", "hot", "meltdown"] {
        assert!(frames.clip(clip).is_some(), "reactor missing clip '{clip}'");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn tileset_json_keeps_ids_and_gains_rects() {
    let dir = out_dir("tiles");
    flux_assetgen::generate_all(&dir).unwrap();
    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("world.tileset.json")).unwrap()).unwrap();
    assert_eq!(doc["texture"], "art/terrain.png");
    let tiles = doc["tiles"].as_array().unwrap();
    // Stable ids/order: the worldgen config + saved grids index into this list.
    let ids: Vec<&str> = tiles.iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        ["water", "sand", "grass", "forest", "rock", "mountain", "coal", "iron", "copper", "uranium", "rare"]
    );
    for t in tiles {
        assert_eq!(t["rect"].as_array().unwrap().len(), 4);
    }
    let _ = fs::remove_dir_all(&dir);
}
