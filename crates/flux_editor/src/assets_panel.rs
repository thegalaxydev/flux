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
                .icon(Icon::Folder)
                .size(16.0)
                .button(ui)
                .on_hover_text("New folder")
                .clicked()
            {
                if let Err(e) = new_folder(root, &state.asset_dir) {
                    state.status = format!("New folder failed: {e}");
                }
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

    asset_dialogs(ui, state, root);
}

/// Rename and delete confirmation dialogs for the Assets panel.
fn asset_dialogs(ui: &mut Ui, state: &mut UiState, root: &Path) {
    // Rename.
    let mut apply_rename: Option<(String, String)> = None;
    let mut close_rename = false;
    if let Some((rel, buf)) = state.asset_rename.as_mut() {
        egui::Window::new("Rename asset")
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                let resp = ui.text_edit_singleline(buf);
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if (ui.button("Rename").clicked() || enter) && !buf.trim().is_empty() {
                        apply_rename = Some((rel.clone(), buf.trim().to_string()));
                        close_rename = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close_rename = true;
                    }
                });
            });
    }
    if let Some((rel, new_name)) = apply_rename {
        if let Err(e) = rename_asset(root, &rel, &new_name) {
            state.status = format!("Rename failed: {e}");
        }
    }
    if close_rename {
        state.asset_rename = None;
    }

    // Delete confirmation.
    let mut apply_delete: Option<String> = None;
    let mut close_delete = false;
    if let Some(rel) = state.asset_delete.clone() {
        let name = rel.rsplit(['/', '\\']).next().unwrap_or(&rel).to_string();
        egui::Window::new("Delete asset")
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.label(format!("Delete “{name}”? This can't be undone."));
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        apply_delete = Some(rel.clone());
                        close_delete = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close_delete = true;
                    }
                });
            });
    }
    if let Some(rel) = apply_delete {
        if let Err(e) = delete_asset(root, &rel) {
            state.status = format!("Delete failed: {e}");
        }
    }
    if close_delete {
        state.asset_delete = None;
    }
}

/// Create a uniquely-named "New Folder" in `dir` (relative to `root`).
fn new_folder(root: &Path, dir: &Path) -> std::io::Result<()> {
    let base = root.join(dir);
    let mut n = 0;
    loop {
        let name = if n == 0 {
            "New Folder".to_string()
        } else {
            format!("New Folder {n}")
        };
        let full = base.join(&name);
        if !full.exists() {
            return std::fs::create_dir(&full);
        }
        n += 1;
    }
}

/// Copy `rel` to a uniquely-named `<stem> copy.<ext>` next to it.
fn duplicate_file(root: &Path, rel: &str) -> std::io::Result<()> {
    let src = root.join(rel);
    let (dir, file) = rel.rsplit_once('/').unwrap_or(("", rel));
    let (stem, ext) = match file.rsplit_once('.') {
        Some((s, e)) => (s, format!(".{e}")),
        None => (file, String::new()),
    };
    let base = root.join(dir);
    let mut n = 0;
    loop {
        let cand = if n == 0 {
            format!("{stem} copy{ext}")
        } else {
            format!("{stem} copy {n}{ext}")
        };
        let dest = base.join(&cand);
        if !dest.exists() {
            std::fs::copy(&src, &dest)?;
            return Ok(());
        }
        n += 1;
    }
}

/// Rename `rel` within its own folder to `new_name` (path separators stripped).
fn rename_asset(root: &Path, rel: &str, new_name: &str) -> std::io::Result<()> {
    let new_name = new_name.rsplit(['/', '\\']).next().unwrap_or(new_name);
    let src = root.join(rel);
    let dir = src.parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
    let dest = dir.join(new_name);
    if dest.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "a file with that name already exists",
        ));
    }
    std::fs::rename(&src, &dest)
}

/// Delete `rel` (a file, or a folder and its contents).
fn delete_asset(root: &Path, rel: &str) -> std::io::Result<()> {
    let full = root.join(rel);
    if full.is_dir() {
        std::fs::remove_dir_all(&full)
    } else {
        std::fs::remove_file(&full)
    }
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
        let resp = ui
            .horizontal(|ui| {
                icons.icon(icon).size(16.0).role(IconRole::Accent).show(ui);
                ui.selectable_label(false, name)
            })
            .inner;
        if resp.double_clicked() || resp.clicked() {
            state.asset_dir.push(name);
        }
        resp.context_menu(|ui| entry_menu(ui, state, root, rel, name, true));
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

    let hover = match kind {
        AssetKind::Image => "Drag onto a sprite/Explorer · right-click for options",
        AssetKind::Animation => "Double-click to open · right-click for options",
        _ if is_script => "Double-click to open · right-click for options",
        _ => "Right-click for options",
    };
    resp.on_hover_text(hover).context_menu(|ui| {
        entry_menu(ui, state, root, rel, name, false);
        // Image-specific actions.
        if kind == AssetKind::Image {
            ui.separator();
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
            if ui.button("Create Sprite Frames").clicked() {
                match create_sprite_frames(root, rel) {
                    Ok(lib) => state.open_animation = Some(lib),
                    Err(e) => state.status = format!("Create failed: {e}"),
                }
                ui.close();
            }
        }
    });
}

/// Shared file-management actions for an asset row (rename / duplicate /
/// delete / new folder). Filesystem ops happen immediately; rename and delete
/// open a confirmation dialog rendered by [`show`].
fn entry_menu(ui: &mut Ui, state: &mut UiState, root: &Path, rel: &str, name: &str, is_dir: bool) {
    if ui.button("Rename…").clicked() {
        state.asset_rename = Some((rel.to_string(), name.to_string()));
        ui.close();
    }
    if !is_dir && ui.button("Duplicate").clicked() {
        if let Err(e) = duplicate_file(root, rel) {
            state.status = format!("Duplicate failed: {e}");
        }
        ui.close();
    }
    if ui.button("Delete…").clicked() {
        state.asset_delete = Some(rel.to_string());
        ui.close();
    }
    ui.separator();
    if ui.button("New Folder").clicked() {
        if let Err(e) = new_folder(root, &state.asset_dir) {
            state.status = format!("New folder failed: {e}");
        }
        ui.close();
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

/// Create a `.spriteframes` library next to `texture_rel` with that texture
/// pre-linked and one empty clip, returning its project-relative path. This is
/// the "Create Sprite Frames" action: the texture is the source of truth, so
/// the user never links it separately.
fn create_sprite_frames(root: &Path, texture_rel: &str) -> std::io::Result<String> {
    let (dir, file) = match texture_rel.rsplit_once('/') {
        Some((d, f)) => (format!("{d}/"), f),
        None => (String::new(), texture_rel),
    };
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
    let (rel, full) = {
        let mut n = 0;
        loop {
            let rel = if n == 0 {
                format!("{dir}{stem}.spriteframes")
            } else {
                format!("{dir}{stem}_{n}.spriteframes")
            };
            let full = root.join(&rel);
            if !full.exists() {
                break (rel, full);
            }
            n += 1;
        }
    };
    let content = format!(
        "{{\n  \"texture\": \"{texture_rel}\",\n  \"clips\": {{\n    \"New\": {{ \"loop\": true, \"frames\": [] }}\n  }}\n}}\n"
    );
    std::fs::write(&full, content)?;
    Ok(rel)
}

/// Build a project-root-relative path (forward-slashed) for `dir/name`, where
/// `dir` is the browser's current subfolder relative to the project root.
fn join_rel(dir: &Path, name: &str) -> String {
    let mut parts: Vec<String> = dir.iter().map(|c| c.to_string_lossy().into_owned()).collect();
    parts.push(name.to_string());
    parts.join("/")
}

/// Display name for a file: recognized assets drop their extension since the
/// icon already conveys the kind (`hero.spriteframes` -> `hero`,
/// `main.scene.json` -> `main`, `hero_sheet.png` -> `hero_sheet`). Compound
/// extensions strip whole; otherwise a single trailing extension is removed.
/// Folders and unrecognized files keep their full name.
fn display_name(name: &str, kind: AssetKind) -> String {
    if matches!(kind, AssetKind::Folder | AssetKind::Unknown) {
        return name.to_string();
    }
    // Multi-part extensions the single-dot rule below wouldn't fully strip.
    const COMPOUND: &[&str] = &[".module.luau", ".module.lua", ".scene.json", ".frames.json"];
    let lower = name.to_ascii_lowercase();
    for suffix in COMPOUND {
        if lower.ends_with(suffix) {
            return name[..name.len() - suffix.len()].to_string();
        }
    }
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => name.to_string(),
    }
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
        AssetKind::TileSet => Icon::Material,
        AssetKind::WorldGen => Icon::Material,
        AssetKind::BuildingCatalog => Icon::Material,
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
    fn strips_recognized_extensions_for_display() {
        // Scripts and modules (compound extensions).
        assert_eq!(display_name("main.luau", AssetKind::LuaScript), "main");
        assert_eq!(display_name("main.lua", AssetKind::LuaScript), "main");
        assert_eq!(display_name("test.module.luau", AssetKind::LuaModule), "test");
        assert_eq!(display_name("Util.Module.LUAU", AssetKind::LuaModule), "Util");
        // Other recognized assets drop their extension too.
        assert_eq!(display_name("hero.png", AssetKind::Image), "hero");
        assert_eq!(display_name("hero_sheet.png", AssetKind::Image), "hero_sheet");
        assert_eq!(display_name("hero.spriteframes", AssetKind::Animation), "hero");
        assert_eq!(display_name("hero.frames.json", AssetKind::Animation), "hero");
        assert_eq!(display_name("main.scene.json", AssetKind::Scene), "main");
        // A dotted stem only loses the final extension.
        assert_eq!(display_name("my.cool.tex.png", AssetKind::Image), "my.cool.tex");
        // Folders and unknown files keep their full name.
        assert_eq!(display_name("notes.txt", AssetKind::Unknown), "notes.txt");
        assert_eq!(display_name("sprites", AssetKind::Folder), "sprites");
    }

    #[test]
    fn file_ops_duplicate_rename_new_folder_delete() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("flux_ops_{nanos}"));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/a.txt"), "x").unwrap();

        super::duplicate_file(&root, "sub/a.txt").unwrap();
        assert!(root.join("sub/a copy.txt").exists());

        super::new_folder(&root, std::path::Path::new("sub")).unwrap();
        assert!(root.join("sub/New Folder").is_dir());

        super::rename_asset(&root, "sub/a.txt", "b.txt").unwrap();
        assert!(!root.join("sub/a.txt").exists());
        assert!(root.join("sub/b.txt").exists());
        // A name collision is refused.
        assert!(super::rename_asset(&root, "sub/b.txt", "a copy.txt").is_err());

        super::delete_asset(&root, "sub/b.txt").unwrap();
        assert!(!root.join("sub/b.txt").exists());
        super::delete_asset(&root, "sub").unwrap(); // recursive folder delete
        assert!(!root.join("sub").exists());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn create_sprite_frames_links_texture_and_stays_unique() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("flux_csf_{nanos}"));
        std::fs::create_dir_all(root.join("sprites")).unwrap();

        let rel = super::create_sprite_frames(&root, "sprites/hero.png").unwrap();
        assert_eq!(rel, "sprites/hero.spriteframes");
        let content = std::fs::read_to_string(root.join(&rel)).unwrap();
        assert!(content.contains("\"texture\": \"sprites/hero.png\""), "texture linked");
        assert!(content.contains("\"New\""), "starter clip present");

        // A second call must not clobber the first.
        let rel2 = super::create_sprite_frames(&root, "sprites/hero.png").unwrap();
        assert_eq!(rel2, "sprites/hero_1.spriteframes");

        std::fs::remove_dir_all(&root).ok();
    }
}
