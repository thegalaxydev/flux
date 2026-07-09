//! Launcher / "recent projects" screen shown when no project is open.
//!
//! The editor opens here instead of auto-loading a hard-coded scene, and it
//! remembers recently opened/saved projects across runs (persisted to the user
//! config directory).

use std::path::{Path, PathBuf};

use eframe::egui::{self, Context};
use flux_icons::{Icon, Icons};

/// What the user picked on the launcher.
pub enum LaunchAction {
    /// Open this scene file.
    Open(PathBuf),
    /// Start a fresh, unsaved scene.
    NewScene,
}

/// Persisted list of recently opened scene files (most recent first).
#[derive(Default)]
pub struct Recents {
    paths: Vec<PathBuf>,
}

impl Recents {
    /// `<config>/Flux/recent.json` (platform config dir).
    fn file() -> Option<PathBuf> {
        let dir = if let Ok(appdata) = std::env::var("APPDATA") {
            PathBuf::from(appdata).join("Flux")
        } else if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg).join("flux")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config").join("flux")
        } else {
            return None;
        };
        Some(dir.join("recent.json"))
    }

    pub fn load() -> Self {
        let paths = Self::file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect();
        Recents { paths }
    }

    /// Move `path` to the front (dedup), cap the list, and persist.
    pub fn push(&mut self, path: PathBuf) {
        self.promote(path);
        self.save();
    }

    /// In-memory promotion (no disk write) — the testable core of [`push`].
    fn promote(&mut self, path: PathBuf) {
        self.paths.retain(|p| p != &path);
        self.paths.insert(0, path);
        self.paths.truncate(12);
    }

    fn save(&self) {
        let Some(file) = Self::file() else { return };
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let list: Vec<String> =
            self.paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        if let Ok(json) = serde_json::to_string_pretty(&list) {
            let _ = std::fs::write(file, json);
        }
    }

    pub fn entries(&self) -> &[PathBuf] {
        &self.paths
    }
}

/// A friendly project name from a scene path: the containing folder, or the file
/// stem for a loose scene file.
fn project_name(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .or_else(|| path.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[derive(Default)]
pub struct Launcher {
    pub error: Option<String>,
}

impl Launcher {
    pub fn ui(&mut self, ctx: &Context, icons: &Icons, recents: &Recents) -> Option<LaunchAction> {
        let mut action = None;
        egui::CentralPanel::default().show(ctx, |ui| {
            // `auto_shrink([false, false])` is essential: the default shrinks the
            // content to its minimum width, which with wrapping labels collapses
            // the whole column to one character per line.
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                // A centred, width-bounded column so long paths have a real width
                // to truncate against and can never collapse.
                let full = ui.available_width();
                let col = full.clamp(340.0, 560.0);
                let pad = ((full - col) * 0.5).max(0.0);
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        ui.set_width(col);
                        self.column(ui, icons, recents, &mut action);
                    });
                });
            });
        });
        action
    }

    fn column(
        &mut self,
        ui: &mut egui::Ui,
        icons: &Icons,
        recents: &Recents,
        action: &mut Option<LaunchAction>,
    ) {
        ui.add_space(36.0);
        ui.horizontal(|ui| {
            icons.icon(Icon::Project).size(30.0).show(ui);
            ui.vertical(|ui| {
                ui.heading(egui::RichText::new("Flux").size(30.0));
                ui.label(egui::RichText::new("Open a project to get started").weak());
            });
        });
        ui.add_space(18.0);

        ui.horizontal(|ui| {
            if ui.button("  New Scene  ").clicked() {
                *action = Some(LaunchAction::NewScene);
            }
            if ui.button("  Open Project…  ").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Flux scene", &["json"])
                    .pick_file()
                {
                    *action = Some(LaunchAction::Open(path));
                }
            }
        });

        if let Some(err) = &self.error {
            ui.add_space(8.0);
            ui.colored_label(egui::Color32::from_rgb(235, 100, 100), err);
        }

        ui.add_space(20.0);
        ui.separator();
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Recent projects").strong());
        ui.add_space(4.0);

        if recents.entries().is_empty() {
            ui.weak("No recent projects yet — open one to see it here.");
            return;
        }

        for path in recents.entries() {
            let exists = path.exists();
            let name = project_name(path);
            let resp = self.recent_row(ui, icons, &name, path, exists);
            if resp.clicked() && exists {
                *action = Some(LaunchAction::Open(path.clone()));
            }
        }
    }

    fn recent_row(
        &self,
        ui: &mut egui::Ui,
        icons: &Icons,
        name: &str,
        path: &Path,
        exists: bool,
    ) -> egui::Response {
        let row_w = ui.available_width();
        // Reserve a slot behind the row for the hover highlight.
        let bg = ui.painter().add(egui::Shape::Noop);
        let inner = ui
            .horizontal(|ui| {
                // Fix the row width so the text column can't grow past it (and so
                // the hover bar spans the full width).
                ui.set_width(row_w);
                ui.add_space(2.0);
                let dim = if exists { 1.0 } else { 0.45 };
                icons.icon(Icon::Scene).size(20.0).opacity(dim).show(ui);
                ui.add_space(4.0);

                // Text column: bound to the remaining width so both lines
                // truncate with an ellipsis instead of wrapping.
                let text_w = (ui.available_width() - 4.0).max(0.0);
                ui.vertical(|ui| {
                    ui.set_width(text_w);
                    let name_text = egui::RichText::new(name).strong();
                    ui.add(egui::Label::new(name_text).truncate());

                    let path_str = path.to_string_lossy();
                    let sub = if exists {
                        egui::RichText::new(path_str).weak().small()
                    } else {
                        egui::RichText::new(format!("{path_str}  (missing)"))
                            .weak()
                            .small()
                            .italics()
                    };
                    ui.add(egui::Label::new(sub).truncate());
                });
            })
            .response;

        let resp = inner.interact(egui::Sense::click());
        if resp.hovered() && exists {
            ui.painter().set(
                bg,
                egui::Shape::rect_filled(
                    resp.rect.expand2(egui::vec2(6.0, 3.0)),
                    4.0,
                    ui.visuals().widgets.hovered.bg_fill,
                ),
            );
        }
        ui.add_space(2.0);
        if exists {
            resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        } else {
            resp
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promote_dedups_and_orders_most_recent_first() {
        let mut r = Recents::default();
        r.promote(PathBuf::from("a"));
        r.promote(PathBuf::from("b"));
        r.promote(PathBuf::from("a")); // re-open a -> moves to front, no dup
        let names: Vec<_> = r.entries().iter().map(|p| p.to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["a", "b"]);
    }

    #[test]
    fn promote_caps_the_list() {
        let mut r = Recents::default();
        for i in 0..20 {
            r.promote(PathBuf::from(format!("p{i}")));
        }
        assert_eq!(r.entries().len(), 12);
        // Most recent stays at the front.
        assert_eq!(r.entries()[0].to_string_lossy(), "p19");
    }

    #[test]
    fn project_name_uses_containing_folder() {
        assert_eq!(project_name(Path::new("projects/dino_run/main.scene.json")), "dino_run");
        assert_eq!(project_name(Path::new("scene.json")), "scene");
    }
}
