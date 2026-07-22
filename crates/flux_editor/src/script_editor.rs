use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align2, Color32, Event, FontId, Key, Pos2, Rect, Sense, Stroke, TextBuffer, TextEdit,
    TextFormat, Ui, pos2,
    text::{CCursor, CCursorRange, LayoutJob},
    text_edit::TextEditState,
    vec2,
};
use flux_core::{InstanceId, World};
use flux_icons::{Icon, Icons};

use crate::language::{
    Completion, CompletionKind, Diagnostic, DiagnosticSeverity, SceneResolver, ScriptLanguageService,
};
use crate::settings::SyntaxTheme;

/// Hierarchy-aware completion source over the live world: resolves `script`,
/// `game`, and `workspace` navigations to instances and lists their children.
pub struct SceneNav<'a> {
    pub world: &'a World,
    /// The instance the open script is attached to (what `script` resolves to).
    pub script: Option<InstanceId>,
}

impl SceneResolver for SceneNav<'_> {
    fn children(&self, base: &str) -> Option<Vec<(String, String)>> {
        let inst = resolve_instance(self.world, self.script, base)?;
        Some(
            self.world
                .children(inst)
                .iter()
                .filter_map(|&c| {
                    Some((
                        self.world.name(c)?.to_string(),
                        self.world.class_name(c).unwrap_or("Instance").to_string(),
                    ))
                })
                .collect(),
        )
    }
}

/// Walk a dotted base expression to an instance: a `script`/`game`/`workspace`
/// root, then `.Parent` or `.ChildName` steps.
fn resolve_instance(world: &World, script: Option<InstanceId>, base: &str) -> Option<InstanceId> {
    let mut segs = base.split('.').map(str::trim);
    let mut cur = match segs.next()? {
        "script" => script?,
        "game" => world.root(),
        "workspace" => world.workspace(),
        _ => return None,
    };
    for seg in segs {
        if seg.is_empty() {
            return None;
        }
        cur = if seg == "Parent" {
            world.parent(cur)?
        } else {
            world.find_first_child(cur, seg)?
        };
    }
    Some(cur)
}

const MIN_FONT: f32 = 9.0;
const MAX_FONT: f32 = 28.0;

pub struct ScriptEditor {
    pub tabs: Vec<ScriptTab>,
    pub active: ActiveTab,
    pub font_size: f32,
    pub find: FindState,
    pub pending_close: Option<usize>,
    /// IDE features (completion, hover, signature help, diagnostics).
    pub assist: Assist,
}

/// Language-service state plus the transient UI state of the completion popup.
pub struct Assist {
    pub svc: ScriptLanguageService,
    /// Suggestions currently offered for the active token.
    completions: Vec<Completion>,
    selected: usize,
    /// Byte offset of the token the current suggestions are anchored to; when
    /// the cursor moves to a different token the session resets.
    anchor: Option<usize>,
    /// True after the user pressed Escape to dismiss the current session.
    dismissed: bool,
    /// Whether the popup was shown last frame (drives key interception).
    visible: bool,
    /// Last known 1-based (line, column) of the cursor, for the status readout.
    cursor_lc: (usize, usize),
}

impl Default for Assist {
    fn default() -> Self {
        Self {
            svc: ScriptLanguageService::default(),
            completions: Vec::new(),
            selected: 0,
            anchor: None,
            dismissed: false,
            visible: false,
            cursor_lc: (1, 1),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum ActiveTab {
    Scene,
    Script(usize),
    /// A docked asset editor (its state lives on `EditorApp`, not here).
    Animation,
    TileSet,
    WorldGen,
    Json,
}

/// A non-script editor docked into the central tab strip (tileset / worldgen /
/// generic JSON). The editor's own state lives on `EditorApp`; this is just what
/// [`tab_strip`] needs to draw and select it.
pub struct DockTab {
    pub tab: ActiveTab,
    pub label: String,
    pub icon: Icon,
    pub dirty: bool,
}

#[derive(Default)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    pub from: usize,
    pub focus: bool,
}

/// A 1-based (line, column) source location.
pub type Loc = (usize, usize);

pub struct ScriptTab {
    pub rel: String,
    pub abs: PathBuf,
    pub name: String,
    pub buffer: String,
    pub saved: String,
    /// Pending cursor jump (from go-to-error or find), applied on next draw.
    pub goto: Option<Loc>,
}

impl ScriptTab {
    pub fn dirty(&self) -> bool {
        self.buffer != self.saved
    }
}

impl Default for ScriptEditor {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active: ActiveTab::Scene,
            font_size: 14.0,
            find: FindState::default(),
            pending_close: None,
            assist: Assist::default(),
        }
    }
}

impl ScriptEditor {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn any_dirty(&self) -> bool {
        self.tabs.iter().any(|t| t.dirty())
    }

    pub fn open(&mut self, rel: &str, root: &Path, loc: Option<Loc>) {
        if let Some(idx) = self.tabs.iter().position(|t| t.rel == rel) {
            self.active = ActiveTab::Script(idx);
            if let Some(l) = loc {
                self.tabs[idx].goto = Some(l);
            }
            return;
        }
        let abs = root.join(rel);
        let source = std::fs::read_to_string(&abs).unwrap_or_default();
        let name = Path::new(rel)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| rel.to_string());
        self.tabs.push(ScriptTab {
            rel: rel.to_string(),
            abs,
            name,
            buffer: source.clone(),
            saved: source,
            goto: loc,
        });
        self.active = ActiveTab::Script(self.tabs.len() - 1);
    }

    pub fn close(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        self.active = match self.active {
            ActiveTab::Script(a) if a == idx => {
                if self.tabs.is_empty() {
                    ActiveTab::Scene
                } else {
                    ActiveTab::Script(idx.min(self.tabs.len() - 1))
                }
            }
            ActiveTab::Script(a) if a > idx => ActiveTab::Script(a - 1),
            other => other,
        };
    }

    pub fn save_tab(&mut self, idx: usize) -> Result<(), String> {
        let Some(tab) = self.tabs.get_mut(idx) else {
            return Ok(());
        };
        std::fs::write(&tab.abs, &tab.buffer).map_err(|e| e.to_string())?;
        tab.saved = tab.buffer.clone();
        Ok(())
    }

    pub fn save_all_dirty(&mut self) {
        let dirty: Vec<usize> = (0..self.tabs.len()).filter(|&i| self.tabs[i].dirty()).collect();
        for i in dirty {
            let _ = self.save_tab(i);
        }
    }

    pub fn active_index(&self) -> Option<usize> {
        match self.active {
            ActiveTab::Script(i) => Some(i),
            _ => None,
        }
    }

    pub fn open_find(&mut self) {
        self.find.open = true;
        self.find.focus = true;
    }

    pub fn bump_font(&mut self, delta: f32) {
        self.font_size = (self.font_size + delta).clamp(MIN_FONT, MAX_FONT);
    }
}

/// Draws the tab strip (Scene + open scripts + docked asset editors). Mutates
/// `active`; queues a script close (immediate for clean tabs, deferred via
/// `pending_close` for dirty ones). Returns a docked asset editor whose close
/// button was clicked, for the caller to act on (its state isn't held here).
pub fn tab_strip(
    ui: &mut Ui,
    editor: &mut ScriptEditor,
    icons: &Icons,
    extras: &[DockTab],
) -> Option<ActiveTab> {
    let mut extra_close = None;
    ui.horizontal(|ui| {
        let scene_selected = editor.active == ActiveTab::Scene;
        if ui.selectable_label(scene_selected, "🎬 Scene").clicked() {
            editor.active = ActiveTab::Scene;
        }
        let mut close_request = None;
        for i in 0..editor.tabs.len() {
            let tab = &editor.tabs[i];
            let selected = editor.active == ActiveTab::Script(i);
            let dirty = tab.dirty();

            // Reserve a slot behind the tab so the label and close button share one
            // background (filled once we know the tab's hover state and bounds).
            let bg = ui.painter().add(egui::Shape::Noop);
            let mut label_clicked = false;
            let inner = ui.horizontal(|ui| {
                ui.add_space(4.0);
                label_clicked = ui
                    .add(egui::Label::new(&tab.name).selectable(false).sense(Sense::click()))
                    .clicked();
                // Unsaved-changes dot, drawn with the same lucide pipeline as the
                // rest of the UI (the plain `●` char renders as tofu in egui's font).
                if dirty {
                    icons
                        .icon(Icon::Modified)
                        .size(10.0)
                        .show(ui)
                        .on_hover_text("Unsaved changes");
                }
                if icons
                    .icon(Icon::Close)
                    .size(11.0)
                    .button(ui)
                    .on_hover_text("Close")
                    .clicked()
                {
                    close_request = Some(i);
                }
            });

            let rect = inner.response.rect.expand2(vec2(2.0, 3.0));
            let hovered = ui.rect_contains_pointer(rect);
            let fill = if selected {
                ui.visuals().selection.bg_fill
            } else if hovered {
                ui.visuals().widgets.hovered.bg_fill
            } else {
                Color32::TRANSPARENT
            };
            ui.painter()
                .set(bg, egui::Shape::rect_filled(rect, 4.0, fill));

            if label_clicked {
                editor.active = ActiveTab::Script(i);
            }
        }
        if let Some(i) = close_request {
            if editor.tabs[i].dirty() {
                editor.pending_close = Some(i);
            } else {
                editor.close(i);
            }
        }

        // Docked asset editors (tileset / worldgen / json), drawn like script
        // tabs but keyed by their `ActiveTab` variant rather than an index.
        for ex in extras {
            let selected = editor.active == ex.tab;
            let bg = ui.painter().add(egui::Shape::Noop);
            let mut label_clicked = false;
            let inner = ui.horizontal(|ui| {
                ui.add_space(4.0);
                icons.icon(ex.icon).size(13.0).show(ui);
                label_clicked = ui
                    .add(egui::Label::new(&ex.label).selectable(false).sense(Sense::click()))
                    .clicked();
                if ex.dirty {
                    icons
                        .icon(Icon::Modified)
                        .size(10.0)
                        .show(ui)
                        .on_hover_text("Unsaved changes");
                }
                if icons
                    .icon(Icon::Close)
                    .size(11.0)
                    .button(ui)
                    .on_hover_text("Close")
                    .clicked()
                {
                    extra_close = Some(ex.tab);
                }
            });
            let rect = inner.response.rect.expand2(vec2(2.0, 3.0));
            let hovered = ui.rect_contains_pointer(rect);
            let fill = if selected {
                ui.visuals().selection.bg_fill
            } else if hovered {
                ui.visuals().widgets.hovered.bg_fill
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().set(bg, egui::Shape::rect_filled(rect, 4.0, fill));
            if label_clicked {
                editor.active = ex.tab;
            }
        }
    });
    extra_close
}

#[allow(clippy::too_many_arguments)]
pub fn code_area(
    ui: &mut Ui,
    tab: &mut ScriptTab,
    font_size: &mut f32,
    find: &mut FindState,
    assist: &mut Assist,
    icons: &Icons,
    scene: Option<&dyn SceneResolver>,
    syntax: &SyntaxTheme,
) {
    let size = *font_size;
    let colors = SyntaxColors::from_theme(syntax);
    let font = FontId::monospace(size);
    let row_h = ui.fonts(|f| f.row_height(&font));
    let ctx = ui.ctx().clone();

    // Diagnostics for this buffer (cached by the service; cheap when unchanged).
    let diags: Vec<Diagnostic> = assist.svc.diagnostics(&tab.buffer).to_vec();
    let errors = diags.iter().filter(|d| d.severity == DiagnosticSeverity::Error).count();
    let warnings = diags
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Warning)
        .count();

    // When the completion popup is open, steal navigation keys before the
    // TextEdit consumes them (so arrows/Enter/Tab drive the popup, not the text).
    let nav = if assist.visible { intercept_nav(ui) } else { Nav::default() };

    // Header: find bar (when open) + font stepper + cursor/diagnostic readout.
    ui.horizontal(|ui| {
        if find.open {
            icons.icon(Icon::Search).size(14.0).show(ui);
            let resp = ui.add(TextEdit::singleline(&mut find.query).desired_width(180.0));
            if find.focus {
                resp.request_focus();
                find.focus = false;
            }
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if ui.small_button("Next").clicked() || enter {
                find_next(tab, find);
                find.focus = true;
            }
            if icons.icon(Icon::Close).size(12.0).button(ui).clicked()
                || ui.input(|i| i.key_pressed(Key::Escape))
            {
                find.open = false;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("A+").clicked() {
                *font_size = (*font_size + 1.0).clamp(MIN_FONT, MAX_FONT);
            }
            ui.label(format!("{}px", size as i32));
            if ui.small_button("A-").clicked() {
                *font_size = (*font_size - 1.0).clamp(MIN_FONT, MAX_FONT);
            }
            ui.separator();
            let (line, col) = assist.cursor_lc;
            ui.label(format!("Ln {line}, Col {col}"));
            if errors > 0 {
                ui.colored_label(C_ERR, format!("● {errors}")).on_hover_text("errors");
            }
            if warnings > 0 {
                ui.colored_label(C_WARN, format!("▲ {warnings}")).on_hover_text("warnings");
            }
        });
    });

    let line_count = tab.buffer.matches('\n').count() + 1;
    let digits = line_count.max(1).to_string().len();
    let gutter_w = digits as f32 * size * 0.62 + 12.0;

    // Worst diagnostic severity per (1-based) line, for gutter colouring.
    let mut line_sev: std::collections::HashMap<usize, DiagnosticSeverity> =
        std::collections::HashMap::new();
    for d in &diags {
        line_sev
            .entry(d.start.line)
            .and_modify(|s| {
                if sev_rank(d.severity) > sev_rank(*s) {
                    *s = d.severity;
                }
            })
            .or_insert(d.severity);
    }

    let edit_id = ui.make_persistent_id(("script_code", &tab.rel));
    let mut area_out: Option<AreaOut> = None;

    ui.horizontal_top(|ui| {
        let (gutter_rect, _) =
            ui.allocate_exact_size(vec2(gutter_w, ui.available_height()), Sense::hover());

        let goto = tab.goto.take();
        let mut area = egui::ScrollArea::both().auto_shrink([false, false]);
        if let Some((line, _)) = goto {
            area = area.vertical_scroll_offset((line.saturating_sub(1)) as f32 * row_h);
        }
        let out = area.show(ui, |ui| {
            // Reserve a shape *behind* the text for line/bracket highlights.
            let behind = ui.painter().add(egui::Shape::Noop);

            let mut layouter = |ui: &Ui, buf: &dyn TextBuffer, _wrap: f32| {
                let job = highlight(buf.as_str(), size, &colors);
                ui.fonts(|f| f.layout_job(job))
            };
            let o = TextEdit::multiline(&mut tab.buffer)
                .id(edit_id)
                .font(font.clone())
                .code_editor()
                .desired_width(f32::INFINITY)
                .desired_rows(30)
                .lock_focus(true)
                .layouter(&mut layouter)
                .show(ui);

            // Apply a pending go-to (error/find): move the cursor to (line, col).
            if let Some((line, col)) = goto {
                let target = line_col_to_char(&tab.buffer, line, col);
                let mut st = o.state.clone();
                st.cursor
                    .set_char_range(Some(CCursorRange::one(CCursor::new(target))));
                st.store(ui.ctx(), edit_id);
                ui.ctx().memory_mut(|m| m.request_focus(edit_id));
            }

            let galley = o.galley.clone();
            let gpos = o.galley_pos;
            let cursor_char = o.cursor_range.map(|c| c.primary.index);
            let cursor_rect = o
                .cursor_range
                .map(|c| galley.pos_from_cursor(c.primary).translate(gpos.to_vec2()));
            let hover_char = o
                .response
                .hover_pos()
                .map(|p| galley.cursor_from_pos(p - gpos).index);

            // Line + matching-bracket highlight, drawn behind the text.
            let mut shapes: Vec<egui::Shape> = Vec::new();
            if let Some(rect) = cursor_rect {
                let width = ui.clip_rect().width().max(galley.size().x);
                let full = Rect::from_min_max(
                    pos2(gpos.x, rect.min.y),
                    pos2(gpos.x + width, rect.max.y),
                );
                shapes.push(egui::Shape::rect_filled(
                    full,
                    egui::CornerRadius::ZERO,
                    line_highlight(ui),
                ));
            }
            if let Some(cb) = cursor_char.map(|c| crate::language::byte_of_char(&tab.buffer, c)) {
                if let Some((a, b)) = matching_bracket(&tab.buffer, cb) {
                    for pos in [a, b] {
                        if let Some(r) = glyph_rect(&galley, &tab.buffer, pos) {
                            shapes.push(egui::Shape::rect_stroke(
                                r.translate(gpos.to_vec2()),
                                egui::CornerRadius::same(2),
                                Stroke::new(1.0, ui.visuals().text_color()),
                                egui::StrokeKind::Inside,
                            ));
                        }
                    }
                }
            }
            if !shapes.is_empty() {
                ui.painter().set(behind, egui::Shape::Vec(shapes));
            }

            // Diagnostic underlines, on top (squiggle for error/warn/info,
            // dotted for hints).
            let painter = ui.painter();
            for d in &diags {
                draw_diagnostic_underline(&painter, &galley, gpos, &tab.buffer, d);
            }

            AreaOut { response: o.response, cursor_char, cursor_rect, hover_char }
        });
        area_out = Some(out.inner);

        // Gutter line numbers (coloured where a line has a diagnostic).
        let offset = out.state.offset.y;
        let painter = ui.painter_at(gutter_rect);
        let weak = ui.visuals().weak_text_color();
        for line in 0..line_count {
            let y = gutter_rect.top() + 2.0 + line as f32 * row_h - offset;
            if y + row_h < gutter_rect.top() || y > gutter_rect.bottom() {
                continue;
            }
            let color = match line_sev.get(&(line + 1)) {
                Some(s) => severity_color(*s),
                None => weak,
            };
            painter.text(
                pos2(gutter_rect.right() - 6.0, y),
                Align2::RIGHT_TOP,
                (line + 1).to_string(),
                font.clone(),
                color,
            );
        }
    });

    let Some(area) = area_out else { return };

    if let Some(cc) = area.cursor_char {
        let byte = crate::language::byte_of_char(&tab.buffer, cc);
        assist.cursor_lc = crate::language::line_col(&tab.buffer, byte);
    }

    // --- Completion session -------------------------------------------------
    let mut mutated = false;
    if let Some(cursor_char) = area.cursor_char {
        let cursor_b = crate::language::byte_of_char(&tab.buffer, cursor_char);
        let anchor = token_start(&tab.buffer, cursor_b);
        if assist.anchor != Some(anchor) {
            assist.anchor = Some(anchor);
            assist.selected = 0;
            assist.dismissed = false;
        }
        assist.completions = assist.svc.completions(&tab.buffer, cursor_char, scene);
        if assist.selected >= assist.completions.len() {
            assist.selected = 0;
        }
        let n = assist.completions.len();
        if n > 0 {
            if nav.down {
                assist.selected = (assist.selected + 1) % n;
            }
            if nav.up {
                assist.selected = (assist.selected + n - 1) % n;
            }
        }
        if nav.dismiss {
            assist.dismissed = true;
        }

        let show = n > 0 && !assist.dismissed && area.response.has_focus();
        let clicked = if show {
            let pos = area
                .cursor_rect
                .map(|r| r.left_bottom() + vec2(-4.0, 2.0))
                .unwrap_or(area.response.rect.left_top());
            completion_popup(&ctx, edit_id.with("completions"), pos, &assist.completions, assist.selected, &colors)
        } else {
            None
        };

        let accept = if (nav.accept || nav.commit.is_some()) && show {
            Some(assist.selected)
        } else {
            clicked
        };
        if let Some(i) = accept {
            if let Some(c) = assist.completions.get(i) {
                let mut insert = c.insert.clone();
                if let Some(sep) = nav.commit {
                    insert.push(sep);
                }
                let new_char = accept_completion(&mut tab.buffer, cursor_char, &insert);
                let mut st = TextEditState::load(&ctx, edit_id).unwrap_or_default();
                st.cursor
                    .set_char_range(Some(CCursorRange::one(CCursor::new(new_char))));
                st.store(&ctx, edit_id);
                ctx.memory_mut(|m| m.request_focus(edit_id));
                assist.completions.clear();
                if nav.commit.is_some() {
                    // Committed on `.`/`:` — reopen member suggestions next frame.
                    assist.anchor = None;
                    assist.dismissed = false;
                } else {
                    // Plain accept — keep the popup closed until the token changes.
                    let nb = crate::language::byte_of_char(&tab.buffer, new_char);
                    assist.anchor = Some(token_start(&tab.buffer, nb));
                    assist.dismissed = true;
                }
                mutated = true;
                ctx.request_repaint();
            }
        }
        assist.visible = show && !mutated;
    } else {
        assist.completions.clear();
        assist.visible = false;
        assist.anchor = None;
    }

    // --- Signature help -----------------------------------------------------
    if !mutated {
        if let (Some(cc), Some(rect)) = (area.cursor_char, area.cursor_rect) {
            if let Some(sig) = assist.svc.signature(&tab.buffer, cc) {
                signature_popup(&ctx, edit_id.with("signature"), rect, &sig);
            }
        }
    }

    // --- Hover --------------------------------------------------------------
    // A squiggle takes priority: hovering it shows every overlapping diagnostic.
    // Otherwise fall back to symbol hover (type/doc).
    if !mutated && !assist.visible && area.response.hovered() {
        if let (Some(hc), Some(pos)) = (area.hover_char, area.response.hover_pos()) {
            let byte = crate::language::byte_of_char(&tab.buffer, hc);
            let hovered: Vec<&Diagnostic> =
                diags.iter().filter(|d| d.range().contains(&byte)).collect();
            if !hovered.is_empty() {
                diagnostic_popup(&ctx, pos, &hovered);
            } else if let Some(h) = assist.svc.hover(&tab.buffer, hc) {
                hover_popup(&ctx, pos, &h);
            }
        }
    }
}

/// Result of drawing the code TextEdit, needed for the IDE overlays.
struct AreaOut {
    response: egui::Response,
    cursor_char: Option<usize>,
    cursor_rect: Option<Rect>,
    hover_char: Option<usize>,
}

#[derive(Default)]
struct Nav {
    up: bool,
    down: bool,
    accept: bool,
    dismiss: bool,
    /// A `.`/`:` that should commit the selection and reopen member suggestions.
    commit: Option<char>,
}

/// Consume completion-navigation keys from the input queue so the TextEdit
/// doesn't act on them this frame.
fn intercept_nav(ui: &Ui) -> Nav {
    ui.input_mut(|i| {
        let mut nav = Nav::default();
        i.events.retain(|e| match e {
            Event::Key { key: Key::ArrowDown, pressed: true, .. } => {
                nav.down = true;
                false
            }
            Event::Key { key: Key::ArrowUp, pressed: true, .. } => {
                nav.up = true;
                false
            }
            Event::Key { key: Key::Enter | Key::Tab, pressed: true, .. } => {
                nav.accept = true;
                false
            }
            Event::Key { key: Key::Escape, pressed: true, .. } => {
                nav.dismiss = true;
                false
            }
            Event::Text(t) if t == "." || t == ":" => {
                nav.commit = t.chars().next();
                false
            }
            _ => true,
        });
        nav
    })
}

/// Byte offset of the start of the identifier ending at `cursor`.
fn token_start(src: &str, cursor: usize) -> usize {
    let b = src.as_bytes();
    let mut start = cursor.min(b.len());
    while start > 0 && (b[start - 1].is_ascii_alphanumeric() || b[start - 1] == b'_') {
        start -= 1;
    }
    start
}

/// Replace the identifier prefix ending at `cursor_char` with `insert`; returns
/// the new cursor position (char index).
fn accept_completion(buffer: &mut String, cursor_char: usize, insert: &str) -> usize {
    let cursor_b = crate::language::byte_of_char(buffer, cursor_char);
    let start_b = token_start(buffer, cursor_b);
    buffer.replace_range(start_b..cursor_b, insert);
    crate::language::char_of_byte(buffer, start_b + insert.len())
}

/// Char index of 1-based (line, col) in `src`.
fn line_col_to_char(src: &str, line: usize, col: usize) -> usize {
    let mut cur_line = 1;
    let mut chars = 0usize;
    for ch in src.chars() {
        if cur_line >= line {
            break;
        }
        if ch == '\n' {
            cur_line += 1;
        }
        chars += 1;
    }
    (chars + col.saturating_sub(1)).min(src.chars().count())
}

/// Byte positions of the bracket adjacent to `cursor` and its match, if any.
fn matching_bracket(src: &str, cursor: usize) -> Option<(usize, usize)> {
    let b = src.as_bytes();
    for cand in [cursor.checked_sub(1), (cursor < b.len()).then_some(cursor)]
        .into_iter()
        .flatten()
    {
        if let Some(m) = scan_match(b, cand) {
            return Some((cand.min(m), cand.max(m)));
        }
    }
    None
}

fn scan_match(b: &[u8], i: usize) -> Option<usize> {
    const OPEN: &[u8; 3] = b"([{";
    const CLOSE: &[u8; 3] = b")]}";
    let c = b[i];
    if let Some(k) = OPEN.iter().position(|&x| x == c) {
        let close = CLOSE[k];
        let mut depth = 0i32;
        for (j, &ch) in b.iter().enumerate().skip(i) {
            if ch == c {
                depth += 1;
            } else if ch == close {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
        }
    } else if let Some(k) = CLOSE.iter().position(|&x| x == c) {
        let open = OPEN[k];
        let mut depth = 0i32;
        let mut j = i as isize;
        while j >= 0 {
            let ch = b[j as usize];
            if ch == c {
                depth += 1;
            } else if ch == open {
                depth -= 1;
                if depth == 0 {
                    return Some(j as usize);
                }
            }
            j -= 1;
        }
    }
    None
}

/// Galley-local rect of the single character at byte offset `byte`.
fn glyph_rect(galley: &egui::Galley, src: &str, byte: usize) -> Option<Rect> {
    let c0 = crate::language::char_of_byte(src, byte);
    let r0 = galley.pos_from_cursor(CCursor::new(c0));
    let r1 = galley.pos_from_cursor(CCursor::new(c0 + 1));
    if (r1.min.y - r0.min.y).abs() > 1.0 {
        return Some(r0);
    }
    Some(Rect::from_min_max(r0.min, pos2(r1.max.x, r0.max.y)))
}

fn sev_rank(s: DiagnosticSeverity) -> u8 {
    match s {
        DiagnosticSeverity::Hint => 0,
        DiagnosticSeverity::Information => 1,
        DiagnosticSeverity::Warning => 2,
        DiagnosticSeverity::Error => 3,
    }
}

fn severity_color(s: DiagnosticSeverity) -> Color32 {
    match s {
        DiagnosticSeverity::Error => C_ERR,
        DiagnosticSeverity::Warning => C_WARN,
        DiagnosticSeverity::Information => C_INFO,
        DiagnosticSeverity::Hint => C_HINT,
    }
}

fn draw_diagnostic_underline(
    painter: &egui::Painter,
    galley: &egui::Galley,
    gpos: Pos2,
    src: &str,
    d: &Diagnostic,
) {
    let range = d.range();
    let c0 = crate::language::char_of_byte(src, range.start);
    let c1 = crate::language::char_of_byte(src, range.end);
    let r0 = galley.pos_from_cursor(CCursor::new(c0));
    let r1 = galley.pos_from_cursor(CCursor::new(c1));
    let same_row = (r1.min.y - r0.min.y).abs() < 1.0;
    let x0 = gpos.x + r0.min.x;
    let x1 = (gpos.x + if same_row { r1.max.x } else { r0.max.x }).max(x0 + 4.0);
    let y = gpos.y + r0.max.y - 1.0;
    let color = severity_color(d.severity);
    // Hints get a subtle dotted underline; everything else a squiggle.
    if d.severity == DiagnosticSeverity::Hint {
        draw_dotted(painter, x0, x1, y, color);
    } else {
        draw_squiggle(painter, x0, x1, y, color);
    }
}

fn draw_squiggle(painter: &egui::Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    let mut pts = Vec::new();
    let mut x = x0;
    let mut up = false;
    while x < x1 {
        pts.push(pos2(x, if up { y - 2.0 } else { y }));
        x += 3.0;
        up = !up;
    }
    pts.push(pos2(x1, y));
    if pts.len() >= 2 {
        painter.add(egui::Shape::line(pts, Stroke::new(1.0, color)));
    }
}

fn draw_dotted(painter: &egui::Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    let mut x = x0;
    while x < x1 {
        painter.line_segment([pos2(x, y), pos2((x + 1.5).min(x1), y)], Stroke::new(1.0, color));
        x += 3.0;
    }
}

fn line_highlight(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_white_alpha(10)
    } else {
        Color32::from_black_alpha(12)
    }
}

#[allow(clippy::too_many_arguments)]
fn completion_popup(
    ctx: &egui::Context,
    id: egui::Id,
    pos: Pos2,
    list: &[Completion],
    selected: usize,
    colors: &SyntaxColors,
) -> Option<usize> {
    let mut clicked = None;
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(420.0);
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for (i, c) in list.iter().enumerate() {
                            let job = completion_job(ui, c, colors);
                            let resp = ui.selectable_label(i == selected, job);
                            if resp.clicked() {
                                clicked = Some(i);
                            }
                            if i == selected {
                                resp.scroll_to_me(Some(egui::Align::Center));
                            }
                        }
                    });
                // Documentation for the highlighted suggestion.
                if let Some(doc) = list.get(selected).map(|c| &c.doc).filter(|d| !d.is_empty()) {
                    ui.separator();
                    ui.add(egui::Label::new(egui::RichText::new(doc).size(12.0).weak()));
                }
            });
        });
    clicked
}

fn completion_job(ui: &Ui, c: &Completion, colors: &SyntaxColors) -> LayoutJob {
    let mut job = LayoutJob::default();
    let accent = kind_color(colors, c.kind);
    let strong = ui.visuals().strong_text_color();
    let weak = ui.visuals().weak_text_color();
    job.append(
        &format!("{:<5}", c.kind.glyph()),
        0.0,
        TextFormat { font_id: FontId::monospace(11.0), color: accent, ..Default::default() },
    );
    job.append(
        &c.label,
        0.0,
        TextFormat { font_id: FontId::proportional(13.0), color: strong, ..Default::default() },
    );
    if !c.detail.is_empty() {
        job.append(
            &format!("    {}", c.detail),
            0.0,
            TextFormat { font_id: FontId::proportional(12.0), color: weak, ..Default::default() },
        );
    }
    job
}

fn kind_color(colors: &SyntaxColors, kind: CompletionKind) -> Color32 {
    match kind {
        CompletionKind::Keyword => colors.keyword,
        CompletionKind::Module => colors.service,
        CompletionKind::Function | CompletionKind::Method => colors.function,
        CompletionKind::Event => colors.service,
        CompletionKind::Property => colors.global,
        CompletionKind::Variable => colors.text,
        CompletionKind::Instance => colors.service,
    }
}

fn signature_popup(
    ctx: &egui::Context,
    id: egui::Id,
    cursor_rect: Rect,
    sig: &crate::language::SignatureHelp,
) {
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(cursor_rect.left_top() + vec2(-4.0, -2.0))
        .pivot(Align2::LEFT_BOTTOM)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(460.0);
                let mut job = LayoutJob::default();
                let strong = ui.visuals().strong_text_color();
                let weak = ui.visuals().weak_text_color();
                let base = FontId::monospace(12.0);
                job.append(
                    &format!("{}(", sig.name),
                    0.0,
                    TextFormat { font_id: base.clone(), color: strong, ..Default::default() },
                );
                for (i, p) in sig.params.iter().enumerate() {
                    if i > 0 {
                        job.append(", ", 0.0, fmt(&base, weak));
                    }
                    let active = i == sig.active;
                    let color = if active { C_SIG_ACTIVE } else { weak };
                    let mut f = fmt(&base, color);
                    if active {
                        f.underline = Stroke::new(1.0, C_SIG_ACTIVE);
                    }
                    job.append(p, 0.0, f);
                }
                job.append(")", 0.0, fmt(&base, strong));
                if let Some(ret) = &sig.returns {
                    job.append(&format!(" -> {ret}"), 0.0, fmt(&base, weak));
                }
                ui.label(job);
                if !sig.doc.is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(&sig.doc).weak().size(12.0)));
                }
            });
        });
}

fn hover_popup(ctx: &egui::Context, pos: Pos2, h: &crate::language::Hover) {
    egui::Area::new(egui::Id::new("script_hover"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos + vec2(12.0, 18.0))
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(420.0);
                ui.label(
                    egui::RichText::new(&h.title)
                        .monospace()
                        .color(ui.visuals().strong_text_color()),
                );
                if !h.doc.is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(&h.doc).size(12.0)));
                }
            });
        });
}

/// Tooltip listing every diagnostic under the pointer (a squiggle can carry
/// several overlapping messages).
fn diagnostic_popup(ctx: &egui::Context, pos: Pos2, diags: &[&Diagnostic]) {
    egui::Area::new(egui::Id::new("script_diag_hover"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos + vec2(12.0, 18.0))
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(460.0);
                for (i, d) in diags.iter().enumerate() {
                    if i > 0 {
                        ui.separator();
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(severity_color(d.severity), severity_tag(d.severity));
                        ui.label(&d.message);
                    });
                }
            });
        });
}

fn severity_tag(s: DiagnosticSeverity) -> &'static str {
    match s {
        DiagnosticSeverity::Error => "error",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Information => "info",
        DiagnosticSeverity::Hint => "hint",
    }
}

fn fmt(font: &FontId, color: Color32) -> TextFormat {
    TextFormat { font_id: font.clone(), color, ..Default::default() }
}

fn find_next(tab: &mut ScriptTab, find: &mut FindState) {
    if find.query.is_empty() {
        return;
    }
    let hay = tab.buffer.to_lowercase();
    let needle = find.query.to_lowercase();
    let start = find.from.min(hay.len());
    let hit = hay[start..]
        .find(&needle)
        .map(|p| start + p)
        .or_else(|| hay.find(&needle));
    if let Some(pos) = hit {
        let (line, col) = crate::language::line_col(&tab.buffer, pos);
        tab.goto = Some((line, col));
        find.from = pos + needle.len();
    }
}

/// Parses `scripts/foo.luau:42: message` (or `:42:7:`) into (path, line, col).
pub fn parse_error_location(message: &str) -> Option<(String, usize, usize)> {
    let marker = ".luau:";
    let idx = message.find(marker)?;
    let end = idx + ".luau".len();
    let start = message[..end]
        .rfind(|c: char| c.is_whitespace() || c == '"' || c == '[' || c == '(')
        .map(|i| i + 1)
        .unwrap_or(0);
    let path = message[start..end].to_string();
    if path.is_empty() {
        return None;
    }
    let rest = &message[end + 1..];
    let line_digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let line = line_digits.parse().ok()?;
    // Optional `:col`.
    let after = &rest[line_digits.len()..];
    let col = after
        .strip_prefix(':')
        .map(|s| s.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
        .and_then(|d| d.parse().ok())
        .unwrap_or(1);
    Some((path, line, col))
}

// --- Luau syntax highlighting -------------------------------------------------

/// Resolved syntax colors (from the user's [`SyntaxTheme`] setting) used by the
/// highlighter and completion popup.
#[derive(Clone, Copy)]
pub struct SyntaxColors {
    pub text: Color32,
    pub keyword: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
    pub global: Color32,
    pub service: Color32,
    pub function: Color32,
}

impl SyntaxColors {
    pub fn from_theme(t: &SyntaxTheme) -> Self {
        let c = |a: [u8; 3]| Color32::from_rgb(a[0], a[1], a[2]);
        Self {
            text: c(t.text),
            keyword: c(t.keyword),
            string: c(t.string),
            number: c(t.number),
            comment: c(t.comment),
            global: c(t.global),
            service: c(t.service),
            function: c(t.function),
        }
    }
}

impl Default for SyntaxColors {
    fn default() -> Self {
        Self::from_theme(&SyntaxTheme::default())
    }
}

/// Accent for the active parameter in signature help (fixed, not themed).
const C_SIG_ACTIVE: Color32 = Color32::from_rgb(220, 220, 170);
const C_ERR: Color32 = Color32::from_rgb(235, 100, 100);
const C_WARN: Color32 = Color32::from_rgb(230, 190, 80);
const C_INFO: Color32 = Color32::from_rgb(90, 160, 230);
const C_HINT: Color32 = Color32::from_rgb(150, 150, 150);

const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while", "continue",
    "type", "typeof", "export",
];

const GLOBALS: &[&str] = &[
    "game", "workspace", "script", "print", "warn", "task", "Vec2", "Color", "UDim", "UDim2",
    "Enum", "Input", "require",
    "pairs", "ipairs", "next", "select", "tostring", "tonumber", "setmetatable", "getmetatable",
    "rawget", "rawset", "rawequal", "rawlen", "assert", "error", "pcall", "xpcall", "unpack",
    "math", "string", "table", "os", "coroutine", "bit32", "utf8", "_G", "_VERSION", "self",
];

const SERVICES: &[&str] = &[
    "Workspace",
    "Storage",
    "Scripts",
    "Gui",
    "DataStoreService",
    "Players",
    "ReplicatedStorage",
    "ServerStorage",
    "ServerScriptService",
    "RunService",
    "UserInputService",
    "TweenService",
    "HttpService",
    "Lighting",
    "SoundService",
    "CollectionService",
];

fn highlight(text: &str, font_size: f32, colors: &SyntaxColors) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let b = text.as_bytes();
    let n = b.len();
    let mut i = 0;

    let seg = |job: &mut LayoutJob, range: std::ops::Range<usize>, color: Color32| {
        job.append(
            &text[range],
            0.0,
            TextFormat {
                font_id: FontId::monospace(font_size),
                color,
                ..Default::default()
            },
        );
    };

    while i < n {
        let c = b[i];
        let next = if i + 1 < n { b[i + 1] } else { 0 };

        if c == b'-' && next == b'-' {
            let start = i;
            if i + 3 < n && b[i + 2] == b'[' && b[i + 3] == b'[' {
                let end = find2(b, i + 4, b']', b']').map(|e| e + 2).unwrap_or(n);
                i = end;
            } else {
                let end = memchr(b, i + 2, b'\n').unwrap_or(n);
                i = end;
            }
            seg(&mut job, start..i, colors.comment);
        } else if c == b'"' || c == b'\'' {
            let start = i;
            i += 1;
            while i < n {
                if b[i] == b'\\' {
                    i += 2;
                } else if b[i] == c {
                    i += 1;
                    break;
                } else {
                    i += char_len(text, i);
                }
            }
            i = i.min(n);
            seg(&mut job, start..i, colors.string);
        } else if c == b'[' && next == b'[' {
            let start = i;
            let end = find2(b, i + 2, b']', b']').map(|e| e + 2).unwrap_or(n);
            i = end;
            seg(&mut job, start..i, colors.string);
        } else if c.is_ascii_digit() || (c == b'.' && next.is_ascii_digit()) {
            let start = i;
            i += 1;
            while i < n {
                let d = b[i];
                let dn = if i + 1 < n { b[i + 1] } else { 0 };
                if d.is_ascii_alphanumeric() || d == b'.' || d == b'_' {
                    i += 1;
                } else if (d == b'e' || d == b'E' || d == b'p' || d == b'P')
                    && (dn == b'+' || dn == b'-')
                {
                    i += 2;
                } else {
                    break;
                }
            }
            seg(&mut job, start..i, colors.number);
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            let color = if KEYWORDS.contains(&word) {
                colors.keyword
            } else if SERVICES.contains(&word) {
                colors.service
            } else if GLOBALS.contains(&word) {
                colors.global
            } else if is_call(b, i) {
                colors.function
            } else {
                colors.text
            };
            seg(&mut job, start..i, color);
        } else {
            let start = i;
            i += char_len(text, i);
            while i < n && !is_token_start(b, i) {
                i += char_len(text, i);
            }
            seg(&mut job, start..i, colors.text);
        }
    }
    job
}

fn is_call(b: &[u8], mut j: usize) -> bool {
    while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
        j += 1;
    }
    j < b.len() && b[j] == b'('
}

fn is_token_start(b: &[u8], i: usize) -> bool {
    let c = b[i];
    let next = if i + 1 < b.len() { b[i + 1] } else { 0 };
    (c == b'-' && next == b'-')
        || c == b'"'
        || c == b'\''
        || (c == b'[' && next == b'[')
        || c.is_ascii_digit()
        || c.is_ascii_alphabetic()
        || c == b'_'
        || (c == b'.' && next.is_ascii_digit())
}

fn char_len(text: &str, i: usize) -> usize {
    text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1)
}

fn memchr(b: &[u8], from: usize, needle: u8) -> Option<usize> {
    (from..b.len()).find(|&i| b[i] == needle)
}

fn find2(b: &[u8], from: usize, a: u8, c: u8) -> Option<usize> {
    (from..b.len().saturating_sub(1)).find(|&i| b[i] == a && b[i + 1] == c)
}

#[cfg(test)]
mod tests {
    use super::{SyntaxColors, highlight, parse_error_location};

    #[test]
    fn highlight_reconstructs_input_exactly() {
        // egui panics if the layouter's galley text differs from the source, so
        // the tokenizer must cover every byte with no gaps or overlaps.
        let samples = [
            "",
            "\n\n",
            "local x = 1 -- comment\nprint(\"hi\")\n",
            "function f(a) return a + 1 end",
            "local s = [[long\nstring]] .. 'two'",
            "-- trailing line comment",
            "game:GetService(\"DataStoreService\").Foo",
            "local n = 0xFF + 1.5e3 - .25",
            "local t = { a = 1, b = 'two', [3] = nil }",
            "local emoji = \"héllo 🌟 wörld\"\n",
        ];
        for s in samples {
            let job = highlight(s, 14.0, &SyntaxColors::default());
            assert_eq!(job.text, s, "coverage mismatch for {s:?}");
        }
    }

    #[test]
    fn parses_luau_error_locations() {
        assert_eq!(
            parse_error_location("runtime error: scripts/movement.luau:42: attempt to index nil"),
            Some(("scripts/movement.luau".to_string(), 42, 1))
        );
        assert_eq!(
            parse_error_location("ui.luau:3: bad"),
            Some(("ui.luau".to_string(), 3, 1))
        );
        assert_eq!(
            parse_error_location("scripts/a.luau:5:12: bad"),
            Some(("scripts/a.luau".to_string(), 5, 12))
        );
        assert_eq!(parse_error_location("no location in this message"), None);
    }
}
