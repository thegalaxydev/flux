use std::path::Path;

use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Stroke, Ui};
use flux_core::gui::{self, Rect2};
use flux_core::transform::{self, SpriteXform};
use flux_core::{InstanceId, UDim2, Value, World};
use flux_view::{Camera, TextureCache, draw_scene, game_camera, gui_absolute_rect};

use crate::app::{AssetDrag, Pending, UiState};
use crate::command::Command;

/// Size (px) of the square transform handles.
const HANDLE: f32 = 9.0;
/// How far outside the top edge the rotation handle sits.
const ROTATE_ARM: f32 = 26.0;
/// Move-arrow geometry: dead zone near the centre, shaft length, arrowhead size.
const ARROW_INNER: f32 = 12.0;
const ARROW_LEN: f32 = 52.0;
const ARROW_HEAD: f32 = 9.0;
const ACCENT: Color32 = Color32::from_rgb(255, 200, 60);
const HOVER: Color32 = Color32::from_rgb(120, 180, 240);

/// The active scene-editing tool. Only one is active at a time.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    Move,
    Resize,
    Rotate,
}

/// A resize handle position for a GuiObject, or `Move` for dragging the body.
#[derive(Clone, Copy, PartialEq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Handle {
    const ALL: [Handle; 8] = [
        Handle::TopLeft,
        Handle::Top,
        Handle::TopRight,
        Handle::Right,
        Handle::BottomRight,
        Handle::Bottom,
        Handle::BottomLeft,
        Handle::Left,
    ];

    /// Handle centre as a fraction of the rect (0..1 on each axis).
    fn frac(self) -> egui::Vec2 {
        use Handle::*;
        match self {
            TopLeft => egui::vec2(0.0, 0.0),
            Top => egui::vec2(0.5, 0.0),
            TopRight => egui::vec2(1.0, 0.0),
            Right => egui::vec2(1.0, 0.5),
            BottomRight => egui::vec2(1.0, 1.0),
            Bottom => egui::vec2(0.5, 1.0),
            BottomLeft => egui::vec2(0.0, 1.0),
            Left => egui::vec2(0.0, 0.5),
        }
    }

    fn dir_x(self) -> f32 {
        self.frac().x * 2.0 - 1.0
    }

    fn dir_y(self) -> f32 {
        self.frac().y * 2.0 - 1.0
    }

    fn center(self, r: egui::Rect) -> egui::Pos2 {
        r.min + r.size() * self.frac()
    }
}

/// Live move/resize of a GuiObject (screen-space UDim2 layout).
#[derive(Clone, Copy)]
pub struct GuiOp {
    id: InstanceId,
    handle: Option<Handle>,
    start: Rect2,
    parent: Rect2,
    start_pos: UDim2,
    start_size: UDim2,
    anchor: glam::Vec2,
    start_pointer: glam::Vec2,
}

/// What a sprite drag is doing.
#[derive(Clone, Copy)]
enum SpriteKind {
    /// Free move (dragging the body).
    Move,
    /// Move constrained to a world axis (`(1,0)` or `(0,1)`) via a move arrow.
    MoveAxis(glam::Vec2),
    /// Resize by a handle whose local direction is `dir` (-1/0/1 per axis).
    Resize(glam::Vec2),
    Rotate,
}

/// Live move/resize/rotate of a sprite in world space, captured at drag start.
#[derive(Clone, Copy)]
pub struct SpriteOp {
    id: InstanceId,
    kind: SpriteKind,
    start: SpriteXform,
    /// Pointer position in world space at drag start.
    start_pointer: glam::Vec2,
}

fn to_glam(p: egui::Pos2) -> glam::Vec2 {
    glam::vec2(p.x, p.y)
}

fn screen_rect(rect: egui::Rect) -> Rect2 {
    Rect2::new(glam::vec2(rect.min.x, rect.min.y), glam::vec2(rect.width(), rect.height()))
}

fn udim2_prop(world: &World, id: InstanceId, name: &str) -> UDim2 {
    match world.get_prop(id, name) {
        Some(Value::UDim2(u)) => *u,
        _ => UDim2::default(),
    }
}

pub fn show(
    ui: &mut Ui,
    world: &World,
    state: &mut UiState,
    playing: bool,
    root: Option<&Path>,
    textures: &mut TextureCache,
) {
    let (response, painter) =
        ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
    let rect = response.rect;
    state.viewport_rect = rect;
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(24));
    let origin = rect.center();

    if !playing && response.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            if let Some(pointer) = response.hover_pos() {
                let world_at = (pointer - origin) / state.cam_zoom + state.cam_offset;
                state.cam_zoom = (state.cam_zoom * (1.0 + scroll * 0.001)).clamp(0.05, 20.0);
                state.cam_offset = world_at - (pointer - origin) / state.cam_zoom;
            }
        }
    }

    let camera = if playing {
        game_camera(world).unwrap_or(Camera {
            offset: state.cam_offset,
            zoom: state.cam_zoom,
        })
    } else {
        Camera {
            offset: state.cam_offset,
            zoom: state.cam_zoom,
        }
    };

    // World <-> screen mapping, matching flux_view's draw_scene.
    let to_screen = |w: glam::Vec2| -> Pos2 {
        origin + (egui::vec2(w.x, w.y) - camera.offset) * camera.zoom
    };
    let to_world = |s: Pos2| -> glam::Vec2 {
        let v = (s - origin) / camera.zoom + camera.offset;
        glam::vec2(v.x, v.y)
    };

    if !playing {
        if state.grid_snap {
            draw_grid(&painter, rect, state.grid_size, camera, origin);
        }
        let axis = origin + (egui::Vec2::ZERO - camera.offset) * camera.zoom;
        let stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(45));
        painter.line_segment(
            [egui::pos2(rect.left(), axis.y), egui::pos2(rect.right(), axis.y)],
            stroke,
        );
        painter.line_segment(
            [egui::pos2(axis.x, rect.top()), egui::pos2(axis.x, rect.bottom())],
            stroke,
        );
    }

    let drawn = draw_scene(
        &painter,
        ui.ctx(),
        world,
        textures,
        rect,
        camera,
        root,
        state.selection,
        response.hover_pos(),
        playing,
    );

    // Topmost pickable object under a point: oriented test for sprites, AABB for GUI.
    let pick = |p: Pos2| -> Option<InstanceId> {
        drawn.iter().rev().find_map(|(id, r)| {
            let hit = match SpriteXform::read(world, *id) {
                Some(xf) => xf.contains(to_world(p)),
                None => r.contains(p),
            };
            hit.then_some(*id)
        })
    };

    if response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            state.selection = pick(p);
        }
    }

    if !playing {
        let sel = state.selection;
        let sel_sprite = sel
            .filter(|&id| !gui::is_gui_object(world, id))
            .and_then(|id| SpriteXform::read(world, id).map(|xf| (id, xf)));
        let sel_gui = sel
            .filter(|&id| gui::is_gui_object(world, id))
            .and_then(|id| gui_absolute_rect(world, id, rect).map(|r| (id, r)));

        // A cancelled drag (Escape) is ignored until the mouse is released.
        if state.suppress_drag && !response.dragged() {
            state.suppress_drag = false;
        }

        if response.drag_started() && !state.suppress_drag {
            state.sprite_op = None;
            state.gui_op = None;
            if let Some(p) = response.interact_pointer_pos() {
                // 1. Sprite transform handles (depend on the active tool).
                if let Some((id, xf)) = sel_sprite.filter(|&(id, _)| !is_locked(world, id)) {
                    let corners = xf.corners().map(to_screen);
                    if state.tool == Tool::Rotate && near_rotate_handle(&corners, p) {
                        state.sprite_op = Some(begin_sprite_op(id, xf, SpriteKind::Rotate, p, to_world));
                    } else if state.tool == Tool::Resize {
                        if let Some(dir) = sprite_handle_at(&corners, p) {
                            state.sprite_op =
                                Some(begin_sprite_op(id, xf, SpriteKind::Resize(dir), p, to_world));
                        }
                    } else if state.tool == Tool::Move {
                        if let Some(axis) = move_arrow_at(corners_center(&corners), p) {
                            state.sprite_op =
                                Some(begin_sprite_op(id, xf, SpriteKind::MoveAxis(axis), p, to_world));
                        }
                    }
                }
                // 2. GuiObject resize handles.
                if state.sprite_op.is_none() {
                    if let Some((id, gr)) = sel_gui {
                        if let Some(h) = handle_at(gr, p) {
                            state.gui_op = Some(begin_gui_op(world, id, Some(h), rect, p));
                        }
                    }
                }
                // 3. Otherwise pick the topmost object and drag its body (= move).
                if state.sprite_op.is_none() && state.gui_op.is_none() {
                    match pick(p) {
                        Some(id) if gui::is_gui_object(world, id) => {
                            state.selection = Some(id);
                            state.gui_op = Some(begin_gui_op(world, id, None, rect, p));
                        }
                        Some(id) => {
                            state.selection = Some(id);
                            if let Some(xf) = SpriteXform::read(world, id) {
                                state.sprite_op =
                                    Some(begin_sprite_op(id, xf, SpriteKind::Move, p, to_world));
                            }
                        }
                        None => {}
                    }
                }
            }
        }

        if response.dragged() && !state.suppress_drag {
            let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
            if let Some(op) = state.sprite_op {
                if let Some(p) = response.interact_pointer_pos() {
                    apply_sprite_op(state, op, p, to_world, to_screen, shift, alt);
                }
            } else if let Some(op) = state.gui_op {
                if let Some(p) = response.interact_pointer_pos() {
                    apply_gui_op(state, op, p, shift, alt);
                }
            } else {
                state.cam_offset -= response.drag_delta() / camera.zoom;
            }
        }
        if response.drag_stopped() {
            state.sprite_op = None;
            state.gui_op = None;
            state.suppress_drag = false;
        }

        // Hover outline for the sprite under the cursor (when not dragging).
        if state.sprite_op.is_none() && state.gui_op.is_none() {
            if let Some(p) = response.hover_pos() {
                if let Some(hid) = pick(p) {
                    if Some(hid) != sel {
                        if let Some(xf) = SpriteXform::read(world, hid) {
                            outline(&painter, &xf.corners().map(to_screen), HOVER, 1.0);
                        }
                    }
                }
            }
        }

        // Selection adornments + tool gizmos + cursor.
        if let Some((_, xf)) = sel_sprite {
            let corners = xf.corners().map(to_screen);
            outline(&painter, &corners, ACCENT, 1.5);
            match state.tool {
                Tool::Move => draw_move_arrows(&painter, corners_center(&corners)),
                Tool::Resize => {
                    for (c, _) in sprite_handles(&corners) {
                        handle_dot(&painter, c);
                    }
                }
                Tool::Rotate => {
                    let (arm, knob) = rotate_handle(&corners);
                    painter.line_segment([arm, knob], Stroke::new(1.5, ACCENT));
                    painter.circle_filled(knob, HANDLE * 0.5, ACCENT);
                }
                Tool::Select => {}
            }
            set_transform_cursor(ui, &response, state.tool, &corners);
        } else if let Some((_, gr)) = sel_gui {
            for h in Handle::ALL {
                handle_dot(&painter, h.center(gr));
            }
        }
    }

    if let Some(payload) = response.dnd_release_payload::<AssetDrag>() {
        if let Some(pos) = response.hover_pos() {
            if let Some(id) = pick(pos) {
                if world.get_prop(id, "Texture").is_some() {
                    let old = world
                        .get_prop(id, "Texture")
                        .cloned()
                        .unwrap_or(Value::Asset(String::new()));
                    state.queue.push(Pending {
                        cmd: Command::set_prop(id, "Texture", old, Value::Asset(payload.0.clone())),
                        merge: false,
                    });
                    state.selection = Some(id);
                }
            }
        }
    }

    let label = if playing {
        "Play mode".to_string()
    } else {
        format!("Workspace — edit mode ({:.0}%)", camera.zoom * 100.0)
    };
    painter.text(
        rect.left_top() + egui::vec2(8.0, 8.0),
        egui::Align2::LEFT_TOP,
        label,
        egui::FontId::proportional(12.0),
        egui::Color32::GRAY,
    );
}

// ---- sprite transform ------------------------------------------------------

fn begin_sprite_op(
    id: InstanceId,
    xf: SpriteXform,
    kind: SpriteKind,
    pointer: Pos2,
    to_world: impl Fn(Pos2) -> glam::Vec2,
) -> SpriteOp {
    SpriteOp {
        id,
        kind,
        start: xf,
        start_pointer: to_world(pointer),
    }
}

fn apply_sprite_op(
    state: &mut UiState,
    op: SpriteOp,
    pointer: Pos2,
    to_world: impl Fn(Pos2) -> glam::Vec2,
    to_screen: impl Fn(glam::Vec2) -> Pos2,
    shift: bool,
    alt: bool,
) {
    let s = op.start;
    let entries = match op.kind {
        SpriteKind::Move => {
            let delta = to_world(pointer) - op.start_pointer;
            let mut pos = s.position + delta;
            if state.grid_snap {
                pos = transform::snap_to_grid(pos, state.grid_size);
            }
            vec![("Position", Value::Vec2(s.position), Value::Vec2(pos))]
        }
        SpriteKind::MoveAxis(axis) => {
            // Project the drag onto the chosen world axis so the other stays put.
            let delta = to_world(pointer) - op.start_pointer;
            let moved = s.position + axis * delta.dot(axis);
            let pos = if state.grid_snap {
                let snapped = transform::snap_to_grid(moved, state.grid_size);
                // Snap only along the moved axis; keep the other coordinate exact.
                s.position * (glam::Vec2::ONE - axis) + snapped * axis
            } else {
                moved
            };
            vec![("Position", Value::Vec2(s.position), Value::Vec2(pos))]
        }
        SpriteKind::Resize(dir) => {
            let delta = to_world(pointer) - op.start_pointer;
            let r = transform::resize(&s, dir, delta, shift, alt);
            vec![
                ("Size", Value::Vec2(s.size), Value::Vec2(r.size)),
                ("Position", Value::Vec2(s.position), Value::Vec2(r.position)),
            ]
        }
        SpriteKind::Rotate => {
            let pivot = to_screen(s.position);
            let a0 = angle(pivot, to_screen(op.start_pointer));
            let a1 = angle(pivot, pointer);
            let mut deg = s.rotation + (a1 - a0).to_degrees();
            if shift {
                deg = transform::snap_angle(deg, 15.0);
            }
            vec![(
                "Rotation",
                Value::Number(s.rotation as f64),
                Value::Number(deg as f64),
            )]
        }
    };
    state.queue.push(Pending {
        cmd: Command::set_props(op.id, entries),
        merge: true,
    });
}

fn angle(from: Pos2, to: Pos2) -> f32 {
    (to.y - from.y).atan2(to.x - from.x)
}

fn is_locked(world: &World, id: InstanceId) -> bool {
    matches!(world.get_prop(id, "Locked"), Some(Value::Bool(true)))
}

fn corners_center(c: &[Pos2; 4]) -> Pos2 {
    egui::pos2(
        (c[0].x + c[1].x + c[2].x + c[3].x) * 0.25,
        (c[0].y + c[1].y + c[2].y + c[3].y) * 0.25,
    )
}

/// The four move arrows: (screen direction, constrained world axis).
/// Screen Y grows downward, so "up" is -Y on screen but the +Y world axis.
fn move_arrow_dirs() -> [(egui::Vec2, glam::Vec2); 4] {
    [
        (egui::vec2(1.0, 0.0), glam::vec2(1.0, 0.0)),  // right
        (egui::vec2(-1.0, 0.0), glam::vec2(1.0, 0.0)), // left
        (egui::vec2(0.0, 1.0), glam::vec2(0.0, 1.0)),  // down
        (egui::vec2(0.0, -1.0), glam::vec2(0.0, 1.0)), // up
    ]
}

/// World axis of the move arrow under `p`, if any.
fn move_arrow_at(center: Pos2, p: Pos2) -> Option<glam::Vec2> {
    move_arrow_dirs().into_iter().find_map(|(dir, axis)| {
        let a = center + dir * ARROW_INNER;
        let b = center + dir * ARROW_LEN;
        (dist_to_segment(p, a, b) <= HANDLE * 0.75).then_some(axis)
    })
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 <= f32::EPSILON {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

fn draw_move_arrows(painter: &egui::Painter, center: Pos2) {
    for (dir, _) in move_arrow_dirs() {
        let tip = center + dir * ARROW_LEN;
        painter.line_segment([center + dir * ARROW_INNER, tip], Stroke::new(2.0, ACCENT));
        // Arrowhead: a small triangle pointing along `dir`.
        let perp = egui::vec2(-dir.y, dir.x);
        let base = tip - dir * ARROW_HEAD;
        painter.add(egui::Shape::convex_polygon(
            vec![tip, base + perp * ARROW_HEAD * 0.6, base - perp * ARROW_HEAD * 0.6],
            ACCENT,
            Stroke::NONE,
        ));
    }
}

/// Screen positions + local direction of the 8 resize handles of an oriented box.
/// `corners` is TL, TR, BR, BL.
fn sprite_handles(c: &[Pos2; 4]) -> [(Pos2, glam::Vec2); 8] {
    let mid = |a: Pos2, b: Pos2| a + (b - a) * 0.5;
    [
        (c[0], glam::vec2(-1.0, -1.0)),
        (mid(c[0], c[1]), glam::vec2(0.0, -1.0)),
        (c[1], glam::vec2(1.0, -1.0)),
        (mid(c[1], c[2]), glam::vec2(1.0, 0.0)),
        (c[2], glam::vec2(1.0, 1.0)),
        (mid(c[2], c[3]), glam::vec2(0.0, 1.0)),
        (c[3], glam::vec2(-1.0, 1.0)),
        (mid(c[3], c[0]), glam::vec2(-1.0, 0.0)),
    ]
}

fn sprite_handle_at(corners: &[Pos2; 4], p: Pos2) -> Option<glam::Vec2> {
    sprite_handles(corners).into_iter().find_map(|(c, dir)| {
        Rect::from_center_size(c, egui::vec2(HANDLE + 5.0, HANDLE + 5.0))
            .contains(p)
            .then_some(dir)
    })
}

/// The rotation handle: (top-edge anchor, knob) in screen space.
fn rotate_handle(c: &[Pos2; 4]) -> (Pos2, Pos2) {
    let top = c[0] + (c[1] - c[0]) * 0.5;
    let center = c[0] + (c[2] - c[0]) * 0.5;
    let dir = (top - center).normalized();
    (top, top + dir * ROTATE_ARM)
}

fn near_rotate_handle(corners: &[Pos2; 4], p: Pos2) -> bool {
    let (_, knob) = rotate_handle(corners);
    (p - knob).length() <= HANDLE + 5.0
}

fn outline(painter: &egui::Painter, c: &[Pos2; 4], color: Color32, width: f32) {
    painter.add(egui::Shape::closed_line(c.to_vec(), Stroke::new(width, color)));
}

fn handle_dot(painter: &egui::Painter, c: Pos2) {
    painter.rect_filled(Rect::from_center_size(c, egui::vec2(HANDLE, HANDLE)), 1.0, ACCENT);
}

fn set_transform_cursor(ui: &Ui, response: &egui::Response, tool: Tool, corners: &[Pos2; 4]) {
    let Some(p) = response.hover_pos() else { return };
    let icon = match tool {
        Tool::Rotate if near_rotate_handle(corners, p) => Some(CursorIcon::Crosshair),
        Tool::Resize => sprite_handle_at(corners, p).map(|dir| {
            if dir.x != 0.0 && dir.y != 0.0 {
                CursorIcon::ResizeNwSe
            } else if dir.x != 0.0 {
                CursorIcon::ResizeHorizontal
            } else {
                CursorIcon::ResizeVertical
            }
        }),
        Tool::Move => move_arrow_at(corners_center(corners), p).map(|axis| {
            if axis.x != 0.0 {
                CursorIcon::ResizeHorizontal
            } else {
                CursorIcon::ResizeVertical
            }
        }),
        _ => None,
    };
    if let Some(icon) = icon {
        ui.ctx().set_cursor_icon(icon);
    }
}

fn draw_grid(painter: &egui::Painter, rect: Rect, grid: f32, camera: Camera, origin: Pos2) {
    if grid <= 0.0 || camera.zoom * grid < 4.0 {
        return; // too dense to be useful
    }
    let stroke = Stroke::new(1.0, Color32::from_gray(38));
    let to_world_x = |sx: f32| (sx - origin.x) / camera.zoom + camera.offset.x;
    let to_world_y = |sy: f32| (sy - origin.y) / camera.zoom + camera.offset.y;
    let left = to_world_x(rect.left());
    let right = to_world_x(rect.right());
    let top = to_world_y(rect.top());
    let bottom = to_world_y(rect.bottom());
    let mut x = (left / grid).floor() * grid;
    while x <= right {
        let sx = origin.x + (x - camera.offset.x) * camera.zoom;
        painter.line_segment([egui::pos2(sx, rect.top()), egui::pos2(sx, rect.bottom())], stroke);
        x += grid;
    }
    let mut y = (top / grid).floor() * grid;
    while y <= bottom {
        let sy = origin.y + (y - camera.offset.y) * camera.zoom;
        painter.line_segment([egui::pos2(rect.left(), sy), egui::pos2(rect.right(), sy)], stroke);
        y += grid;
    }
}

// ---- GuiObject transform (screen-space UDim2) ------------------------------

fn begin_gui_op(
    world: &World,
    id: InstanceId,
    handle: Option<Handle>,
    rect: egui::Rect,
    pointer: egui::Pos2,
) -> GuiOp {
    let screen = screen_rect(rect);
    let start = gui::absolute_rect(world, id, screen)
        .unwrap_or(Rect2::new(glam::Vec2::ZERO, glam::Vec2::ZERO));
    GuiOp {
        id,
        handle,
        start,
        parent: gui::parent_rect(world, id, screen),
        start_pos: udim2_prop(world, id, "Position"),
        start_size: udim2_prop(world, id, "Size"),
        anchor: gui::anchor_point(world, id),
        start_pointer: to_glam(pointer),
    }
}

fn apply_gui_op(state: &mut UiState, op: GuiOp, pointer: Pos2, shift: bool, alt: bool) {
    let delta = to_glam(pointer) - op.start_pointer;
    let target = match op.handle {
        None => Rect2::new(op.start.min + delta, op.start.size),
        Some(h) => apply_resize(op.start, h, delta, shift, alt),
    };
    let (new_pos, new_size) =
        gui::solve_offsets(op.start_pos, op.start_size, op.anchor, op.parent, target);
    let mut entries = vec![(
        "Position",
        Value::UDim2(op.start_pos),
        Value::UDim2(new_pos),
    )];
    if op.handle.is_some() {
        entries.push(("Size", Value::UDim2(op.start_size), Value::UDim2(new_size)));
    }
    state.queue.push(Pending {
        cmd: Command::set_props(op.id, entries),
        merge: true,
    });
}

fn handle_at(r: egui::Rect, p: egui::Pos2) -> Option<Handle> {
    Handle::ALL.into_iter().find(|h| {
        egui::Rect::from_center_size(h.center(r), egui::vec2(HANDLE + 4.0, HANDLE + 4.0)).contains(p)
    })
}

/// Recompute a GuiObject absolute rect from a resize-handle drag. `shift`
/// preserves aspect ratio; `alt` resizes symmetrically about the centre.
fn apply_resize(start: Rect2, handle: Handle, delta: glam::Vec2, shift: bool, alt: bool) -> Rect2 {
    let (mut minx, mut maxx) = (start.min.x, start.max().x);
    let (mut miny, mut maxy) = (start.min.y, start.max().y);
    let (dx, dy) = (delta.x, delta.y);
    let (hx, hy) = (handle.dir_x(), handle.dir_y());

    if hx < 0.0 {
        minx += dx;
        if alt {
            maxx -= dx;
        }
    } else if hx > 0.0 {
        maxx += dx;
        if alt {
            minx -= dx;
        }
    }
    if hy < 0.0 {
        miny += dy;
        if alt {
            maxy -= dy;
        }
    } else if hy > 0.0 {
        maxy += dy;
        if alt {
            miny -= dy;
        }
    }

    if shift && start.size.y.abs() > f32::EPSILON && start.size.x.abs() > f32::EPSILON {
        let ratio = start.size.x / start.size.y;
        let corner = hx != 0.0 && hy != 0.0;
        if corner || hx != 0.0 {
            let w = maxx - minx;
            let h = w / ratio;
            let cy = (miny + maxy) * 0.5;
            if corner && !alt {
                if hy < 0.0 {
                    miny = maxy - h;
                } else {
                    maxy = miny + h;
                }
            } else {
                miny = cy - h * 0.5;
                maxy = cy + h * 0.5;
            }
        } else {
            let h = maxy - miny;
            let w = h * ratio;
            let cx = (minx + maxx) * 0.5;
            minx = cx - w * 0.5;
            maxx = cx + w * 0.5;
        }
    }

    Rect2::new(
        glam::vec2(minx.min(maxx), miny.min(maxy)),
        glam::vec2((maxx - minx).abs(), (maxy - miny).abs()),
    )
}
