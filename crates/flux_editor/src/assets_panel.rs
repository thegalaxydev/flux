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

    let is_script = matches!(kind, AssetKind::LuaScript | AssetKind::Script);

    // The label senses click+drag so the row can be both a drag source (into the
    // scene/explorer) and double-clicked to open scripts.
    let resp = ui
        .horizontal(|ui| {
            if kind == AssetKind::Image {
                if let Some(tex) = textures.get(ui.ctx(), root, rel) {
                    let sized = egui::load::SizedTexture::new(tex.id(), egui::vec2(18.0, 18.0));
                    ui.add(egui::Image::new(sized));
                } else {
                    icons.icon(icon).size(16.0).show(ui);
                }
            } else {
                icons.icon(icon).size(16.0).role(IconRole::Muted).show(ui);
            }
            ui.add(egui::Label::new(name).sense(egui::Sense::click_and_drag()))
        })
        .inner;

    resp.dnd_set_drag_payload(AssetDrag(rel.to_string()));

    if is_script && resp.double_clicked() {
        state.open_script = Some((rel.to_string(), None));
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
    } else if is_script {
        resp.on_hover_text("Double-click to open · drag into the Explorer");
    }
}

/// Build a project-root-relative path (forward-slashed) for `dir/name`, where
/// `dir` is the browser's current subfolder relative to the project root.
fn join_rel(dir: &Path, name: &str) -> String {
    let mut parts: Vec<String> = dir.iter().map(|c| c.to_string_lossy().into_owned()).collect();
    parts.push(name.to_string());
    parts.join("/")
}

fn kind_icon(kind: AssetKind) -> Icon {
    match kind {
        AssetKind::Folder => Icon::Folder,
        AssetKind::Image => Icon::Texture,
        AssetKind::Audio => Icon::Audio,
        AssetKind::Model => Icon::Mesh,
        AssetKind::Script => Icon::Script,
        AssetKind::LuaScript => Icon::LuaScript,
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
