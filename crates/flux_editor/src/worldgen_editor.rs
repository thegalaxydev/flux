//! The worldgen editor: a floating window for authoring `*.worldgen.json`
//! procedural-generation configs. It edits a [`WorldGenDoc`] in place — global
//! noise scales, an ordered biome table, and an ore table — with a **live
//! minimap preview** produced by the real [`generate`] function against a chosen
//! `TileSet`, so what you see is exactly what a `Tilemap` will build. Modelled
//! on [`crate::animation_editor::AnimationEditor`].

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use eframe::egui::{self, Color32, Sense, Ui, vec2};
use flux_core::tilemap::{
    BiomeDoc, OreDoc, TileGrid, TileSet, TileSetCache, WorldGen, WorldGenDoc, generate,
};
use flux_icons::{Icon, Icons};

use crate::tileset_editor::save_indicator;

/// Fallback colours for the built-in placeholder generator (indices 0..3) when
/// no tileset is chosen, so the preview still shows terrain shape.
const PLACEHOLDER: [Color32; 4] = [
    Color32::from_rgb(46, 89, 158),  // water
    Color32::from_rgb(196, 178, 118), // sand
    Color32::from_rgb(78, 138, 66),  // grass
    Color32::from_rgb(120, 120, 126), // rock
];

pub struct WorldGenEditor {
    pub open: bool,
    /// Project-relative path of the open config.
    rel: String,
    doc: WorldGenDoc,
    /// JSON of the last saved state, for the dirty check.
    saved: String,
    status: String,
    /// Set for one frame after a successful save (drop caches, regenerate maps).
    just_saved: bool,

    // ---- preview state (editor-only, not part of the doc) ----
    /// Project-relative `*.tileset.json` the preview resolves tile ids against.
    tileset_path: String,
    seed: i64,
    preview_w: u32,
    preview_h: u32,
    cache: TileSetCache,
    grid: Option<TileGrid>,
    /// Signature of the `(doc, tileset, seed, size)` the cached grid came from.
    grid_sig: u64,
}

impl Default for WorldGenEditor {
    fn default() -> Self {
        Self {
            open: false,
            rel: String::new(),
            doc: WorldGenDoc::from_json("{}").unwrap(),
            saved: String::new(),
            status: String::new(),
            just_saved: false,
            tileset_path: String::new(),
            seed: 1,
            preview_w: 72,
            preview_h: 48,
            cache: TileSetCache::default(),
            grid: None,
            grid_sig: 0,
        }
    }
}

impl WorldGenEditor {
    /// Load `json` (a `*.worldgen.json`) for editing. Invalid JSON opens an empty
    /// document so the user can start over rather than being blocked. `root` lets
    /// us auto-pick a sibling tileset for the preview.
    pub fn open_doc(&mut self, rel: &str, json: &str, root: &Path) {
        let doc = WorldGenDoc::from_json(json).unwrap_or_else(|_| default_doc());
        self.rel = rel.to_string();
        self.saved = doc.to_json();
        self.doc = doc;
        self.status.clear();
        self.cache.clear();
        self.grid = None;
        self.grid_sig = 0;
        self.tileset_path = guess_sibling_tileset(root, rel).unwrap_or_default();
        self.open = true;
    }

    pub fn dirty(&self) -> bool {
        self.doc.to_json() != self.saved
    }

    /// Project-relative path of the open config (for the tab label).
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
    pub fn dock_ui(&mut self, ui: &mut Ui, root: &Path, icons: &Icons) {
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

        // Resolve the preview tileset (cached) and its ordered tile ids.
        let tileset = self.cache.get(&self.tileset_path, root);
        let ids: Vec<String> = tileset
            .as_deref()
            .map(tile_ids)
            .unwrap_or_default();

        ui.horizontal_top(|ui| {
            // ---- left: config tables ----
            ui.vertical(|ui| {
                ui.set_min_width(430.0);
                let avail_h = ui.available_height();
                egui::ScrollArea::vertical()
                    .id_salt("worldgen_tables")
                    .max_height(avail_h)
                    .show(ui, |ui| {
                        self.globals(ui);
                        ui.separator();
                        self.biome_table(ui, &ids);
                        ui.separator();
                        self.ore_table(ui, &ids);
                    });
            });
            ui.separator();
            // ---- right: live preview ----
            ui.vertical(|ui| {
                self.preview_controls(ui, root, tileset.is_some());
                self.preview(ui, tileset.as_deref());
            });
        });
    }

    fn globals(&mut self, ui: &mut Ui) {
        ui.strong("Noise");
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.doc.elevation_scale)
                    .speed(0.25)
                    .range(1.0..=256.0)
                    .prefix("elevation "),
            )
            .on_hover_text("Feature size of the elevation noise — bigger = broader landmasses");
            ui.add(
                egui::DragValue::new(&mut self.doc.moisture_scale)
                    .speed(0.25)
                    .range(1.0..=256.0)
                    .prefix("moisture "),
            )
            .on_hover_text("Feature size of the moisture noise (forest vs plains)");
        });
    }

    // ---- biome table ------------------------------------------------------

    fn biome_table(&mut self, ui: &mut Ui, ids: &[String]) {
        ui.horizontal(|ui| {
            ui.strong("Biomes");
            ui.weak("(first matching band wins — put specific ones first)");
        });
        let mut op: Option<RowOp> = None;
        for i in 0..self.doc.biomes.len() {
            ui.push_id(("biome", i), |ui| {
                ui.horizontal(|ui| {
                    tile_id_picker(ui, &mut self.doc.biomes[i].tile, ids, "b");
                    let b = &mut self.doc.biomes[i];
                    ui.add(
                        egui::DragValue::new(&mut b.max_elevation)
                            .speed(0.01)
                            .range(0.0..=1.0)
                            .prefix("≤elev "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut b.min_moisture)
                            .speed(0.01)
                            .range(0.0..=1.0)
                            .prefix("moist "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut b.max_moisture)
                            .speed(0.01)
                            .range(0.0..=1.0)
                            .prefix("..="),
                    );
                    if let Some(o) = row_buttons(ui, i, self.doc.biomes.len()) {
                        op = Some(o);
                    }
                });
            });
        }
        if let Some(op) = op {
            op.apply(&mut self.doc.biomes);
        }
        if ui.button("➕ Add biome").clicked() {
            self.doc.biomes.push(BiomeDoc {
                tile: ids.first().cloned().unwrap_or_default(),
                max_elevation: 1.0,
                min_moisture: 0.0,
                max_moisture: 1.0,
            });
        }
    }

    // ---- ore table --------------------------------------------------------

    fn ore_table(&mut self, ui: &mut Ui, ids: &[String]) {
        ui.strong("Ores");
        let mut op: Option<RowOp> = None;
        for i in 0..self.doc.ores.len() {
            ui.push_id(("ore", i), |ui| {
                ui.horizontal(|ui| {
                    tile_id_picker(ui, &mut self.doc.ores[i].tile, ids, "o");
                    let o = &mut self.doc.ores[i];
                    ui.add(
                        egui::DragValue::new(&mut o.frequency)
                            .speed(0.005)
                            .range(0.0..=1.0)
                            .prefix("freq "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut o.richness)
                            .speed(10.0)
                            .range(0.0..=65535.0)
                            .prefix("rich "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut o.scale)
                            .speed(0.25)
                            .range(1.0..=256.0)
                            .prefix("scale "),
                    );
                    if let Some(o) = row_buttons(ui, i, self.doc.ores.len()) {
                        op = Some(o);
                    }
                });
                let o = &mut self.doc.ores[i];
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    ui.add(
                        egui::DragValue::new(&mut o.min_elevation)
                            .speed(0.01)
                            .range(0.0..=1.0)
                            .prefix("elev "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut o.max_elevation)
                            .speed(0.01)
                            .range(0.0..=1.0)
                            .prefix("..="),
                    );
                    on_terrain_picker(ui, &mut o.on, ids);
                });
            });
        }
        if let Some(op) = op {
            op.apply(&mut self.doc.ores);
        }
        if ui.button("➕ Add ore").clicked() {
            self.doc.ores.push(OreDoc {
                tile: ids.first().cloned().unwrap_or_default(),
                frequency: 0.06,
                richness: 2000.0,
                min_elevation: 0.0,
                max_elevation: 1.0,
                scale: 5.0,
                on: Vec::new(),
            });
        }
    }

    // ---- preview ----------------------------------------------------------

    fn preview_controls(&mut self, ui: &mut Ui, root: &Path, has_tileset: bool) {
        ui.strong("Preview");
        ui.horizontal(|ui| {
            ui.label("tileset");
            ui.add(
                egui::TextEdit::singleline(&mut self.tileset_path)
                    .hint_text("*.tileset.json")
                    .desired_width(180.0),
            );
            if ui.small_button("⟳").on_hover_text("Reload tileset from disk").clicked() {
                self.cache.clear();
                self.grid_sig = 0;
            }
        });
        if !has_tileset && !self.tileset_path.trim().is_empty() {
            ui.colored_label(Color32::from_rgb(210, 140, 60), "tileset not found");
        } else if !has_tileset {
            ui.weak("Pick a tileset to colour the preview by tile id.");
        }
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.seed).speed(1.0).prefix("seed "));
            ui.add(
                egui::DragValue::new(&mut self.preview_w)
                    .speed(1.0)
                    .range(8..=256)
                    .prefix("w "),
            );
            ui.add(
                egui::DragValue::new(&mut self.preview_h)
                    .speed(1.0)
                    .range(8..=256)
                    .prefix("h "),
            );
        });
        let _ = root;
    }

    fn preview(&mut self, ui: &mut Ui, tileset: Option<&TileSet>) {
        self.refresh_grid(tileset);
        let Some(grid) = &self.grid else { return };
        let (gw, gh) = (grid.width(), grid.height());
        if gw == 0 || gh == 0 {
            return;
        }
        let avail = ui.available_size();
        let cell = (avail.x / gw as f32).min(avail.y / gh as f32).clamp(1.0, 12.0);
        let size = vec2(gw as f32 * cell, gh as f32 * cell);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, Color32::from_gray(20));
        for row in 0..gh as i32 {
            for col in 0..gw as i32 {
                let Some(c) = grid.cell(col, row) else { continue };
                let base = tile_color(tileset, c.tile);
                let min = rect.min + vec2(col as f32 * cell, row as f32 * cell);
                let cell_rect = egui::Rect::from_min_size(min, vec2(cell, cell));
                painter.rect_filled(cell_rect, 0.0, base);
                if c.has_ore() && cell >= 3.0 {
                    let pip = tile_color(tileset, c.ore);
                    painter.rect_filled(cell_rect.shrink(cell * 0.3), 0.0, pip);
                }
            }
        }
    }

    /// Regenerate the cached preview grid when its inputs changed.
    fn refresh_grid(&mut self, tileset: Option<&TileSet>) {
        let mut h = DefaultHasher::new();
        self.doc.to_json().hash(&mut h);
        self.tileset_path.hash(&mut h);
        self.seed.hash(&mut h);
        self.preview_w.hash(&mut h);
        self.preview_h.hash(&mut h);
        let sig = h.finish();
        if self.grid.is_some() && sig == self.grid_sig {
            return;
        }
        let wg = WorldGen::from_doc(&self.doc);
        self.grid = Some(generate(
            self.preview_w,
            self.preview_h,
            self.seed as u64,
            Some(&wg),
            tileset,
        ));
        self.grid_sig = sig;
    }
}

// ---- row reordering --------------------------------------------------------

enum RowOp {
    Up(usize),
    Down(usize),
    Delete(usize),
}

impl RowOp {
    fn apply<T>(self, v: &mut Vec<T>) {
        match self {
            RowOp::Up(i) if i > 0 => v.swap(i, i - 1),
            RowOp::Down(i) if i + 1 < v.len() => v.swap(i, i + 1),
            RowOp::Delete(i) if i < v.len() => {
                v.remove(i);
            }
            _ => {}
        }
    }
}

/// Up / down / delete buttons for a table row; returns the requested op.
fn row_buttons(ui: &mut Ui, i: usize, len: usize) -> Option<RowOp> {
    let mut op = None;
    ui.add_enabled_ui(i > 0, |ui| {
        if ui.small_button("⬆").clicked() {
            op = Some(RowOp::Up(i));
        }
    });
    ui.add_enabled_ui(i + 1 < len, |ui| {
        if ui.small_button("⬇").clicked() {
            op = Some(RowOp::Down(i));
        }
    });
    if ui.small_button("🗑").clicked() {
        op = Some(RowOp::Delete(i));
    }
    op
}

// ---- free helpers ----------------------------------------------------------

fn tile_ids(ts: &TileSet) -> Vec<String> {
    (0..ts.len() as u16)
        .filter_map(|i| ts.tile(i).map(|t| t.id.clone()))
        .collect()
}

fn tile_color(tileset: Option<&TileSet>, index: u16) -> Color32 {
    if let Some(t) = tileset.and_then(|ts| ts.tile(index)) {
        let c = t.color;
        return Color32::from_rgba_unmultiplied(
            (c.r * 255.0) as u8,
            (c.g * 255.0) as u8,
            (c.b * 255.0) as u8,
            (c.a * 255.0) as u8,
        );
    }
    PLACEHOLDER[(index as usize).min(PLACEHOLDER.len() - 1)]
}

/// A tile-id widget: a `ComboBox` of the tileset's ids when known, else a free
/// text field (so worldgen stays editable without a tileset).
fn tile_id_picker(ui: &mut Ui, value: &mut String, ids: &[String], salt: &str) {
    if ids.is_empty() {
        ui.add(egui::TextEdit::singleline(value).hint_text("tile id").desired_width(90.0));
        return;
    }
    // The row is already scoped by `ui.push_id`, so a per-widget salt suffices
    // and stays stable across frames (keeping the popup's open state).
    egui::ComboBox::from_id_salt(salt)
        .width(96.0)
        .selected_text(if value.is_empty() { "(pick)" } else { value.as_str() })
        .show_ui(ui, |ui| {
            for id in ids {
                ui.selectable_value(value, id.clone(), id);
            }
        });
}

/// A multi-select of terrain ids an ore may spawn on (empty = any terrain).
fn on_terrain_picker(ui: &mut Ui, on: &mut Vec<String>, ids: &[String]) {
    let label = if on.is_empty() {
        "on: any".to_string()
    } else {
        format!("on: {}", on.join(", "))
    };
    if ids.is_empty() {
        // No tileset: fall back to a comma-separated text field.
        let mut s = on.join(", ");
        if ui
            .add(egui::TextEdit::singleline(&mut s).hint_text("terrain ids (any)").desired_width(150.0))
            .changed()
        {
            *on = s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
        }
        return;
    }
    ui.menu_button(label, |ui| {
        for id in ids {
            let mut checked = on.contains(id);
            if ui.checkbox(&mut checked, id).changed() {
                if checked {
                    on.push(id.clone());
                } else {
                    on.retain(|x| x != id);
                }
            }
        }
    });
}

/// A minimal starter config (used for "New worldgen" and bad-JSON recovery).
pub fn default_doc() -> WorldGenDoc {
    let json = r#"{
  "elevation_scale": 14,
  "moisture_scale": 9,
  "biomes": [
    { "tile": "water", "max_elevation": 0.35 },
    { "tile": "grass", "max_elevation": 0.75 },
    { "tile": "rock",  "max_elevation": 1.0 }
  ],
  "ores": []
}"#;
    WorldGenDoc::from_json(json).unwrap()
}

/// First `*.tileset.json` sitting in the same folder as the worldgen file, so a
/// freshly opened editor can colour its preview without manual setup.
fn guess_sibling_tileset(root: &Path, rel: &str) -> Option<String> {
    let dir_rel = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let dir = if dir_rel.is_empty() { root.to_path_buf() } else { root.join(dir_rel) };
    let name = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .find(|n| n.to_ascii_lowercase().ends_with(".tileset.json"))?;
    Some(if dir_rel.is_empty() { name } else { format!("{dir_rel}/{name}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_is_clean_and_edits_mark_dirty() {
        let mut ed = WorldGenEditor::default();
        let json = r#"{"elevation_scale":8,"moisture_scale":6,"biomes":[{"tile":"water","max_elevation":0.4}],"ores":[]}"#;
        ed.open_doc("world.worldgen.json", json, Path::new("."));
        assert!(ed.open);
        assert!(!ed.dirty(), "freshly opened file should not read dirty");
        ed.doc.elevation_scale = 20.0;
        assert!(ed.dirty(), "an edit should mark the doc dirty");
    }

    #[test]
    fn preview_regenerates_when_inputs_change() {
        let ts = TileSet::parse(
            r#"{"tiles":[{"id":"water"},{"id":"grass"},{"id":"rock"}]}"#,
        )
        .unwrap();
        let mut ed = WorldGenEditor::default();
        ed.open_doc("w.worldgen.json", &default_doc().to_json(), Path::new("."));
        ed.preview_w = 32;
        ed.preview_h = 24;

        ed.refresh_grid(Some(&ts));
        let sig1 = ed.grid_sig;
        assert_eq!(ed.grid.as_ref().unwrap().width(), 32);
        assert_eq!(ed.grid.as_ref().unwrap().height(), 24);

        // Same inputs -> no regeneration (signature stable).
        ed.refresh_grid(Some(&ts));
        assert_eq!(ed.grid_sig, sig1);

        // Changing the seed changes the signature and the grid.
        ed.seed = 99;
        ed.refresh_grid(Some(&ts));
        assert_ne!(ed.grid_sig, sig1);
    }
}
