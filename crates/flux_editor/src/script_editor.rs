use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align2, Color32, FontId, Key, Sense, TextBuffer, TextEdit, TextFormat, Ui,
    text::LayoutJob, vec2,
};
use flux_icons::{Icon, Icons};

const MIN_FONT: f32 = 9.0;
const MAX_FONT: f32 = 28.0;

pub struct ScriptEditor {
    pub tabs: Vec<ScriptTab>,
    pub active: ActiveTab,
    pub font_size: f32,
    pub find: FindState,
    pub pending_close: Option<usize>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ActiveTab {
    Scene,
    Script(usize),
}

#[derive(Default)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    pub from: usize,
    pub focus: bool,
}

pub struct ScriptTab {
    pub rel: String,
    pub abs: PathBuf,
    pub name: String,
    pub buffer: String,
    pub saved: String,
    pub goto_line: Option<usize>,
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

    pub fn open(&mut self, rel: &str, root: &Path, line: Option<usize>) {
        if let Some(idx) = self.tabs.iter().position(|t| t.rel == rel) {
            self.active = ActiveTab::Script(idx);
            if let Some(l) = line {
                self.tabs[idx].goto_line = Some(l);
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
            goto_line: line,
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
            ActiveTab::Scene => None,
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

/// Draws the tab strip (Scene + open scripts). Mutates `active`; queues a close
/// (immediate for clean tabs, deferred via `pending_close` for dirty ones).
pub fn tab_strip(ui: &mut Ui, editor: &mut ScriptEditor, icons: &Icons) {
    ui.horizontal(|ui| {
        let scene_selected = editor.active == ActiveTab::Scene;
        if ui.selectable_label(scene_selected, "🎬 Scene").clicked() {
            editor.active = ActiveTab::Scene;
        }
        let mut close_request = None;
        for i in 0..editor.tabs.len() {
            let tab = &editor.tabs[i];
            let selected = editor.active == ActiveTab::Script(i);
            let label = format!("{}{}", tab.name, if tab.dirty() { " ●" } else { "" });

            // Reserve a slot behind the tab so the label and close button share one
            // background (filled once we know the tab's hover state and bounds).
            let bg = ui.painter().add(egui::Shape::Noop);
            let mut label_clicked = false;
            let inner = ui.horizontal(|ui| {
                ui.add_space(4.0);
                label_clicked = ui
                    .add(egui::Label::new(label).selectable(false).sense(Sense::click()))
                    .clicked();
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
    });
}

pub fn code_area(
    ui: &mut Ui,
    tab: &mut ScriptTab,
    font_size: &mut f32,
    find: &mut FindState,
    icons: &Icons,
) {
    let size = *font_size;
    let font = FontId::monospace(size);
    let row_h = ui.fonts(|f| f.row_height(&font));

    // Header: find bar (when open) + font-size stepper.
    ui.horizontal(|ui| {
        if find.open {
            ui.label("🔍");
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
        });
    });

    let line_count = tab.buffer.matches('\n').count() + 1;
    let digits = line_count.max(1).to_string().len();
    let gutter_w = digits as f32 * size * 0.62 + 12.0;

    ui.horizontal_top(|ui| {
        let (gutter_rect, _) =
            ui.allocate_exact_size(vec2(gutter_w, ui.available_height()), Sense::hover());

        let goto = tab.goto_line.take();
        let mut area = egui::ScrollArea::both().auto_shrink([false, false]);
        if let Some(line) = goto {
            area = area.vertical_scroll_offset((line.saturating_sub(1)) as f32 * row_h);
        }
        let out = area.show(ui, |ui| {
            let mut layouter = |ui: &Ui, buf: &dyn TextBuffer, _wrap: f32| {
                let job = highlight(buf.as_str(), size);
                ui.fonts(|f| f.layout_job(job))
            };
            ui.add(
                TextEdit::multiline(&mut tab.buffer)
                    .font(font.clone())
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(30)
                    .lock_focus(true)
                    .layouter(&mut layouter),
            )
        });

        let offset = out.state.offset.y;
        let painter = ui.painter_at(gutter_rect);
        let color = ui.visuals().weak_text_color();
        for line in 0..line_count {
            let y = gutter_rect.top() + 2.0 + line as f32 * row_h - offset;
            if y + row_h < gutter_rect.top() || y > gutter_rect.bottom() {
                continue;
            }
            painter.text(
                egui::pos2(gutter_rect.right() - 6.0, y),
                Align2::RIGHT_TOP,
                (line + 1).to_string(),
                font.clone(),
                color,
            );
        }
    });
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
        let line = tab.buffer[..pos].matches('\n').count() + 1;
        tab.goto_line = Some(line);
        find.from = pos + needle.len();
    }
}

/// Parses `scripts/foo.luau:42: message` (Luau `@`-named chunks) into (path, line).
pub fn parse_error_location(message: &str) -> Option<(String, usize)> {
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
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let line = digits.parse().ok()?;
    Some((path, line))
}

// --- Luau syntax highlighting -------------------------------------------------

const C_DEFAULT: Color32 = Color32::from_rgb(212, 212, 212);
const C_KEYWORD: Color32 = Color32::from_rgb(197, 134, 192);
const C_STRING: Color32 = Color32::from_rgb(206, 145, 120);
const C_NUMBER: Color32 = Color32::from_rgb(181, 206, 168);
const C_COMMENT: Color32 = Color32::from_rgb(106, 153, 85);
const C_GLOBAL: Color32 = Color32::from_rgb(86, 156, 214);
const C_SERVICE: Color32 = Color32::from_rgb(78, 201, 176);
const C_FUNCTION: Color32 = Color32::from_rgb(220, 220, 170);

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

fn highlight(text: &str, font_size: f32) -> LayoutJob {
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
            seg(&mut job, start..i, C_COMMENT);
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
            seg(&mut job, start..i, C_STRING);
        } else if c == b'[' && next == b'[' {
            let start = i;
            let end = find2(b, i + 2, b']', b']').map(|e| e + 2).unwrap_or(n);
            i = end;
            seg(&mut job, start..i, C_STRING);
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
            seg(&mut job, start..i, C_NUMBER);
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            let color = if KEYWORDS.contains(&word) {
                C_KEYWORD
            } else if SERVICES.contains(&word) {
                C_SERVICE
            } else if GLOBALS.contains(&word) {
                C_GLOBAL
            } else if is_call(b, i) {
                C_FUNCTION
            } else {
                C_DEFAULT
            };
            seg(&mut job, start..i, color);
        } else {
            let start = i;
            i += char_len(text, i);
            while i < n && !is_token_start(b, i) {
                i += char_len(text, i);
            }
            seg(&mut job, start..i, C_DEFAULT);
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
    use super::{highlight, parse_error_location};

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
            let job = highlight(s, 14.0);
            assert_eq!(job.text, s, "coverage mismatch for {s:?}");
        }
    }

    #[test]
    fn parses_luau_error_locations() {
        assert_eq!(
            parse_error_location("runtime error: scripts/movement.luau:42: attempt to index nil"),
            Some(("scripts/movement.luau".to_string(), 42))
        );
        assert_eq!(
            parse_error_location("ui.luau:3: bad"),
            Some(("ui.luau".to_string(), 3))
        );
        assert_eq!(parse_error_location("no location in this message"), None);
    }
}
