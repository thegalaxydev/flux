#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod assets_panel;
mod command;
mod explorer;
mod properties;
mod script_editor;
mod textures;
mod viewport;

use std::path::{Path, PathBuf};

use eframe::egui;
use flux_core::{Color, Value, World};
use flux_render::LoadedImage;
use glam::Vec2;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--write-demo") {
        let path = args
            .get(i + 1)
            .map(String::as_str)
            .unwrap_or("projects/demo/main.scene.json");
        let path = Path::new(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create demo dir");
            write_demo_assets(parent);
        }
        std::fs::write(path, demo_world().to_json()).expect("write demo scene");
        println!("wrote {}", path.display());
        return Ok(());
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 720.0])
        .with_title("Flux Editor");
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../../../logo/flux.png")) {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let (world, path) = load_startup();
    eframe::run_native(
        "Flux",
        options,
        Box::new(|_cc| Ok(Box::new(app::EditorApp::new(world, path)))),
    )
}

fn load_startup() -> (World, Option<PathBuf>) {
    let path = PathBuf::from("projects/demo/main.scene.json");
    if let Ok(json) = std::fs::read_to_string(&path) {
        if let Ok(world) = World::from_json(&json) {
            return (world, Some(path));
        }
    }
    (demo_world(), None)
}

fn demo_world() -> World {
    let mut w = World::new();
    let ws = w.workspace();
    let storage = w.service("Storage").unwrap();

    let player = w.create("Sprite", ws).unwrap();
    w.set_name(player, "Player").unwrap();
    w.set_prop(player, "Size", Value::Vec2(Vec2::new(48.0, 48.0))).unwrap();
    w.set_prop(player, "Position", Value::Vec2(Vec2::new(0.0, -60.0))).unwrap();
    w.set_prop(player, "Texture", Value::Asset("assets/sprites/hero.png".into())).unwrap();
    w.set_prop(player, "ZIndex", Value::Number(1.0)).unwrap();

    let movement = w.create("Script", player).unwrap();
    w.set_name(movement, "Movement").unwrap();
    w.set_prop(movement, "SourcePath", Value::Asset("scripts/movement.luau".into())).unwrap();

    let env = w.create("Folder", ws).unwrap();
    w.set_name(env, "Environment").unwrap();

    let ground = w.create("Sprite", env).unwrap();
    w.set_name(ground, "Ground").unwrap();
    w.set_prop(ground, "Size", Value::Vec2(Vec2::new(720.0, 48.0))).unwrap();
    w.set_prop(ground, "Position", Value::Vec2(Vec2::new(0.0, 120.0))).unwrap();
    w.set_prop(ground, "Texture", Value::Asset("assets/sprites/ground.png".into())).unwrap();

    let crate1 = w.create("Sprite", env).unwrap();
    w.set_name(crate1, "Crate").unwrap();
    w.set_prop(crate1, "Size", Value::Vec2(Vec2::new(56.0, 56.0))).unwrap();
    w.set_prop(crate1, "Position", Value::Vec2(Vec2::new(150.0, 68.0))).unwrap();
    w.set_prop(crate1, "Texture", Value::Asset("assets/sprites/crate.png".into())).unwrap();

    let crate2 = w.create("Sprite", env).unwrap();
    w.set_name(crate2, "Crate").unwrap();
    w.set_prop(crate2, "Size", Value::Vec2(Vec2::new(56.0, 56.0))).unwrap();
    w.set_prop(crate2, "Position", Value::Vec2(Vec2::new(210.0, 68.0))).unwrap();
    w.set_prop(crate2, "Texture", Value::Asset("assets/sprites/crate.png".into())).unwrap();

    let template = w.create("Sprite", storage).unwrap();
    w.set_name(template, "BulletTemplate").unwrap();
    w.set_prop(template, "Size", Value::Vec2(Vec2::new(12.0, 4.0))).unwrap();

    let gui = w.gui().unwrap();
    let hud = w.create("Frame", gui).unwrap();
    w.set_name(hud, "HUD").unwrap();
    w.set_prop(hud, "Position", Value::Vec2(Vec2::new(12.0, 12.0))).unwrap();
    w.set_prop(hud, "Size", Value::Vec2(Vec2::new(240.0, 72.0))).unwrap();
    w.set_prop(hud, "BackgroundColor", Value::Color(Color::new(0.07, 0.08, 0.11, 0.85))).unwrap();

    let title = w.create("Label", gui).unwrap();
    w.set_name(title, "Title").unwrap();
    w.set_prop(title, "Position", Value::Vec2(Vec2::new(24.0, 20.0))).unwrap();
    w.set_prop(title, "Size", Value::Vec2(Vec2::new(216.0, 20.0))).unwrap();
    w.set_prop(title, "Text", Value::String("Flux — click the button".into())).unwrap();
    w.set_prop(title, "TextSize", Value::Number(15.0)).unwrap();
    w.set_prop(title, "BackgroundColor", Value::Color(Color::new(0.0, 0.0, 0.0, 0.0))).unwrap();
    w.set_prop(title, "ZIndex", Value::Number(1.0)).unwrap();

    let button = w.create("Button", gui).unwrap();
    w.set_name(button, "Btn").unwrap();
    w.set_prop(button, "Position", Value::Vec2(Vec2::new(24.0, 46.0))).unwrap();
    w.set_prop(button, "Size", Value::Vec2(Vec2::new(150.0, 26.0))).unwrap();
    w.set_prop(button, "Text", Value::String("Click me".into())).unwrap();
    w.set_prop(button, "BackgroundColor", Value::Color(Color::new(0.20, 0.45, 0.80, 1.0))).unwrap();
    w.set_prop(button, "ZIndex", Value::Number(1.0)).unwrap();

    let ui_script = w.create("Script", button).unwrap();
    w.set_name(ui_script, "UI").unwrap();
    w.set_prop(ui_script, "SourcePath", Value::Asset("scripts/ui.luau".into())).unwrap();

    w
}

fn write_demo_assets(root: &Path) {
    let dir = root.join("assets/sprites");
    std::fs::create_dir_all(&dir).expect("create sprites dir");
    for (name, image) in [
        ("hero.png", hero_texture()),
        ("crate.png", crate_texture()),
        ("ground.png", ground_texture()),
    ] {
        let png = flux_render::encode_png(&image).expect("encode png");
        std::fs::write(dir.join(name), png).expect("write texture");
    }
}

fn build_image(w: u32, h: u32, mut f: impl FnMut(u32, u32) -> [u8; 4]) -> LoadedImage {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&f(x, y));
        }
    }
    LoadedImage::new(w, h, rgba)
}

fn hero_texture() -> LoadedImage {
    let (w, h) = (32, 32);
    build_image(w, h, |x, y| {
        let edge = x < 2 || y < 2 || x >= w - 2 || y >= h - 2;
        if edge {
            return [28, 66, 132, 255];
        }
        let shade = 1.0 - (x + y) as f32 / (w + h) as f32 * 0.45;
        let eye = (y > 9 && y < 15) && ((x > 8 && x < 13) || (x > 18 && x < 23));
        if eye {
            return [245, 245, 255, 255];
        }
        [
            (70.0 * shade) as u8,
            (150.0 * shade) as u8,
            (235.0 * shade) as u8,
            255,
        ]
    })
}

fn crate_texture() -> LoadedImage {
    let (w, h) = (32, 32);
    build_image(w, h, |x, y| {
        let edge = x < 2 || y < 2 || x >= w - 2 || y >= h - 2;
        let plank = x == 15 || x == 16 || y == 15 || y == 16;
        let bolt = (x < 6 || x >= w - 6) && (y < 6 || y >= h - 6) && x % 2 == y % 2;
        if edge || bolt {
            return [92, 60, 30, 255];
        }
        if plank {
            return [120, 84, 44, 255];
        }
        [158, 116, 62, 255]
    })
}

fn ground_texture() -> LoadedImage {
    let (w, h) = (64, 16);
    build_image(w, h, |x, y| {
        let noise = ((x * 7 + y * 13) % 5) as i32 - 2;
        if y < 4 {
            [
                (95 + noise) as u8,
                (165 + noise * 2) as u8,
                (80 + noise) as u8,
                255,
            ]
        } else {
            [
                (70 + noise) as u8,
                (120 + noise) as u8,
                (58 + noise) as u8,
                255,
            ]
        }
    })
}
