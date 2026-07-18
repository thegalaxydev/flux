use std::collections::HashSet;

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Id, LayerId, Order, Pos2, Rect, Sense, Shape,
    Stroke, StrokeKind, Ui, pos2, vec2,
};
use flux_core::gui::{self, Rect2};
use flux_core::{InstanceId, UDim2, Value, World, registry};
use flux_icons::{Icon, IconRole, Icons};
use flux_render::{AssetKind, classify};

use crate::app::{AssetDrag, Pending, RenameState, UiState};
use crate::command::Command;

/// Cursor travel (pixels) required after mouse-down before a drag starts.
/// Below this a press-and-release is treated as a plain click.
const DRAG_THRESHOLD: f32 = 6.0;
/// Seconds a drag must hover a collapsed container before it auto-expands.
const AUTO_EXPAND_DELAY: f64 = 0.6;
/// Distance from the panel edge that triggers auto-scroll during a drag.
const AUTO_SCROLL_MARGIN: f32 = 24.0;
/// Auto-scroll speed in pixels per frame.
const AUTO_SCROLL_SPEED: f32 = 9.0;

const ROW_HEIGHT: f32 = 20.0;
const INDENT: f32 = 14.0;
const CARET_W: f32 = 14.0;
const ICON: f32 = 16.0;
const ACCENT: Color32 = Color32::from_rgb(90, 169, 230);

#[derive(Default)]
pub struct ExplorerState {
    collapsed: HashSet<InstanceId>,
    /// Idle = None. Set on mouse-down over a row: (row, press origin).
    press: Option<(InstanceId, Pos2)>,
    /// True once the press has crossed the drag threshold.
    dragging: bool,
    /// Container being hovered during a drag, and when the hover began.
    auto_expand: Option<(InstanceId, f64)>,
}

struct Row {
    id: InstanceId,
    rect: Rect,
    caret: Option<Rect>,
    has_children: bool,
    open: bool,
    parent: Option<InstanceId>,
    index: usize,
    depth: usize,
    double_clicked: bool,
}

enum DropKind {
    Into,
    Before,
    After,
}

struct Drop {
    parent: InstanceId,
    index: usize,
    kind: DropKind,
    row_id: InstanceId,
}

pub fn show(ui: &mut Ui, world: &World, state: &mut UiState, icons: &Icons) {
    ui.spacing_mut().item_spacing.y = 1.0;
    let mut rows = Vec::new();
    // The root ("game") is implicit — surface its services as the top level so the
    // Game object can't be selected, renamed, reparented, or deleted. Scripts still
    // reach it through the `game` global.
    for child in world.children(world.root()).to_vec() {
        node(ui, world, state, icons, child, 0, &mut rows);
    }
    interact(ui, world, state, &rows);
}

fn node(
    ui: &mut Ui,
    world: &World,
    state: &mut UiState,
    icons: &Icons,
    id: InstanceId,
    depth: usize,
    rows: &mut Vec<Row>,
) {
    let Some(name) = world.name(id).map(str::to_string) else {
        return;
    };
    let children: Vec<InstanceId> = world.children(id).to_vec();
    let has_children = !children.is_empty();
    let open = has_children && !state.explorer.collapsed.contains(&id);

    draw_row(ui, world, state, icons, id, &name, depth, has_children, open, rows);

    if open {
        for child in children {
            node(ui, world, state, icons, child, depth + 1, rows);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    ui: &mut Ui,
    world: &World,
    state: &mut UiState,
    icons: &Icons,
    id: InstanceId,
    name: &str,
    depth: usize,
    has_children: bool,
    open: bool,
    rows: &mut Vec<Row>,
) {
    let full_w = ui.available_width().max(1.0);
    let indent = depth as f32 * INDENT;
    let selected = state.selection == Some(id);

    let renaming = state.rename.as_ref().is_some_and(|r| r.id == id);
    if renaming {
        let ir = ui.allocate_ui_with_layout(
            vec2(full_w, ROW_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(indent + CARET_W + ICON + 8.0);
                let r = state.rename.as_mut().unwrap();
                let resp = ui.text_edit_singleline(&mut r.text);
                if r.focus {
                    resp.request_focus();
                    r.focus = false;
                }
                resp
            },
        );
        let resp = ir.inner;
        let text = state.rename.as_ref().unwrap().text.clone();
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            state.rename = None;
        } else if resp.lost_focus() {
            if !text.is_empty() && text != name {
                state.queue.push(Pending {
                    cmd: Command::rename(id, name.to_string(), text),
                    merge: false,
                });
            }
            state.rename = None;
        }
        rows.push(Row {
            id,
            rect: ir.response.rect,
            caret: None,
            has_children,
            open,
            parent: world.parent(id),
            index: world.child_index(id).unwrap_or(0),
            depth,
            double_clicked: false,
        });
        return;
    }

    let (rect, response) = ui.allocate_exact_size(vec2(full_w, ROW_HEIGHT), Sense::click_and_drag());

    if selected {
        ui.painter()
            .rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
    } else if response.hovered() {
        ui.painter().rect_filled(
            rect,
            2.0,
            ui.visuals().widgets.hovered.bg_fill.gamma_multiply(0.45),
        );
    }

    let caret_rect = has_children.then(|| {
        Rect::from_min_size(pos2(rect.left() + indent, rect.top()), vec2(CARET_W, ROW_HEIGHT))
    });
    if let Some(cr) = caret_rect {
        draw_caret(ui.painter(), cr, open, ui.visuals().weak_text_color());
    }

    let icon_rect = Rect::from_min_size(
        pos2(rect.left() + indent + CARET_W, rect.center().y - ICON / 2.0),
        vec2(ICON, ICON),
    );
    let role = if selected {
        IconRole::Selected
    } else {
        IconRole::Muted
    };
    icons.icon(icon_for(world, id, open)).size(ICON).role(role).paint_at(ui, icon_rect);

    let text_color = if selected {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color()
    };
    ui.painter().text(
        pos2(icon_rect.right() + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(13.0),
        text_color,
    );

    response.context_menu(|ui| context_menu(ui, world, state, id, name));

    // Drop an asset from the file browser onto this row to spawn an instance
    // parented to it: images become Sprites, scripts become Scripts.
    if let Some(payload) = response.dnd_release_payload::<AssetDrag>() {
        drop_asset(state, id, &payload.0);
    }

    rows.push(Row {
        id,
        rect,
        caret: caret_rect,
        has_children,
        open,
        parent: world.parent(id),
        index: world.child_index(id).unwrap_or(0),
        depth,
        double_clicked: response.double_clicked(),
    });
}

fn interact(ui: &mut Ui, world: &World, state: &mut UiState, rows: &[Row]) {
    let (pressed, down, released, pointer, origin, now) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.primary_released(),
            i.pointer.hover_pos(),
            i.pointer.press_origin(),
            i.time,
        )
    });
    let row_at = |p: Pos2| rows.iter().find(|r| r.rect.contains(p));

    // F2 renames the selection (except services/root).
    if ui.input(|i| i.key_pressed(egui::Key::F2)) {
        if let Some(sel) = state.selection {
            if renameable(world, sel) {
                state.rename = Some(RenameState {
                    id: sel,
                    text: world.name(sel).unwrap_or_default().to_string(),
                    focus: true,
                });
            }
        }
    }

    // MouseDown → select immediately (Inspector updates now), enter PendingDrag.
    if pressed {
        if let Some(o) = origin.or(pointer) {
            match row_at(o) {
                Some(row) if row.caret.is_some_and(|c| c.contains(o)) => {
                    toggle(&mut state.explorer.collapsed, row.id);
                    state.explorer.press = None;
                }
                Some(row) => {
                    state.selection = Some(row.id);
                    state.explorer.press = Some((row.id, o));
                    state.explorer.dragging = false;
                }
                None => state.explorer.press = None,
            }
        }
    }

    // Double click: toggle a container, or open a script in the editor.
    for row in rows {
        if !row.double_clicked {
            continue;
        }
        if row.has_children {
            toggle(&mut state.explorer.collapsed, row.id);
        } else if is_scriptable(world, row.id) {
            match world.get_prop(row.id, "SourcePath") {
                // No backing file yet — offer to generate one.
                Some(Value::Asset(p)) if p.is_empty() => state.create_source = Some(row.id),
                Some(Value::Asset(p)) => state.open_script = Some((p.clone(), None)),
                _ => {}
            }
        }
        state.explorer.press = None;
        state.explorer.dragging = false;
    }

    // PendingDrag → Dragging once the cursor moves past the threshold.
    if let (Some((_, o)), Some(p)) = (state.explorer.press, pointer) {
        if down && !state.explorer.dragging && (p - o).length() > DRAG_THRESHOLD {
            state.explorer.dragging = true;
        }
    }

    // Active drag: drop target, indicators, auto-expand, auto-scroll.
    let mut drop = None;
    if state.explorer.dragging {
        ui.ctx().request_repaint();
        if let (Some(p), Some((src, _))) = (pointer, state.explorer.press) {
            if let Some(row) = row_at(p) {
                drop = make_drop(world, src, row, p);
                if let Some(d) = &drop {
                    if matches!(d.kind, DropKind::Into) && row.has_children && !row.open {
                        match state.explorer.auto_expand {
                            Some((eid, t0)) if eid == row.id => {
                                if now - t0 >= AUTO_EXPAND_DELAY {
                                    state.explorer.collapsed.remove(&row.id);
                                    state.explorer.auto_expand = None;
                                }
                            }
                            _ => state.explorer.auto_expand = Some((row.id, now)),
                        }
                    } else {
                        state.explorer.auto_expand = None;
                    }
                } else {
                    state.explorer.auto_expand = None;
                }
            } else {
                state.explorer.auto_expand = None;
            }

            if let Some(d) = &drop {
                draw_drop_indicator(ui, d, rows);
            }
            draw_drag_ghost(ui, world, src, p);

            let clip = ui.clip_rect();
            if p.y > clip.bottom() - AUTO_SCROLL_MARGIN {
                ui.scroll_with_delta(vec2(0.0, -AUTO_SCROLL_SPEED));
            } else if p.y < clip.top() + AUTO_SCROLL_MARGIN {
                ui.scroll_with_delta(vec2(0.0, AUTO_SCROLL_SPEED));
            }
        }
        ui.ctx().set_cursor_icon(if drop.is_some() {
            CursorIcon::Grabbing
        } else {
            CursorIcon::NoDrop
        });
    }

    // MouseUp: perform the move if a drag was active; otherwise selection stands.
    if released {
        if state.explorer.dragging {
            if let (Some((src, _)), Some(d)) = (state.explorer.press, drop) {
                state.queue.push(Pending {
                    cmd: Command::reparent_at(src, d.parent, d.index),
                    merge: false,
                });
                state.selection = Some(src);
            }
        }
        state.explorer.press = None;
        state.explorer.dragging = false;
        state.explorer.auto_expand = None;
    }
}

fn make_drop(world: &World, src: InstanceId, row: &Row, p: Pos2) -> Option<Drop> {
    if !draggable(world, src) {
        return None;
    }
    let rel = ((p.y - row.rect.top()) / row.rect.height()).clamp(0.0, 1.0);
    let kind = if rel < 0.30 {
        DropKind::Before
    } else if rel > 0.70 {
        DropKind::After
    } else {
        DropKind::Into
    };
    match kind {
        DropKind::Into => {
            if is_ancestor_or_self(world, src, row.id) {
                return None;
            }
            Some(Drop {
                parent: row.id,
                index: world.children(row.id).len(),
                kind,
                row_id: row.id,
            })
        }
        DropKind::Before | DropKind::After => {
            let parent = row.parent?;
            if is_ancestor_or_self(world, src, parent) {
                return None;
            }
            let mut index = row.index + matches!(kind, DropKind::After) as usize;
            if world.parent(src) == Some(parent) {
                let current = world.child_index(src).unwrap_or(0);
                if current < index {
                    index -= 1;
                }
            }
            Some(Drop {
                parent,
                index,
                kind,
                row_id: row.id,
            })
        }
    }
}

fn draw_drop_indicator(ui: &Ui, drop: &Drop, rows: &[Row]) {
    let Some(row) = rows.iter().find(|r| r.id == drop.row_id) else {
        return;
    };
    let painter = ui.painter();
    match drop.kind {
        DropKind::Into => {
            painter.rect_stroke(row.rect, 2.0, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
        }
        DropKind::Before | DropKind::After => {
            let y = if matches!(drop.kind, DropKind::After) {
                row.rect.bottom()
            } else {
                row.rect.top()
            };
            let x0 = row.rect.left() + row.depth as f32 * INDENT + CARET_W;
            painter.circle_filled(pos2(x0, y), 3.0, ACCENT);
            painter.hline(x0..=row.rect.right() - 4.0, y, Stroke::new(2.0, ACCENT));
        }
    }
}

fn draw_drag_ghost(ui: &Ui, world: &World, src: InstanceId, p: Pos2) {
    let Some(name) = world.name(src) else { return };
    let painter =
        ui.ctx().layer_painter(LayerId::new(Order::Tooltip, Id::new("flux_explorer_ghost")));
    let galley = painter.layout_no_wrap(name.to_string(), FontId::proportional(13.0), Color32::WHITE);
    let at = p + vec2(14.0, 6.0);
    let bg = Rect::from_min_size(at, galley.size()).expand(4.0);
    painter.rect_filled(bg, 3.0, Color32::from_black_alpha(190));
    painter.galley(at, galley, Color32::WHITE);
}

fn draw_caret(painter: &egui::Painter, rect: Rect, open: bool, color: Color32) {
    let c = rect.center();
    let s = 3.5;
    let pts = if open {
        vec![
            pos2(c.x - s, c.y - s * 0.5),
            pos2(c.x + s, c.y - s * 0.5),
            pos2(c.x, c.y + s * 0.8),
        ]
    } else {
        vec![
            pos2(c.x - s * 0.5, c.y - s),
            pos2(c.x - s * 0.5, c.y + s),
            pos2(c.x + s * 0.8, c.y),
        ]
    };
    painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
}

fn toggle(set: &mut HashSet<InstanceId>, id: InstanceId) {
    if !set.remove(&id) {
        set.insert(id);
    }
}

fn draggable(world: &World, id: InstanceId) -> bool {
    id != world.root()
        && world
            .class_of(id)
            .is_some_and(|c| !registry().info(c).service)
}

fn renameable(world: &World, id: InstanceId) -> bool {
    draggable(world, id)
}

fn is_ancestor_or_self(world: &World, ancestor: InstanceId, node: InstanceId) -> bool {
    let mut cur = Some(node);
    while let Some(c) = cur {
        if c == ancestor {
            return true;
        }
        cur = world.parent(c);
    }
    false
}

fn context_menu(ui: &mut Ui, world: &World, state: &mut UiState, id: InstanceId, name: &str) {
    ui.menu_button("Insert", |ui| {
        for class in registry().creatable_classes() {
            if ui.button(class.name).clicked() {
                state.queue.push(Pending {
                    cmd: Command::create(class.name, id),
                    merge: false,
                });
                state.selection = Some(id);
                ui.close();
            }
        }
    });
    if gui::is_gui_object(world, id) {
        gui_convert_menu(ui, world, state, id);
    }
    // A script/module with no backing file can generate one and pick where to
    // save it; an existing one is opened by double-click instead.
    if needs_source(world, id) {
        if ui.button("Create Source File…").clicked() {
            state.create_source = Some(id);
            ui.close();
        }
    }
    ui.separator();
    if ui.button("Rename").clicked() {
        state.rename = Some(RenameState {
            id,
            text: name.to_string(),
            focus: true,
        });
        ui.close();
    }
    if ui.button("Duplicate").clicked() {
        if let Some(cmd) = Command::duplicate(world, id) {
            state.queue.push(Pending { cmd, merge: false });
        }
        ui.close();
    }
    if ui.button("Delete").clicked() {
        state.queue.push(Pending {
            cmd: Command::delete(id),
            merge: false,
        });
        ui.close();
    }
}

/// Roblox-style "Convert to Offset/Scale" actions for Position and Size. Each
/// keeps the object where it is on screen while re-expressing the UDim2 purely as
/// offset (pixels) or scale (fraction of parent), using the live viewport size.
fn gui_convert_menu(ui: &mut Ui, world: &World, state: &mut UiState, id: InstanceId) {
    let vr = state.viewport_rect;
    let screen = Rect2::new(
        glam::vec2(vr.min.x, vr.min.y),
        glam::vec2(vr.width(), vr.height()),
    );
    let parent = gui::parent_rect(world, id, screen).size;
    let read = |prop: &str| match world.get_prop(id, prop) {
        Some(Value::UDim2(u)) => *u,
        _ => UDim2::default(),
    };
    let mut convert = |ui: &mut Ui, prop: &'static str, to_scale: bool| {
        let cur = read(prop);
        let new = if to_scale {
            gui::to_scale(cur, parent)
        } else {
            gui::to_offset(cur, parent)
        };
        state.queue.push(Pending {
            cmd: Command::set_prop(id, prop, Value::UDim2(cur), Value::UDim2(new)),
            merge: false,
        });
        ui.close();
    };
    ui.menu_button("Convert", |ui| {
        if ui.button("Position → Offset").clicked() {
            convert(ui, "Position", false);
        }
        if ui.button("Position → Scale").clicked() {
            convert(ui, "Position", true);
        }
        if ui.button("Size → Offset").clicked() {
            convert(ui, "Size", false);
        }
        if ui.button("Size → Scale").clicked() {
            convert(ui, "Size", true);
        }
    });
}

/// Create an instance from a dropped asset, parented to `target`.
fn drop_asset(state: &mut UiState, target: InstanceId, rel: &str) {
    let file = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
    let (class, prop) = match classify(file, false) {
        AssetKind::Image => ("Sprite", "Texture"),
        AssetKind::LuaModule => ("Module", "SourcePath"),
        AssetKind::LuaScript | AssetKind::Script => ("Script", "SourcePath"),
        _ => return,
    };
    state.queue.push(Pending {
        cmd: Command::create_with(class, target, vec![(prop, Value::Asset(rel.to_string()))]),
        merge: false,
    });
    state.selection = Some(target);
}

/// Classes whose source lives in an external `.luau` file (`SourcePath`).
fn is_scriptable(world: &World, id: InstanceId) -> bool {
    matches!(world.class_name(id), Some("Script") | Some("Module"))
}

/// A scriptable instance whose `SourcePath` is still empty.
fn needs_source(world: &World, id: InstanceId) -> bool {
    is_scriptable(world, id)
        && matches!(world.get_prop(id, "SourcePath"), Some(Value::Asset(p)) if p.is_empty())
}

fn icon_for(world: &World, id: InstanceId, open: bool) -> Icon {
    match world.class_name(id).unwrap_or("Instance") {
        "Game" => Icon::Project,
        "Workspace" => Icon::World,
        "Storage" => Icon::Package,
        "Scripts" => Icon::Scripting,
        "Folder" => {
            if open {
                Icon::FolderOpen
            } else {
                Icon::Folder
            }
        }
        "Camera2D" => Icon::Camera,
        "Node2D" => Icon::Component,
        "Sprite" => Icon::Sprite,
        "AnimationPlayer" => Icon::Animation,
        "Script" => Icon::Script,
        "Module" => Icon::LuaScript,
        "Gui" => Icon::Ui,
        "Frame" => Icon::Ui,
        "ImageFrame" => Icon::Texture,
        "Label" => Icon::Font,
        "Button" => Icon::Ui,
        _ => Icon::Entity,
    }
}
