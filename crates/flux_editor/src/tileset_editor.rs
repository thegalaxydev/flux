//! The tileset editor: a floating window for authoring `*.tileset.json` tile
//! palettes. It edits a [`TileSetDoc`] in place — library texture + footprint
//! size, an ordered tile list (id, colour, optional atlas rect with a live
//! thumbnail), and a diamond palette preview — then saves the JSON back to the
//! project. Modelled on [`crate::animation_editor::AnimationEditor`].

use std::path::Path;

use eframe::egui::{self, Color32, Sense, Ui, vec2};
use flux_core::animation::AnimationCache;
use flux_core::tilemap::{TileDoc, TileSetDoc};
use flux_icons::{Icon, IconRole, Icons};

use crate::animation_editor::{fit_rect, paint_frame};
use crate::textures::TextureCache;

pub struct TileSetEditor {
    pub open: bool,
    /// Project-relative path of the open tileset.
    rel: String,
    doc: TileSetDoc,
    /// JSON of the last saved state, for the dirty check.
    saved: String,
    selected: Option<usize>,
    status: String,
    /// Set for one frame after a successful save, so the app can drop caches and
    /// regenerate any live `Tilemap` using this palette.
    just_saved: bool,
    /// Loads referenced sprite-frames libraries just to list their clip names in
    /// the animated-tile picker.
    anim_cache: AnimationCache,
}

impl Default for TileSetEditor {
    fn default() -> Self {
        Self {
            open: false,
            rel: String::new(),
            // `{}` yields an empty palette with the schema's default sizes.
            doc: TileSetDoc::from_json("{}").unwrap(),
            saved: String::new(),
            selected: None,
            status: String::new(),
            just_saved: false,
            anim_cache: AnimationCache::default(),
        }
    }
}

impl TileSetEditor {
    /// Load `json` (a `*.tileset.json`) for editing. Invalid JSON opens an empty
    /// document so the user can start over rather than being blocked.
    pub fn open_doc(&mut self, rel: &str, json: &str) {
        let doc = TileSetDoc::from_json(json).unwrap_or_else(|_| default_doc());
        self.selected = (!doc.tiles.is_empty()).then_some(0);
        self.rel = rel.to_string();
        // Baseline the dirty check against normalized JSON so a freshly opened
        // file (whose on-disk formatting may differ) doesn't read dirty.
        self.saved = doc.to_json();
        self.doc = doc;
        self.status.clear();
        self.open = true;
    }

    pub fn dirty(&self) -> bool {
        self.doc.to_json() != self.saved
    }

    /// Project-relative path of the open tileset (for the tab label).
    pub fn rel(&self) -> &str {
        &self.rel
    }

    /// Whether a save happened since the last call (consumes the flag).
    pub fn take_saved(&mut self) -> bool {
        std::mem::take(&mut self.just_saved)
    }

    pub(crate) fn save(&mut self, root: &Path) {
        let json = self.doc.to_json();
        match std::fs::write(root.join(&self.rel), &json) {
            Ok(()) => {
                self.saved = json;
                self.status = "Saved".to_string();
                self.just_saved = true;
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    /// Render into the central panel as a docked tab.
    pub fn dock_ui(&mut self, ui: &mut Ui, textures: &mut TextureCache, root: &Path, icons: &Icons) {
        // ---- header ----
        ui.horizontal(|ui| {
            if icons.icon(Icon::Save).size(16.0).button(ui).on_hover_text("Save (Ctrl+S)").clicked() {
                self.save(root);
            }
            save_indicator(ui, icons, self.dirty());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(&self.status);
            });
        });
        ui.separator();

        // ---- library settings ----
        ui.horizontal(|ui| {
            ui.label("Texture");
            let mut tex = self.doc.texture.clone().unwrap_or_default();
            if ui
                .add(egui::TextEdit::singleline(&mut tex).hint_text("(colour tiles only)"))
                .changed()
            {
                self.doc.texture = (!tex.trim().is_empty()).then_some(tex);
            }
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.doc.tile_width)
                    .speed(1.0)
                    .range(1.0..=1024.0)
                    .prefix("tile w "),
            );
            ui.add(
                egui::DragValue::new(&mut self.doc.tile_height)
                    .speed(1.0)
                    .range(1.0..=1024.0)
                    .prefix("tile h "),
            );
        });
        ui.separator();

        self.palette_preview(ui);
        ui.add_space(4.0);

        ui.horizontal_top(|ui| {
            self.tile_list(ui);
            ui.separator();
            ui.vertical(|ui| self.tile_detail(ui, textures, root));
        });
    }

    // ---- diamond palette preview ------------------------------------------

    fn palette_preview(&mut self, ui: &mut Ui) {
        if self.doc.tiles.is_empty() {
            return;
        }
        let (tw, th) = (34.0_f32, 17.0_f32);
        let n = self.doc.tiles.len();
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, th + 10.0), Sense::hover());
        let painter = ui.painter_at(rect);
        let mut x = rect.left() + tw * 0.5 + 2.0;
        let cy = rect.center().y;
        for tile in &self.doc.tiles {
            if x + tw * 0.5 > rect.right() {
                break;
            }
            let c = tile.color;
            let col = Color32::from_rgba_unmultiplied(
                (c[0] * 255.0) as u8,
                (c[1] * 255.0) as u8,
                (c[2] * 255.0) as u8,
                (c[3] * 255.0) as u8,
            );
            let pts = vec![
                egui::pos2(x, cy - th * 0.5),
                egui::pos2(x + tw * 0.5, cy),
                egui::pos2(x, cy + th * 0.5),
                egui::pos2(x - tw * 0.5, cy),
            ];
            painter.add(egui::Shape::convex_polygon(
                pts,
                col,
                egui::Stroke::new(1.0, Color32::from_gray(40)),
            ));
            x += tw + 3.0;
        }
        ui.weak(format!("{n} tile{}", if n == 1 { "" } else { "s" }));
    }

    // ---- tile list --------------------------------------------------------

    fn tile_list(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            ui.set_min_width(150.0);
            ui.set_max_width(180.0);
            ui.strong("Tiles");
            egui::ScrollArea::vertical()
                .id_salt("tile_list")
                .max_height(280.0)
                .show(ui, |ui| {
                    for i in 0..self.doc.tiles.len() {
                        let selected = self.selected == Some(i);
                        let id = self.doc.tiles[i].id.clone();
                        let label = if id.trim().is_empty() {
                            format!("[{i}] (unnamed)")
                        } else {
                            format!("[{i}] {id}")
                        };
                        if ui.selectable_label(selected, label).clicked() {
                            self.selected = Some(i);
                        }
                    }
                });
            ui.separator();
            if ui.button("➕ Add tile").clicked() {
                self.doc.tiles.push(default_tile(self.doc.tiles.len()));
                self.selected = Some(self.doc.tiles.len() - 1);
            }
        });
    }

    // ---- selected-tile detail ---------------------------------------------

    fn tile_detail(&mut self, ui: &mut Ui, textures: &mut TextureCache, root: &Path) {
        let Some(idx) = self.selected.filter(|&i| i < self.doc.tiles.len()) else {
            ui.weak("Select or add a tile.");
            return;
        };
        let texture = self.doc.texture.clone();

        // Clip names of the tile's referenced library (loaded once via the cache),
        // resolved before the `&mut tile` borrow below.
        let frames_path = self.doc.tiles[idx].frames.clone();
        let clip_names: Vec<String> = frames_path
            .as_ref()
            .filter(|p| !p.trim().is_empty())
            .and_then(|p| self.anim_cache.get(p, root))
            .map(|lib| lib.clip_names())
            .unwrap_or_default();

        // Mutable tile borrow, scoped so the reorder/delete row below can borrow
        // `self.doc.tiles` again.
        let mut reload_anim = false;
        {
            let tile = &mut self.doc.tiles[idx];
            ui.horizontal(|ui| {
                ui.label("id");
                ui.add(egui::TextEdit::singleline(&mut tile.id).desired_width(140.0));
            });
            ui.horizontal(|ui| {
                ui.label("colour");
                ui.color_edit_button_rgba_unmultiplied(&mut tile.color);
            });

            // Optional atlas rect.
            let mut has_rect = tile.rect.is_some();
            if ui.checkbox(&mut has_rect, "Atlas rect (crop from texture)").changed() {
                tile.rect = has_rect.then(|| tile.rect.unwrap_or([0.0, 0.0, 32.0, 32.0]));
            }
            if let Some(rect) = tile.rect.as_mut() {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut rect[0]).speed(1.0).prefix("x "));
                    ui.add(egui::DragValue::new(&mut rect[1]).speed(1.0).prefix("y "));
                    ui.add(egui::DragValue::new(&mut rect[2]).speed(1.0).prefix("w "));
                    ui.add(egui::DragValue::new(&mut rect[3]).speed(1.0).prefix("h "));
                });
            }

            // ---- animated tile (references a sprite-frames clip) ----
            ui.separator();
            ui.strong("Animation");
            ui.horizontal(|ui| {
                ui.label("clip library");
                let mut frames = tile.frames.clone().unwrap_or_default();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut frames)
                        .hint_text(".spriteframes / .frames.json")
                        .desired_width(180.0),
                );
                if resp.changed() {
                    if frames.trim().is_empty() {
                        tile.frames = None;
                        tile.clip = None;
                    } else {
                        tile.frames = Some(frames);
                    }
                }
                if ui.small_button("⟳").on_hover_text("Reload clip list").clicked() {
                    reload_anim = true;
                }
            });
            if tile.frames.is_some() {
                ui.horizontal(|ui| {
                    ui.label("clip");
                    if clip_names.is_empty() {
                        // Library missing or empty: keep the field editable by hand.
                        let mut c = tile.clip.clone().unwrap_or_default();
                        if ui
                            .add(egui::TextEdit::singleline(&mut c).hint_text("clip name").desired_width(140.0))
                            .changed()
                        {
                            tile.clip = (!c.trim().is_empty()).then_some(c);
                        }
                    } else {
                        let cur = tile.clip.clone().unwrap_or_default();
                        egui::ComboBox::from_id_salt("tile_clip")
                            .selected_text(if cur.is_empty() { "(pick clip)" } else { cur.as_str() })
                            .show_ui(ui, |ui| {
                                for name in &clip_names {
                                    ui.selectable_value(&mut tile.clip, Some(name.clone()), name);
                                }
                            });
                    }
                });
                ui.weak("Animated tiles play on a shared clock; static rect/colour is the fallback.");
            }
        }
        if reload_anim {
            self.anim_cache.clear();
        }

        // Thumbnail (texture + rect required).
        if let (Some(tex_path), Some(rect)) = (texture.as_ref(), self.doc.tiles[idx].rect) {
            let ctx = ui.ctx().clone();
            if let Some(handle) = textures.get(&ctx, root, tex_path) {
                let (dest, _) = ui.allocate_exact_size(vec2(96.0, 96.0), Sense::hover());
                ui.painter().rect_filled(dest, 4.0, Color32::from_gray(30));
                paint_frame(ui, &handle, fit_rect(dest.shrink(6.0), rect), rect);
            }
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("⬆").on_hover_text("Move up").clicked() && idx > 0 {
                self.doc.tiles.swap(idx, idx - 1);
                self.selected = Some(idx - 1);
            }
            if ui.button("⬇").on_hover_text("Move down").clicked() && idx + 1 < self.doc.tiles.len() {
                self.doc.tiles.swap(idx, idx + 1);
                self.selected = Some(idx + 1);
            }
            if ui.button("Duplicate").clicked() {
                let t = self.doc.tiles[idx].clone();
                self.doc.tiles.insert(idx + 1, t);
                self.selected = Some(idx + 1);
            }
            if ui.button("🗑 Delete").clicked() {
                self.doc.tiles.remove(idx);
                self.selected = (!self.doc.tiles.is_empty()).then(|| idx.saturating_sub(1));
            }
        });
        ui.weak("Tip: the tile index is what worldgen biome/ore bands paint.");
    }
}

// ---- free helpers ----------------------------------------------------------

/// A small saved/unsaved status chip using the lucide icon pipeline (the plain
/// `●` char renders as tofu in egui's font). Shared by the asset editors.
pub(crate) fn save_indicator(ui: &mut Ui, icons: &Icons, dirty: bool) {
    if dirty {
        icons.icon(Icon::Modified).size(12.0).role(IconRole::Warning).show(ui);
        ui.weak("unsaved");
    } else {
        ui.weak("saved");
    }
}

fn default_tile(n: usize) -> TileDoc {
    // Spread hues so freshly added tiles are visually distinct.
    let hue = (n as f32 * 0.618_034).fract();
    let (r, g, b) = hsv_to_rgb(hue, 0.55, 0.85);
    TileDoc {
        id: format!("tile{n}"),
        color: [r, g, b, 1.0],
        rect: None,
        frames: None,
        clip: None,
    }
}

/// A minimal starter tileset (used for "New tileset" and bad-JSON recovery).
pub fn default_doc() -> TileSetDoc {
    let tile = |id: &str, color: [f32; 4]| TileDoc {
        id: id.into(),
        color,
        rect: None,
        frames: None,
        clip: None,
    };
    TileSetDoc {
        texture: None,
        tile_width: 64.0,
        tile_height: 32.0,
        tiles: vec![
            tile("water", [0.15, 0.35, 0.7, 1.0]),
            tile("grass", [0.3, 0.6, 0.28, 1.0]),
            tile("rock", [0.5, 0.5, 0.52, 1.0]),
        ],
    }
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    match (i as i32).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_is_clean_and_edits_mark_dirty() {
        let mut ed = TileSetEditor::default();
        let json = r#"{"tile_width":64,"tile_height":32,"tiles":[{"id":"water","color":[0.1,0.3,0.7,1.0]}]}"#;
        ed.open_doc("world.tileset.json", json);
        assert!(ed.open);
        assert_eq!(ed.selected, Some(0));
        assert!(!ed.dirty(), "freshly opened file should not read dirty");
        ed.doc.tiles.push(default_tile(1));
        assert!(ed.dirty(), "an edit should mark the doc dirty");
    }

    #[test]
    fn bad_json_recovers_to_a_usable_starter() {
        let mut ed = TileSetEditor::default();
        ed.open_doc("broken.tileset.json", "{ not json");
        assert!(ed.open);
        assert!(!ed.doc.tiles.is_empty(), "recovery gives an editable palette");
    }
}
