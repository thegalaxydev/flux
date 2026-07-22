//! The animation editor: a floating window for authoring `*.frames.json` clip
//! libraries. It edits a [`FramesDoc`] in place — clip list, a duration-scaled
//! timeline of thumbnails sliced from the texture, playback preview, per-frame
//! rect/duration editing, drag reordering, and a grid slicer that generates
//! frames from a uniform sheet — then saves the JSON back to the project.

use std::path::Path;

use eframe::egui::{self, Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use eframe::egui::epaint::Mesh;
use flux_core::animation::{ClipDoc, FrameDoc, FramesDoc};
use flux_icons::{Icon, Icons};

use crate::textures::TextureCache;
use crate::tileset_editor::save_indicator;

/// Grid-slicer dialog state.
struct Slicer {
    cols: i32,
    rows: i32,
    fps: f32,
}

impl Default for Slicer {
    fn default() -> Self {
        Self { cols: 4, rows: 1, fps: 12.0 }
    }
}

#[derive(Default)]
pub struct AnimationEditor {
    pub open: bool,
    /// Project-relative path of the open library.
    rel: String,
    doc: FramesDoc,
    /// JSON of the last saved state, for the dirty check.
    saved: String,
    selected_clip: Option<String>,
    selected_frame: usize,
    playing: bool,
    time: f32,
    /// Frame index currently being dragged in the timeline.
    drag: Option<usize>,
    slicer: Option<Slicer>,
    new_clip_name: String,
    status: String,
    /// Set for one frame after a successful save, so the app can drop the shared
    /// animation cache and refresh any live `AnimatedSprite` using this library.
    just_saved: bool,
}

impl AnimationEditor {
    /// Load `json` (a `*.frames.json`) for editing. Invalid JSON opens an empty
    /// document so the user can start over rather than being blocked.
    pub fn open_doc(&mut self, rel: &str, json: &str) {
        let doc = FramesDoc::from_json(json).unwrap_or_default();
        self.selected_clip = doc.clips.keys().next().cloned();
        self.rel = rel.to_string();
        // Baseline for the dirty check is the *normalized* JSON, so a freshly
        // opened file (whose on-disk formatting may differ) doesn't read dirty.
        self.saved = doc.to_json();
        self.doc = doc;
        self.selected_frame = 0;
        self.playing = false;
        self.time = 0.0;
        self.drag = None;
        self.slicer = None;
        self.status.clear();
        self.open = true;
    }

    pub fn dirty(&self) -> bool {
        self.doc.to_json() != self.saved
    }

    /// Project-relative path of the open library (for the tab label).
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

        ui.horizontal_top(|ui| {
            self.clip_list(ui);
            ui.separator();
            ui.vertical(|ui| self.clip_detail(ui, textures, root));
        });

        self.slicer_window(ui.ctx(), textures, root);
    }

    // ---- clip list ---------------------------------------------------------

    fn clip_list(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            ui.set_min_width(150.0);
            ui.set_max_width(170.0);
            ui.strong("Clips");
            egui::ScrollArea::vertical()
                .id_salt("clip_list")
                .max_height(300.0)
                .show(ui, |ui| {
                let names: Vec<String> = self.doc.clips.keys().cloned().collect();
                for name in names {
                    let selected = self.selected_clip.as_deref() == Some(name.as_str());
                    let count = self.doc.clips.get(&name).map(|c| c.frames.len()).unwrap_or(0);
                    if ui
                        .selectable_label(selected, format!("{name}  ({count})"))
                        .clicked()
                    {
                        self.selected_clip = Some(name.clone());
                        self.selected_frame = 0;
                        self.time = 0.0;
                        self.playing = false;
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_clip_name)
                        .hint_text("new clip")
                        .desired_width(96.0),
                );
                if ui.button("➕").clicked() {
                    let name = self.new_clip_name.trim().to_string();
                    if !name.is_empty() && !self.doc.clips.contains_key(&name) {
                        self.doc.clips.insert(name.clone(), ClipDoc::default());
                        self.selected_clip = Some(name);
                        self.new_clip_name.clear();
                    }
                }
            });
            if let Some(sel) = self.selected_clip.clone() {
                if ui.button("🗑 Delete clip").clicked() {
                    self.doc.clips.shift_remove(&sel);
                    self.selected_clip = self.doc.clips.keys().next().cloned();
                    self.selected_frame = 0;
                }
            }
        });
    }

    // ---- clip detail -------------------------------------------------------

    fn clip_detail(&mut self, ui: &mut Ui, textures: &mut TextureCache, root: &Path) {
        let Some(name) = self.selected_clip.clone() else {
            ui.weak("Select or create a clip.");
            return;
        };
        let doc_texture = self.doc.texture.clone();

        // Settings row (mutable clip borrow, scoped).
        {
            let Some(clip) = self.doc.clips.get_mut(&name) else { return };
            ui.horizontal(|ui| {
                ui.checkbox(&mut clip.looped, "Loop");
                ui.add(egui::DragValue::new(&mut clip.speed).speed(0.05).range(0.0..=16.0).prefix("speed "));
            });
            ui.horizontal(|ui| {
                ui.label("Texture");
                let mut tex = clip.texture.clone().unwrap_or_default();
                if ui
                    .add(egui::TextEdit::singleline(&mut tex).hint_text(doc_texture.as_deref().unwrap_or("(library default)")))
                    .changed()
                {
                    clip.texture = (!tex.trim().is_empty()).then_some(tex);
                }
            });
        }

        // Effective texture for previews/thumbnails (frame overrides handled per cell).
        let clip_tex = self
            .doc
            .clips
            .get(&name)
            .and_then(|c| c.texture.clone())
            .or(doc_texture.clone());

        // Playback + FPS controls.
        let total = self.doc.clips.get(&name).map(clip_total).unwrap_or(0.0);
        ui.horizontal(|ui| {
            if ui.button(if self.playing { "⏸" } else { "▶" }).clicked() {
                self.playing = !self.playing;
            }
            if ui.button("⏹").clicked() {
                self.playing = false;
                self.time = 0.0;
            }
            let mut t = self.time;
            if ui
                .add(egui::Slider::new(&mut t, 0.0..=total.max(0.0001)).show_value(false).text("time"))
                .changed()
            {
                self.time = t;
                self.playing = false;
            }
            ui.monospace(format!("{:.2}s / {:.2}s", self.time, total));
            if ui.button("Import sheet…").clicked() {
                self.slicer = Some(Slicer::default());
            }
        });

        // Advance the preview clock.
        if self.playing && total > 0.0 {
            self.time += ui.input(|i| i.stable_dt);
            if self.time >= total {
                let looped = self.doc.clips.get(&name).map(|c| c.looped).unwrap_or(false);
                if looped {
                    self.time = self.time.rem_euclid(total);
                } else {
                    self.time = total;
                    self.playing = false;
                }
            }
            ui.ctx().request_repaint();
        }

        // Current frame from the clock; keep the selection in sync while playing.
        let cur = self.doc.clips.get(&name).map(|c| frame_at(c, self.time)).unwrap_or(0);
        if self.playing {
            self.selected_frame = cur;
        }

        // ---- big preview ----
        let ctx = ui.ctx().clone();
        {
            let clip = self.doc.clips.get(&name);
            let frame = clip.and_then(|c| c.frames.get(cur));
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().min(360.0), 150.0), Sense::hover());
            ui.painter().rect_filled(rect, 4.0, Color32::from_gray(30));
            if let Some(frame) = frame {
                let tex_path = frame.texture.clone().or(clip_tex.clone());
                if let Some(handle) = tex_path.and_then(|p| textures.get(&ctx, root, &p)) {
                    let dest = fit_rect(rect.shrink(8.0), frame.rect);
                    paint_frame(ui, &handle, dest, frame.rect);
                }
            } else {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "no frames",
                    egui::FontId::proportional(13.0),
                    Color32::GRAY,
                );
            }
        }

        ui.add_space(4.0);
        self.timeline(ui, textures, root, &name, clip_tex.clone());
        ui.add_space(4.0);
        self.frame_inspector(ui, &name);
    }

    // ---- timeline ----------------------------------------------------------

    fn timeline(&mut self, ui: &mut Ui, textures: &mut TextureCache, root: &Path, name: &str, clip_tex: Option<String>) {
        let ctx = ui.ctx().clone();
        ui.strong("Timeline");
        let frames = match self.doc.clips.get(name) {
            Some(c) if !c.frames.is_empty() => c.frames.clone(),
            _ => {
                ui.weak("No frames yet — use “Import sheet…” or add frames.");
                if ui.button("➕ Add blank frame").clicked() {
                    self.push_frame(name, FrameDoc { rect: [0.0, 0.0, 32.0, 32.0], duration: 1.0 / 12.0, texture: None });
                }
                return;
            }
        };

        let total: f32 = frames.iter().map(|f| f.duration.max(1e-4)).sum();
        egui::ScrollArea::horizontal()
            .id_salt("timeline")
            .max_height(96.0)
            .show(ui, |ui| {
            ui.horizontal(|ui| {
                let h = 72.0;
                for (i, frame) in frames.iter().enumerate() {
                    // Width proportional to duration so variable timing is legible.
                    let w = (frame.duration.max(1e-4) / total * 460.0).clamp(28.0, 180.0);
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click_and_drag());
                    ui.painter().rect_filled(rect, 3.0, Color32::from_gray(24));
                    let tex_path = frame.texture.clone().or(clip_tex.clone());
                    if let Some(handle) = tex_path.and_then(|p| textures.get(&ctx, root, &p)) {
                        paint_frame(ui, &handle, fit_rect(rect.shrink(4.0), frame.rect), frame.rect);
                    }
                    let selected = i == self.selected_frame;
                    let stroke = if selected {
                        Stroke::new(2.0, Color32::from_rgb(255, 200, 60))
                    } else if resp.hovered() {
                        Stroke::new(1.0, Color32::from_rgb(120, 180, 240))
                    } else {
                        Stroke::new(1.0, Color32::from_gray(60))
                    };
                    ui.painter().rect_stroke(rect, 3.0, stroke, StrokeKind::Inside);
                    ui.painter().text(
                        rect.left_top() + vec2(3.0, 2.0),
                        egui::Align2::LEFT_TOP,
                        i.to_string(),
                        egui::FontId::monospace(10.0),
                        Color32::from_gray(180),
                    );

                    if resp.clicked() {
                        self.selected_frame = i;
                        self.playing = false;
                    }
                    // Drag to reorder: swap when the pointer crosses into a neighbour.
                    if resp.drag_started() {
                        self.drag = Some(i);
                    }
                    if let Some(from) = self.drag {
                        if resp.dragged() {
                            if let Some(p) = resp.interact_pointer_pos() {
                                let over = i;
                                if p.x < rect.center().x && over > 0 && from == over {
                                    self.reorder(name, from, over - 1);
                                    self.drag = Some(over - 1);
                                    self.selected_frame = over - 1;
                                } else if p.x > rect.center().x && from == over && over + 1 < frames.len() {
                                    self.reorder(name, from, over + 1);
                                    self.drag = Some(over + 1);
                                    self.selected_frame = over + 1;
                                }
                            }
                        }
                    }
                    if resp.drag_stopped() {
                        self.drag = None;
                    }
                }
                if ui.add_sized(vec2(28.0, 72.0), egui::Button::new("➕")).clicked() {
                    let last = frames.last().cloned().unwrap_or(FrameDoc { rect: [0.0, 0.0, 32.0, 32.0], duration: 1.0 / 12.0, texture: None });
                    self.push_frame(name, last);
                }
            });
        });
    }

    // ---- selected-frame inspector -----------------------------------------

    fn frame_inspector(&mut self, ui: &mut Ui, name: &str) {
        let idx = self.selected_frame;
        let len = self.doc.clips.get(name).map(|c| c.frames.len()).unwrap_or(0);
        if idx >= len {
            return;
        }
        let Some(clip) = self.doc.clips.get_mut(name) else { return };
        let frame = &mut clip.frames[idx];
        ui.strong(format!("Frame {idx}"));
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut frame.rect[0]).speed(1.0).prefix("x "));
            ui.add(egui::DragValue::new(&mut frame.rect[1]).speed(1.0).prefix("y "));
            ui.add(egui::DragValue::new(&mut frame.rect[2]).speed(1.0).prefix("w "));
            ui.add(egui::DragValue::new(&mut frame.rect[3]).speed(1.0).prefix("h "));
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut frame.duration)
                    .speed(0.005)
                    .range(0.001..=10.0)
                    .prefix("dur ")
                    .suffix("s"),
            );
            let mut fps = if frame.duration > 0.0 { 1.0 / frame.duration } else { 0.0 };
            if ui.add(egui::DragValue::new(&mut fps).speed(0.5).range(0.1..=240.0).suffix(" fps")).changed()
                && fps > 0.0
            {
                frame.duration = 1.0 / fps;
            }
        });
        ui.horizontal(|ui| {
            if ui.button("⬅ Move").clicked() && idx > 0 {
                let f = clip.frames.remove(idx);
                clip.frames.insert(idx - 1, f);
                self.selected_frame = idx - 1;
            }
            if ui.button("Move ➡").clicked() && idx + 1 < clip.frames.len() {
                let f = clip.frames.remove(idx);
                clip.frames.insert(idx + 1, f);
                self.selected_frame = idx + 1;
            }
            if ui.button("Duplicate").clicked() {
                let f = clip.frames[idx].clone();
                clip.frames.insert(idx + 1, f);
                self.selected_frame = idx + 1;
            }
            if ui.button("🗑 Delete").clicked() {
                clip.frames.remove(idx);
                self.selected_frame = idx.saturating_sub(1);
            }
        });
    }

    // ---- grid slicer -------------------------------------------------------

    fn slicer_window(&mut self, ctx: &egui::Context, textures: &mut TextureCache, root: &Path) {
        let Some(mut slicer) = self.slicer.take() else { return };
        let Some(name) = self.selected_clip.clone() else { return };
        let mut open = true;
        let mut apply = false;
        egui::Window::new("Import from sheet")
            .id(egui::Id::new("anim_slicer"))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Slice the clip's texture into a uniform grid of frames.");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut slicer.cols).range(1..=64).prefix("cols "));
                    ui.add(egui::DragValue::new(&mut slicer.rows).range(1..=64).prefix("rows "));
                });
                ui.add(egui::DragValue::new(&mut slicer.fps).range(0.1..=240.0).suffix(" fps"));
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Generate").clicked() {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        open = false;
                    }
                });
            });

        if apply {
            let tex_path = self
                .doc
                .clips
                .get(&name)
                .and_then(|c| c.texture.clone())
                .or(self.doc.texture.clone());
            match tex_path.and_then(|p| textures.get(ctx, root, &p)) {
                Some(handle) => {
                    let sz = handle.size();
                    let (fw, fh) = (sz[0] as f32 / slicer.cols as f32, sz[1] as f32 / slicer.rows as f32);
                    let dur = 1.0 / slicer.fps.max(0.1);
                    let mut frames = Vec::new();
                    for r in 0..slicer.rows {
                        for c in 0..slicer.cols {
                            frames.push(FrameDoc {
                                rect: [c as f32 * fw, r as f32 * fh, fw, fh],
                                duration: dur,
                                texture: None,
                            });
                        }
                    }
                    if let Some(clip) = self.doc.clips.get_mut(&name) {
                        clip.frames = frames;
                    }
                    self.selected_frame = 0;
                    self.status = format!("Generated {} frames", slicer.cols * slicer.rows);
                }
                None => self.status = "Set the clip/library texture first".to_string(),
            }
            open = false;
        }
        if open {
            self.slicer = Some(slicer);
        }
    }

    // ---- mutation helpers --------------------------------------------------

    fn push_frame(&mut self, name: &str, frame: FrameDoc) {
        if let Some(clip) = self.doc.clips.get_mut(name) {
            clip.frames.push(frame);
            self.selected_frame = clip.frames.len() - 1;
        }
    }

    fn reorder(&mut self, name: &str, from: usize, to: usize) {
        if let Some(clip) = self.doc.clips.get_mut(name) {
            if from < clip.frames.len() && to < clip.frames.len() {
                let f = clip.frames.remove(from);
                clip.frames.insert(to, f);
            }
        }
    }
}

// ---- free helpers ----------------------------------------------------------

fn clip_total(clip: &ClipDoc) -> f32 {
    clip.frames.iter().map(|f| f.duration.max(1e-4)).sum()
}

/// Index of the frame shown at clip-time `t`.
fn frame_at(clip: &ClipDoc, t: f32) -> usize {
    let mut acc = 0.0;
    for (i, f) in clip.frames.iter().enumerate() {
        acc += f.duration.max(1e-4);
        if t < acc {
            return i;
        }
    }
    clip.frames.len().saturating_sub(1)
}

/// Fit a frame's pixel rect inside `bounds` preserving aspect ratio, centred.
pub(crate) fn fit_rect(bounds: Rect, src: [f32; 4]) -> Rect {
    let (sw, sh) = (src[2].max(1.0), src[3].max(1.0));
    let scale = (bounds.width() / sw).min(bounds.height() / sh);
    let size = vec2(sw * scale, sh * scale);
    Rect::from_center_size(bounds.center(), size)
}

/// Paint a texture sub-region (`src` in pixels) into `dest`.
pub(crate) fn paint_frame(ui: &Ui, tex: &egui::TextureHandle, dest: Rect, src: [f32; 4]) {
    let sz = tex.size();
    let (tw, th) = (sz[0] as f32, sz[1] as f32);
    let uv = if src[2] <= 0.0 || src[3] <= 0.0 || tw <= 0.0 || th <= 0.0 {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
    } else {
        Rect::from_min_max(
            pos2(src[0] / tw, src[1] / th),
            pos2((src[0] + src[2]) / tw, (src[1] + src[3]) / th),
        )
    };
    let mut mesh = Mesh::with_texture(tex.id());
    mesh.add_rect_with_uv(dest, uv, Color32::WHITE);
    ui.painter().add(mesh);
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_core::animation::FrameDoc;

    fn clip(durs: &[f32]) -> ClipDoc {
        ClipDoc {
            frames: durs
                .iter()
                .map(|&d| FrameDoc { rect: [0.0, 0.0, 16.0, 16.0], duration: d, texture: None })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn total_and_frame_lookup_respect_variable_timing() {
        let c = clip(&[0.1, 0.2, 0.1]);
        assert!((clip_total(&c) - 0.4).abs() < 1e-6);
        assert_eq!(frame_at(&c, 0.0), 0);
        assert_eq!(frame_at(&c, 0.15), 1);
        assert_eq!(frame_at(&c, 0.25), 1); // inside the long middle frame
        assert_eq!(frame_at(&c, 0.35), 2);
        assert_eq!(frame_at(&c, 99.0), 2); // clamps to last
    }

    #[test]
    fn open_is_clean_and_edits_mark_dirty() {
        let mut ed = AnimationEditor::default();
        let json = r#"{"texture":"x.png","clips":{"Run":{"frames":[{"rect":[0,0,8,8],"duration":0.1}]}}}"#;
        ed.open_doc("a.frames.json", json);
        assert!(ed.open);
        assert_eq!(ed.selected_clip.as_deref(), Some("Run"));
        assert!(!ed.dirty(), "freshly opened file should not read dirty");
        ed.doc.clips.get_mut("Run").unwrap().looped = false;
        assert!(ed.dirty(), "an edit should mark the doc dirty");
    }
}
