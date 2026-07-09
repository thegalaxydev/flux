use flux_core::{Color, InstanceId, UDim2, Value, World};
use eframe::egui::{self, Ui};

use crate::app::{Pending, UiState};
use crate::command::Command;

pub fn show(ui: &mut Ui, world: &World, state: &mut UiState) {
    let Some(id) = state.selection.filter(|&id| world.contains(id)) else {
        ui.weak("Nothing selected");
        return;
    };
    ui.push_id(id, |ui| {
        egui::Grid::new("props")
            .num_columns(2)
            .striped(true)
            .min_col_width(90.0)
            .show(ui, |ui| {
                name_row(ui, world, id, state);
                ui.label("ClassName");
                ui.label(world.class_name(id).unwrap_or_default());
                ui.end_row();
                for (prop, value) in world.props(id) {
                    ui.label(prop);
                    if let Some((new, merge)) = value_widget(ui, world, prop, value) {
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
                let merge =
                    rx.dragged() || ry.dragged() || rx.has_focus() || ry.has_focus();
                (Value::Vec2(glam::Vec2::new(x, y)), merge)
            })
        }
        Value::String(s) => {
            let mut v = s.clone();
            let resp = ui.text_edit_singleline(&mut v);
            resp.changed()
                .then(|| (Value::String(v), resp.has_focus()))
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
                (Value::Color(Color::new(arr[0], arr[1], arr[2], arr[3])), merge)
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
                let a = ui.add(egui::DragValue::new(&mut xs).speed(0.01).prefix(format!("{ax} Scale ")));
                let b = ui.add(egui::DragValue::new(&mut xo).speed(0.5).prefix("Offset "));
                (a, b)
            })
            .inner;
        let ry = ui
            .horizontal(|ui| {
                let a = ui.add(egui::DragValue::new(&mut ys).speed(0.01).prefix(format!("{ay} Scale ")));
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
