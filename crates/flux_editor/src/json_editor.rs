//! A generic JSON editor: a floating window with a monospace text area, live
//! validity feedback, and a pretty-print button. It's the fallback for any
//! `.json` asset without a dedicated editor — plain data files and plugin
//! catalogs (`*.buildings.json`, `*.recipes.json`, …) — so they never need an
//! external tool. Text is saved verbatim; validation never blocks saving.

use std::path::Path;

use eframe::egui::{self, Color32, Ui};
use flux_icons::{Icon, IconRole, Icons};

#[derive(Default)]
pub struct JsonEditor {
    pub open: bool,
    /// Project-relative path of the open file.
    rel: String,
    text: String,
    /// Contents of the last saved state, for the dirty check.
    saved: String,
    status: String,
}

impl JsonEditor {
    /// Load raw file `text` for editing.
    pub fn open_text(&mut self, rel: &str, text: &str) {
        self.rel = rel.to_string();
        self.text = text.to_string();
        self.saved = text.to_string();
        self.status.clear();
        self.open = true;
    }

    pub fn dirty(&self) -> bool {
        self.text != self.saved
    }

    /// Project-relative path of the open file (for the tab label).
    pub fn rel(&self) -> &str {
        &self.rel
    }

    /// `Ok(())` when the current text parses as JSON, else the parse error.
    fn validate(&self) -> Result<(), String> {
        serde_json::from_str::<serde_json::Value>(&self.text)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub(crate) fn save(&mut self, root: &Path) {
        match std::fs::write(root.join(&self.rel), &self.text) {
            Ok(()) => {
                self.saved = self.text.clone();
                self.status = "Saved".to_string();
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    fn format(&mut self) {
        match serde_json::from_str::<serde_json::Value>(&self.text) {
            Ok(v) => {
                self.text = serde_json::to_string_pretty(&v).unwrap_or_else(|_| self.text.clone());
                self.status = "Formatted".to_string();
            }
            Err(e) => self.status = format!("Can't format invalid JSON: {e}"),
        }
    }

    /// Render into the central panel as a docked tab.
    pub fn dock_ui(&mut self, ui: &mut Ui, root: &Path, icons: &Icons) {
        let valid = self.validate();
        ui.horizontal(|ui| {
            if icons.icon(Icon::Save).size(16.0).button(ui).on_hover_text("Save (Ctrl+S)").clicked() {
                self.save(root);
            }
            if ui.button("Format").clicked() {
                self.format();
            }
            match &valid {
                Ok(()) => {
                    icons.icon(Icon::Success).size(14.0).role(IconRole::Success).show(ui);
                    ui.colored_label(Color32::from_rgb(120, 200, 120), "valid JSON");
                }
                Err(_) => {
                    icons.icon(Icon::Error).size(14.0).role(IconRole::Error).show(ui);
                    ui.colored_label(Color32::from_rgb(230, 120, 120), "invalid JSON");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(&self.status);
            });
        });
        if let Err(e) = &valid {
            ui.colored_label(Color32::from_rgb(210, 140, 60), e);
        }
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.text)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(24),
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_is_clean_and_edits_mark_dirty() {
        let mut ed = JsonEditor::default();
        ed.open_text("data.json", "{\n  \"a\": 1\n}\n");
        assert!(ed.open);
        assert!(!ed.dirty());
        assert!(ed.validate().is_ok());
        ed.text.push_str("trailing");
        assert!(ed.dirty());
    }

    #[test]
    fn validation_and_format() {
        let mut ed = JsonEditor::default();
        ed.open_text("data.json", "{ \"a\":1, }"); // trailing comma = invalid
        assert!(ed.validate().is_err());
        ed.text = "{\"a\":1}".to_string();
        assert!(ed.validate().is_ok());
        ed.format();
        assert!(ed.text.contains("\"a\": 1"), "pretty-printed with a space");
    }
}
