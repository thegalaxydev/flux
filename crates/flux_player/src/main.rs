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
    shot: Option<Shot>,
}

/// `--screenshot` dev harness: run a fixed number of frames, capture the
/// window, write a PNG, exit. Lets tooling (and agents) see the composed game.
struct Shot {
    out: PathBuf,
    frames_left: u32,
    requested: bool,
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
        if let Some(shot) = &mut self.shot {
            if shot.frames_left > 0 {
                shot.frames_left -= 1;
            } else if !shot.requested {
                shot.requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            } else {
                let image = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(img) = image {
                    match save_screenshot(&img, &shot.out) {
                        Ok(()) => eprintln!("screenshot written to {}", shot.out.display()),
                        Err(e) => eprintln!("screenshot failed: {e}"),
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        ctx.request_repaint();
    }
}

fn save_screenshot(img: &egui::ColorImage, out: &Path) -> Result<(), String> {
    let (w, h) = (img.size[0] as u32, img.size[1] as u32);
    let mut png = image::RgbaImage::new(w, h);
    for (i, px) in img.pixels.iter().enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        png.put_pixel(x, y, image::Rgba(px.to_array()));
    }
    png.save(out).map_err(|e| e.to_string())
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
    let mut scene = None;
    let mut shot_out: Option<PathBuf> = None;
    let mut shot_frames = 90u32;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screenshot" => shot_out = args.next().map(PathBuf::from),
            "--frames" => shot_frames = args.next().and_then(|v| v.parse().ok()).unwrap_or(90),
            _ => scene = Some(arg),
        }
    }
    let scene = scene.unwrap_or_else(|| "projects/demo/main.scene.json".to_string());
    let path = PathBuf::from(&scene);
    let root = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    // Load the project's plugins (project.json) BEFORE any world exists —
    // the class registry locks to builtins on first touch otherwise.
    match flux_plugin::ensure_project(&root) {
        flux_plugin::Ensure::Ready(loaded) => {
            if !loaded.is_empty() {
                eprintln!("[Info] plugins loaded: {}", loaded.join(", "));
            }
        }
        flux_plugin::Ensure::NeedsRestart(missing) => {
            eprintln!("Flux Player: plugins required before startup: {}", missing.join(", "));
            std::process::exit(1);
        }
        flux_plugin::Ensure::Error(e) => {
            eprintln!("Flux Player: plugin load failed: {e}");
            std::process::exit(1);
        }
    }

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
                shot: shot_out.map(|out| Shot { out, frames_left: shot_frames, requested: false }),
            }))
        }),
    )
}
