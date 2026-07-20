#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use eframe::egui;
use flux_runtime::{DataBackend, InputFrame, Session, SessionOptions};
use flux_view::{AnimationCache, TextureCache, TileSetCache, draw_scene, game_camera};

struct Player {
    session: Session,
    textures: TextureCache,
    anim: AnimationCache,
    tiles: TileSetCache,
    root: PathBuf,
}

impl Player {
    /// Switch to another scene (from `Scene:Load`), loaded from `rel` under the
    /// project root.
    fn load_scene(&mut self, rel: &str) {
        let json = match std::fs::read_to_string(self.root.join(rel)) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("Scene:Load '{rel}': {e}");
                return;
            }
        };
        let options = SessionOptions {
            data: DataBackend::SqliteFile(self.root.join(".flux/data/playtest.sqlite")),
            scene: rel.to_string(),
        };
        match Session::launch(&json, &self.root, options) {
            Ok(session) => self.session = session,
            Err(e) => eprintln!("Scene:Load '{rel}': {e}"),
        }
    }
}

impl eframe::App for Player {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_gray(16)))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let dt = (ctx.input(|i| i.stable_dt) as f64).min(0.1);
                let input = collect_input(ctx, rect);
                self.session.step(dt, &input);

                let world = self.session.world();
                let w = world.borrow();
                let camera = game_camera(&w).unwrap_or_default();
                draw_scene(
                    ui.painter(),
                    ctx,
                    &w,
                    &mut self.textures,
                    &mut self.anim,
                    &mut self.tiles,
                    rect,
                    camera,
                    Some(&self.root),
                    None,
                    None,
                    true,
                );
            });
        for entry in self.session.drain_logs() {
            eprintln!("[{:?}] {}", entry.level, entry.message);
        }
        if let Some(rel) = self.session.take_scene_request() {
            self.load_scene(&rel);
        }
        ctx.request_repaint();
    }
}

fn collect_input(ctx: &egui::Context, rect: egui::Rect) -> InputFrame {
    ctx.input(|i| {
        let keys: HashSet<String> = i.keys_down.iter().map(|k| format!("{k:?}")).collect();
        let pos = i.pointer.latest_pos();
        let over = pos.is_some_and(|p| rect.contains(p));
        let mut mouse_buttons = HashSet::new();
        if over {
            if i.pointer.primary_down() {
                mouse_buttons.insert("Left".to_string());
            }
            if i.pointer.secondary_down() {
                mouse_buttons.insert("Right".to_string());
            }
            if i.pointer.middle_down() {
                mouse_buttons.insert("Middle".to_string());
            }
        }
        let rel = pos.map(|p| p - rect.min).unwrap_or_default();
        InputFrame {
            keys,
            mouse_pos: glam::Vec2::new(rel.x, rel.y),
            mouse_buttons,
            viewport: glam::Vec2::new(rect.width(), rect.height()),
            scroll: if over { i.raw_scroll_delta.y } else { 0.0 },
            pointer_over: over,
        }
    })
}

fn main() -> eframe::Result {
    let scene = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "projects/demo/main.scene.json".to_string());
    let path = PathBuf::from(&scene);
    let root = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let json = match std::fs::read_to_string(&path) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("Flux Player: cannot read scene '{}': {e}", path.display());
            std::process::exit(1);
        }
    };
    let scene_rel = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let options = SessionOptions {
        data: DataBackend::SqliteFile(root.join(".flux/data/playtest.sqlite")),
        scene: scene_rel,
    };
    let session = match Session::launch(&json, &root, options) {
        Ok(session) => session,
        Err(e) => {
            eprintln!("Flux Player: failed to load scene: {e}");
            std::process::exit(1);
        }
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([960.0, 600.0])
        .with_title("Flux Player");
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../../../logo/flux.png")) {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Flux Player",
        options,
        Box::new(|_cc| {
            Ok(Box::new(Player {
                session,
                textures: TextureCache::default(),
                anim: Default::default(),
                tiles: TileSetCache::default(),
                root,
            }))
        }),
    )
}
