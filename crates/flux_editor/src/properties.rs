use std::path::Path;

use eframe::egui::{self, Ui};
use flux_core::animation::AnimationCache;
use flux_core::{AssetType, Color, InstanceId, Rect, UDim2, Value, World, registry};

use crate::app::{Pending, UiState};
use crate::asset_field::{AssetFieldAction, asset_field};
use crate::command::Command;

pub fn show(
    ui: &mut Ui,
    world: &World,
    state: &mut UiState,
    root: Option<&Path>,
    anim_cache: &mut AnimationCache,
) {
    let Some(id) = state.selection.filter(|&id| world.contains(id)) else {
        ui.weak("Nothing selected");
        return;
    };
    let class = world.class_name(id);
    ui.push_id(id, |ui| {
        egui::Grid::new("props")
            .num_columns(2)
            .striped(true)
            .min_col_width(90.0)
            .show(ui, |ui| {
                name_row(ui, world, id, state);
                ui.label("ClassName");
                ui.label(class.unwrap_or_default());
                ui.end_row();
                for (prop, value) in world.props(id) {
                    ui.label(prop);
                    if prop == "Animation" && class == Some("AnimatedSprite") {
                        animation_row(ui, world, id, value, root, anim_cache, state);
                    } else if let Value::Asset(cur) = value {
                        asset_row(ui, world, id, prop, cur, root, state);
                    } else if let Some((new, merge)) = value_widget(ui, world, prop, value) {
                        state.queue.push(Pending {
                            cmd: Command::set_prop(id, prop, value.clone(), new),
                            merge,
                        });
                    }
                    ui.end_row();
                }
            });
    });
}

/// The declared asset kind of `prop` on `id`'s class (defaults to `Any`).
fn asset_type_of(world: &World, id: InstanceId, prop: &str) -> AssetType {
    world
        .class_of(id)
        .map(|c| registry().info(c))
        .and_then(|info| info.props.iter().find(|p| p.name == prop))
        .and_then(|pd| pd.asset)
        .unwrap_or(AssetType::Any)
}

/// A typed asset-reference field: drag-and-drop, clear, and open-in-editor.
fn asset_row(
    ui: &mut Ui,
    world: &World,
    id: InstanceId,
    prop: &'static str,
    cur: &str,
    root: Option<&Path>,
    state: &mut UiState,
) {
    let expected = asset_type_of(world, id, prop);
    let set = |state: &mut UiState, new: String| {
        state.queue.push(Pending {
            cmd: Command::set_prop(id, prop, Value::Asset(cur.to_string()), Value::Asset(new)),
            merge: false,
        });
    };
    match asset_field(ui, cur, expected, root) {
        AssetFieldAction::Assign(path) => set(state, path),
        AssetFieldAction::Clear => set(state, String::new()),
        AssetFieldAction::Open => match expected {
            AssetType::Script => state.open_script = Some((cur.to_string(), None)),
            AssetType::SpriteFrames => state.open_animation = Some(cur.to_string()),
            _ => {}
        },
        AssetFieldAction::None => {}
    }
}

/// `Animation` on an AnimatedSprite: a dropdown validated against the assigned
/// `Frames` library. A still-selected animation that no longer exists is shown
/// as missing rather than silently cleared.
fn animation_row(
    ui: &mut Ui,
    world: &World,
    id: InstanceId,
    value: &Value,
    root: Option<&Path>,
    anim_cache: &mut AnimationCache,
    state: &mut UiState,
) {
    let cur = match value {
        Value::String(s) => s.clone(),
        _ => String::new(),
    };
    let frames = match world.get_prop(id, "Frames") {
        Some(Value::Asset(p)) => p.clone(),
        _ => String::new(),
    };
    let names = root
        .filter(|_| !frames.is_empty())
        .and_then(|r| anim_cache.get(&frames, r))
        .map(|f| f.clip_names())
        .unwrap_or_default();

    // No library assigned/loaded yet — fall back to a plain text field.
    if names.is_empty() {
        let mut v = cur.clone();
        let resp = ui.text_edit_singleline(&mut v);
        if resp.changed() {
            state.queue.push(Pending {
                cmd: Command::set_prop(id, "Animation", value.clone(), Value::String(v)),
                merge: resp.has_focus(),
            });
        }
        return;
    }

    let missing = !cur.is_empty() && !names.contains(&cur);
    let selected = if cur.is_empty() {
        "(none)".to_string()
    } else if missing {
        format!("⚠ {cur}")
    } else {
        cur.clone()
    };
    let mut chosen: Option<String> = None;
    egui::ComboBox::from_id_salt((id, "Animation"))
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for n in &names {
                if ui.selectable_label(&cur == n, n).clicked() {
                    chosen = Some(n.clone());
                }
            }
        });
    if let Some(n) = chosen {
        state.queue.push(Pending {
            cmd: Command::set_prop(id, "Animation", value.clone(), Value::String(n)),
            merge: false,
        });
    }
}

fn name_row(ui: &mut Ui, world: &World, id: InstanceId, state: &mut UiState) {
    ui.label("Name");
    let current = world.name(id).unwrap_or_default().to_string();
    let mut text = current.clone();
    let resp = ui.text_edit_singleline(&mut text);
    if resp.changed() && !text.is_empty() {
        state.queue.push(Pending {
            cmd: Command::rename(id, current, text),
            merge: resp.has_focus(),
        });
    }
    ui.end_row();
}

fn value_widget(ui: &mut Ui, world: &World, prop: &str, value: &Value) -> Option<(Value, bool)> {
    match value {
        Value::UDim2(u) => udim2_widget(ui, prop, *u),
        Value::Bool(b) => {
            let mut v = *b;
            let resp = ui.checkbox(&mut v, "");
            resp.changed().then(|| (Value::Bool(v), false))
        }
        Value::Number(n) => {
            let mut v = *n;
            let resp = ui.add(egui::DragValue::new(&mut v).speed(0.5));
            resp.changed()
                .then(|| (Value::Number(v), resp.dragged() || resp.has_focus()))
        }
        Value::Vec2(vec) => {
            let mut x = vec.x;
            let mut y = vec.y;
            let (rx, ry) = ui
                .horizontal(|ui| {
                    let rx = ui.add(egui::DragValue::new(&mut x).speed(0.5).prefix("x "));
                    let ry = ui.add(egui::DragValue::new(&mut y).speed(0.5).prefix("y "));
                    (rx, ry)
                })
                .inner;
            (rx.changed() || ry.changed()).then(|| {
                let merge = rx.dragged() || ry.dragged() || rx.has_focus() || ry.has_focus();
                (Value::Vec2(glam::Vec2::new(x, y)), merge)
            })
        }
        Value::String(s) => {
            let mut v = s.clone();
            let resp = ui.text_edit_singleline(&mut v);
            resp.changed().then(|| (Value::String(v), resp.has_focus()))
        }
        Value::Asset(s) => {
            let mut v = s.clone();
            let resp = ui.text_edit_singleline(&mut v);
            resp.changed().then(|| (Value::Asset(v), resp.has_focus()))
        }
        Value::Color(c) => {
            let mut arr = [c.r, c.g, c.b, c.a];
            let resp = ui.color_edit_button_rgba_unmultiplied(&mut arr);
            resp.changed().then(|| {
                let merge = ui.input(|i| i.pointer.any_down());
                (
                    Value::Color(Color::new(arr[0], arr[1], arr[2], arr[3])),
                    merge,
                )
            })
        }
        Value::Rect(r) => {
            // SliceMargins is authored as border insets (left/top/right/bottom);
            // every other Rect is a texture region (x/y/w/h).
            let (la, lb, lc, ld) = if prop == "SliceMargins" {
                ("L ", "T ", "R ", "B ")
            } else {
                ("x ", "y ", "w ", "h ")
            };
            let (mut x, mut y, mut w, mut h) = (r.x, r.y, r.w, r.h);
            let rs = ui
                .vertical(|ui| {
                    let top = ui
                        .horizontal(|ui| {
                            let a = ui.add(egui::DragValue::new(&mut x).speed(1.0).prefix(la));
                            let b = ui.add(egui::DragValue::new(&mut y).speed(1.0).prefix(lb));
                            (a, b)
                        })
                        .inner;
                    let bot = ui
                        .horizontal(|ui| {
                            let a = ui.add(egui::DragValue::new(&mut w).speed(1.0).prefix(lc));
                            let b = ui.add(egui::DragValue::new(&mut h).speed(1.0).prefix(ld));
                            (a, b)
                        })
                        .inner;
                    [top.0, top.1, bot.0, bot.1]
                })
                .inner;
            rs.iter().any(|r| r.changed()).then(|| {
                let merge = rs.iter().any(|r| r.dragged() || r.has_focus());
                (Value::Rect(Rect::new(x, y, w, h)), merge)
            })
        }
        Value::InstanceRef(t) => {
            match t {
                None => ui.weak("nil"),
                Some(t) => ui.label(world.name(*t).unwrap_or("<destroyed>")),
            };
            None
        }
    }
}

/// Editor for a UDim2 (Position/Size), exposing all four scale/offset terms.
/// `Size` labels its axes Width/Height; everything else uses X/Y.
fn udim2_widget(ui: &mut Ui, prop: &str, u: UDim2) -> Option<(Value, bool)> {
    let (ax, ay) = if prop == "Size" {
        ("W", "H")
    } else {
        ("X", "Y")
    };
    let mut xs = u.x.scale;
    let mut xo = u.x.offset;
    let mut ys = u.y.scale;
    let mut yo = u.y.offset;

    let resp = ui.vertical(|ui| {
        let rx = ui
            .horizontal(|ui| {
                let a = ui.add(
                    egui::DragValue::new(&mut xs)
                        .speed(0.01)
                        .prefix(format!("{ax} Scale ")),
                );
                let b = ui.add(egui::DragValue::new(&mut xo).speed(0.5).prefix("Offset "));
                (a, b)
            })
            .inner;
        let ry = ui
            .horizontal(|ui| {
                let a = ui.add(
                    egui::DragValue::new(&mut ys)
                        .speed(0.01)
                        .prefix(format!("{ay} Scale ")),
                );
                let b = ui.add(egui::DragValue::new(&mut yo).speed(0.5).prefix("Offset "));
                (a, b)
            })
            .inner;
        [rx.0, rx.1, ry.0, ry.1]
    });
    let rs = resp.inner;
    let changed = rs.iter().any(|r| r.changed());
    changed.then(|| {
        let merge = rs.iter().any(|r| r.dragged() || r.has_focus());
        (Value::UDim2(UDim2::new(xs, xo, ys, yo)), merge)
    })
}
