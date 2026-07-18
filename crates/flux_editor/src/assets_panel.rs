use std::path::{Path, PathBuf};

use eframe::egui::{self, Ui};
use flux_core::{Value, World};
use flux_icons::{Icon, IconRole, Icons};
use flux_render::{AssetKind, classify};

use crate::app::{AssetDrag, Pending, UiState};
use crate::command::Command;
use crate::textures::TextureCache;

pub fn show(
    ui: &mut Ui,
    root: Option<&Path>,
    world: &World,
    state: &mut UiState,
    textures: &mut TextureCache,
    icons: &Icons,
) {
    let Some(root) = root else {
        ui.weak("Open or save a project to browse its files.");
        return;
    };
    let project_name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string());

    ui.horizontal(|ui| {
        icons.icon(Icon::Project).size(16.0).show(ui);
        if ui
            .selectable_label(state.asset_dir.as_os_str().is_empty(), &project_name)
            .clicked()
        {
            state.asset_dir.clear();
        }
        let comps: Vec<String> = state
            .asset_dir
            .iter()
            .map(|c| c.to_string_lossy().into_owned())
            .collect();
        let mut acc = PathBuf::new();
        for (i, comp) in comps.iter().enumerate() {
            ui.label("›");
            acc.push(comp);
            if ui
                .selectable_label(i + 1 == comps.len(), comp)
                .clicked()
            {
                state.asset_dir = acc.clone();
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons
                .icon(Icon::Refresh)
                .size(16.0)
                .button(ui)
                .on_hover_text("Reload textures from disk")
                .clicked()
            {
                textures.clear();
            }
            if icons
                .icon(Icon::Animation)
                .size(16.0)
                .button(ui)
                .on_hover_text("New animation library (.frames.json)")
                .clicked()
            {
                if let Ok(rel) = create_frames_library(root, &state.asset_dir) {
                    state.open_animation = Some(rel);
                }
            }
        });
    });
    ui.separator();

    let dir = root.join(&state.asset_dir);
    let mut entries: Vec<(String, bool)> = match std::fs::read_dir(&dir) {
        Ok(read) => read
            .flatten()
            .map(|e| {
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                (e.file_name().to_string_lossy().into_owned(), is_dir)
            })
            // Hide dotfiles / internal dirs like `.flux` (playtest data).
            .filter(|(name, _)| !name.starts_with('.'))
            .collect(),
        Err(_) => {
            ui.weak(format!("Cannot read {}", dir.display()));
            return;
        }
    };
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (name, is_dir) in entries {
                let kind = classify(&name, is_dir);
                let rel = join_rel(&state.asset_dir, &name);
                row(ui, world, state, textures, icons, root, &name, &rel, kind, is_dir);
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    world: &World,
    state: &mut UiState,
    textures: &mut TextureCache,
    icons: &Icons,
    root: &Path,
    name: &str,
    rel: &str,
    kind: AssetKind,
    is_dir: bool,
) {
    let icon = kind_icon(kind);
    if is_dir {
        let resp = ui.horizontal(|ui| {
            icons.icon(icon).size(16.0).role(IconRole::Accent).show(ui);
            ui.selectable_label(false, name)
        });
        if resp.inner.double_clicked() || resp.inner.clicked() {
            state.asset_dir.push(name);
        }
        return;
    }

    let is_script = matches!(kind, AssetKind::LuaScript | AssetKind::LuaModule | AssetKind::Script);
    let label = display_name(name, kind);

    // The whole row is one interactive strip, so the entire thing is a drag
    // source (into the scene/Explorer), double-click target, and right-click menu.
    let full_w = ui.available_width();
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(full_w, 20.0), egui::Sense::click_and_drag());
    if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            2.0,
            ui.visuals().widgets.hovered.bg_fill.gamma_multiply(0.45),
        );
    }

    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 4.0, rect.center().y - 9.0),
        egui::vec2(18.0, 18.0),
    );
    if kind == AssetKind::Image {
        if let Some(tex) = textures.get(ui.ctx(), root, rel) {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(tex.id(), icon_rect, uv, egui::Color32::WHITE);
        } else {
            icons.icon(icon).size(16.0).paint_at(ui, icon_rect);
        }
    } else {
        icons.icon(icon).size(16.0).role(IconRole::Muted).paint_at(ui, icon_rect);
    }
    ui.painter().text(
        egui::pos2(icon_rect.right() + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &label,
        egui::FontId::proportional(13.0),
        ui.visuals().text_color(),
    );

    resp.dnd_set_drag_payload(AssetDrag(rel.to_string()));

    if is_script && resp.double_clicked() {
        state.open_script = Some((rel.to_string(), None));
    }
    if kind == AssetKind::Animation && resp.double_clicked() {
        state.open_animation = Some(rel.to_string());
    }

    if kind == AssetKind::Image {
        resp.on_hover_text("Drag onto a sprite or into the Explorer").context_menu(|ui| {
            let sprite = state
                .selection
                .filter(|&id| world.class_name(id) == Some("Sprite"));
            if ui
                .add_enabled(sprite.is_some(), egui::Button::new("Set as Texture of selection"))
                .clicked()
            {
                if let Some(id) = sprite {
                    let old = world
                        .get_prop(id, "Texture")
                        .cloned()
                        .unwrap_or(Value::Asset(String::new()));
                    state.queue.push(Pending {
                        cmd: Command::set_prop(id, "Texture", old, Value::Asset(rel.to_string())),
                        merge: false,
                    });
                }
                ui.close();
            }
        });
    } else if kind == AssetKind::Animation {
        resp.on_hover_text("Double-click to open in the Animation Editor");
    } else if is_script {
        resp.on_hover_text("Double-click to open · drag into the Explorer");
    }
}

/// Starter content for a freshly created animation library.
const STARTER_FRAMES: &str = "{\n  \"texture\": \"\",\n  \"clips\": {\n    \"New\": { \"loop\": true, \"frames\": [] }\n  }\n}\n";

/// Create a uniquely-named `*.frames.json` in `dir` (relative to `root`) and
/// return its project-relative path.
fn create_frames_library(root: &Path, dir: &Path) -> std::io::Result<String> {
    let mut n = 0;
    let (name, full) = loop {
        let name = if n == 0 {
            "untitled.frames.json".to_string()
        } else {
            format!("untitled_{n}.frames.json")
        };
        let full = root.join(dir).join(&name);
        if !full.exists() {
            break (name, full);
        }
        n += 1;
    };
    std::fs::write(&full, STARTER_FRAMES)?;
    Ok(join_rel(dir, &name))
}

/// Build a project-root-relative path (forward-slashed) for `dir/name`, where
/// `dir` is the browser's current subfolder relative to the project root.
fn join_rel(dir: &Path, name: &str) -> String {
    let mut parts: Vec<String> = dir.iter().map(|c| c.to_string_lossy().into_owned()).collect();
    parts.push(name.to_string());
    parts.join("/")
}

/// Display name for a file: scripts and modules are shown without their
/// extension (`test.module.luau` -> `test`, `main.luau` -> `main`); everything
/// else keeps its full name. The icon already conveys the kind.
fn display_name(name: &str, kind: AssetKind) -> String {
    let lower = name.to_ascii_lowercase();
    let suffixes: &[&str] = match kind {
        AssetKind::LuaModule => &[".module.luau", ".module.lua"],
        AssetKind::LuaScript => &[".luau", ".lua"],
        _ => return name.to_string(),
    };
    for suffix in suffixes {
        if lower.ends_with(suffix) {
            return name[..name.len() - suffix.len()].to_string();
        }
    }
    name.to_string()
}

fn kind_icon(kind: AssetKind) -> Icon {
    match kind {
        AssetKind::Folder => Icon::Folder,
        AssetKind::Image => Icon::Texture,
        AssetKind::Audio => Icon::Audio,
        AssetKind::Model => Icon::Mesh,
        AssetKind::Script => Icon::Script,
        AssetKind::LuaScript => Icon::Script,
        AssetKind::LuaModule => Icon::LuaScript,
        AssetKind::RustModule => Icon::RustModule,
        AssetKind::Scene => Icon::Scene,
        AssetKind::Material => Icon::Material,
        AssetKind::Animation => Icon::Animation,
        AssetKind::Prefab => Icon::Prefab,
        AssetKind::Package => Icon::Package,
        AssetKind::Font => Icon::Font,
        AssetKind::Unknown => Icon::UnknownFile,
    }
}

#[cfg(test)]
mod tests {
    use super::display_name;
    use flux_render::AssetKind;

    #[test]
    fn strips_script_and_module_extensions() {
        assert_eq!(display_name("main.luau", AssetKind::LuaScript), "main");
        assert_eq!(display_name("main.lua", AssetKind::LuaScript), "main");
        assert_eq!(display_name("test.module.luau", AssetKind::LuaModule), "test");
        assert_eq!(display_name("Util.Module.LUAU", AssetKind::LuaModule), "Util");
        // Non-script files keep their full name.
        assert_eq!(display_name("hero.png", AssetKind::Image), "hero.png");
    }
}
