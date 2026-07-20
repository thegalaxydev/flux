//! Reusable typed asset-reference field for the inspector — a Unity-style
//! object field. It shows the current asset, accepts a compatible drag from the
//! Assets panel (validated by asset *type*, not extension), highlights valid vs
//! invalid drops, flags missing references, and offers open/clear buttons.

use std::path::Path;

use eframe::egui::{self, Color32, Sense, Stroke, StrokeKind, Ui, vec2};
use flux_core::AssetType;
use flux_icons::{Icon, Icons};
use flux_render::{AssetKind, classify};

use crate::app::AssetDrag;

/// What the user did with an asset field this frame.
pub enum AssetFieldAction {
    None,
    Assign(String),
    Clear,
    Open,
}

/// Does the asset at `path` satisfy `expected`? Determined by classifying the
/// file, so it validates by asset type rather than raw extension text.
pub fn accepts(expected: AssetType, path: &str) -> bool {
    let kind = classify(basename(path), false);
    match expected {
        AssetType::Texture => kind == AssetKind::Image,
        AssetType::SpriteFrames => kind == AssetKind::Animation,
        AssetType::Script => {
            matches!(
                kind,
                AssetKind::LuaScript | AssetKind::LuaModule | AssetKind::Script
            )
        }
        AssetType::Audio => kind == AssetKind::Audio,
        AssetType::Material => kind == AssetKind::Material,
        AssetType::Scene => matches!(kind, AssetKind::Scene | AssetKind::Prefab),
        AssetType::Any => true,
    }
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

pub fn asset_field(
    ui: &mut Ui,
    value: &str,
    expected: AssetType,
    root: Option<&Path>,
    icons: &Icons,
) -> AssetFieldAction {
    let mut action = AssetFieldAction::None;
    // Right-to-left: the buttons take their natural width first, then the drop
    // box fills *exactly* the remaining width. Sizing the box to
    // `available_width` minus a guess would let the row overflow the panel,
    // which makes a resizable SidePanel grow leftward every frame.
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let has = !value.is_empty();
        // Use the bundled lucide icon set so glyphs always render (a raw "✕"
        // falls back to a missing-glyph box in egui's default font).
        if icons
            .icon(Icon::Remove)
            .size(14.0)
            .disabled(!has)
            .button(ui)
            .on_hover_text("Clear")
            .clicked()
        {
            action = AssetFieldAction::Clear;
        }
        if icons
            .icon(Icon::Open)
            .size(14.0)
            .disabled(!has)
            .button(ui)
            .on_hover_text("Open asset")
            .clicked()
        {
            action = AssetFieldAction::Open;
        }

        let box_w = ui.available_width().max(40.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(box_w, 20.0), Sense::click());

        // Highlight the field while a drag hovers, green if it would be accepted.
        let compatible = resp
            .dnd_hover_payload::<AssetDrag>()
            .map(|p| accepts(expected, &p.0));
        let missing = !value.is_empty() && root.map(|r| !r.join(value).exists()).unwrap_or(false);

        let (bg, border) = match compatible {
            Some(true) => (
                Color32::from_rgb(38, 66, 44),
                Stroke::new(2.0, Color32::from_rgb(120, 210, 130)),
            ),
            Some(false) => (
                Color32::from_rgb(66, 40, 40),
                Stroke::new(2.0, Color32::from_rgb(210, 120, 120)),
            ),
            None => {
                let edge = if missing {
                    Color32::from_rgb(200, 110, 110)
                } else {
                    ui.visuals().widgets.inactive.bg_stroke.color
                };
                (ui.visuals().extreme_bg_color, Stroke::new(1.0, edge))
            }
        };
        ui.painter().rect_filled(rect, 3.0, bg);
        ui.painter()
            .rect_stroke(rect, 3.0, border, StrokeKind::Inside);

        let (text, color) = if value.is_empty() {
            ("None".to_string(), Color32::from_gray(140))
        } else if missing {
            (
                format!("⚠ {}", basename(value)),
                Color32::from_rgb(230, 140, 140),
            )
        } else {
            (basename(value).to_string(), ui.visuals().text_color())
        };
        ui.painter().text(
            rect.left_center() + vec2(6.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::proportional(12.0),
            color,
        );

        // Accept a compatible drop; double-click opens the referenced asset.
        if let Some(p) = resp.dnd_release_payload::<AssetDrag>() {
            if accepts(expected, &p.0) {
                action = AssetFieldAction::Assign(p.0.clone());
            }
        }
        if resp.double_clicked() && !value.is_empty() {
            action = AssetFieldAction::Open;
        }
        let hint = if value.is_empty() {
            "Drag a compatible asset here".to_string()
        } else if missing {
            format!("Missing: {value}")
        } else {
            value.to_string()
        };
        resp.on_hover_text(hint);
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_validates_by_asset_type() {
        assert!(accepts(AssetType::Texture, "art/hero.png"));
        assert!(!accepts(AssetType::Texture, "hero.spriteframes"));
        assert!(accepts(AssetType::SpriteFrames, "anims/hero.spriteframes"));
        assert!(accepts(AssetType::SpriteFrames, "old/hero.frames.json"));
        assert!(!accepts(AssetType::SpriteFrames, "hero.png"));
        assert!(accepts(AssetType::Script, "scripts/main.luau"));
        assert!(accepts(AssetType::Script, "util.module.luau"));
        assert!(!accepts(AssetType::Script, "hero.png"));
        assert!(accepts(AssetType::Any, "anything.xyz"));
    }
}
