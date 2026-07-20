use std::collections::HashSet;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use flux_core::{InstanceId, Value, World, registry};
use flux_icons::{Icon, IconRole, Icons};
use flux_runtime::{DataBackend, InputFrame, LogEntry, LogLevel, Session, SessionOptions};

use crate::command::{Command, History, RemapMap, apply_ephemeral};
use crate::script_editor::{ActiveTab, ScriptEditor};
use crate::textures::TextureCache;

#[derive(Clone)]
pub struct AssetDrag(pub String);

pub struct Pending {
    pub cmd: Command,
    pub merge: bool,
}

pub struct RenameState {
    pub id: InstanceId,
    pub text: String,
    pub focus: bool,
}

pub struct UiState {
    pub selection: Option<InstanceId>,
    pub rename: Option<RenameState>,
    pub queue: Vec<Pending>,
    pub status: String,
    pub cam_offset: egui::Vec2,
    pub cam_zoom: f32,
    pub asset_dir: PathBuf,
    pub gui_op: Option<crate::viewport::GuiOp>,
    pub sprite_op: Option<crate::viewport::SpriteOp>,
    pub tool: crate::viewport::Tool,
    pub grid_snap: bool,
    pub grid_size: f32,
    /// Rotation snap increment (degrees) applied while holding Shift.
    pub angle_snap: f32,
    /// Set when a drag is cancelled (Escape); suppresses further drag handling
    /// until the mouse is released.
    pub suppress_drag: bool,
    pub viewport_rect: egui::Rect,
    pub explorer: crate::explorer::ExplorerState,
    pub open_script: Option<(String, Option<(usize, usize)>)>,
    /// A `*.frames.json` asset to open in the animation editor.
    pub open_animation: Option<String>,
    /// A Script/Module without a backing file whose source should be generated;
    /// drained into a save-file dialog on the next frame.
    pub create_source: Option<InstanceId>,
    /// An asset being renamed in the Assets panel: `(relative path, edit buffer)`.
    pub asset_rename: Option<(String, String)>,
    /// An asset pending delete confirmation (relative path).
    pub asset_delete: Option<String>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            selection: None,
            rename: None,
            queue: Vec::new(),
            status: String::new(),
            cam_offset: egui::Vec2::ZERO,
            cam_zoom: 1.0,
            asset_dir: PathBuf::new(),
            gui_op: None,
            sprite_op: None,
            tool: crate::viewport::Tool::default(),
            grid_snap: false,
            grid_size: 32.0,
            angle_snap: 15.0,
            suppress_drag: false,
            viewport_rect: egui::Rect::NOTHING,
            explorer: crate::explorer::ExplorerState::default(),
            open_script: None,
            open_animation: None,
            create_source: None,
            asset_rename: None,
            asset_delete: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum FileOp {
    New,
    Open,
    Save,
    SaveAs,
    Exit,
    /// Return to the launcher / recent-projects screen.
    Home,
}

pub struct EditorApp {
    world: World,
    ui: UiState,
    history: History,
    icons: Icons,
    textures: TextureCache,
    editor: ScriptEditor,
    anim: crate::animation_editor::AnimationEditor,
    /// Shared clip-library cache for drawing AnimatedSprites in edit mode.
    anim_cache: flux_core::animation::AnimationCache,
    /// Shared tileset cache for drawing Tilemaps in edit mode.
    tile_cache: flux_core::tilemap::TileSetCache,
    /// World-generation config cache used when regenerating Tilemap grids.
    worldgen_cache: flux_core::tilemap::WorldGenCache,
    play: Option<Session>,
    logs: Vec<LogEntry>,
    path: Option<PathBuf>,
    dirty: bool,
    persist_playtest_data: bool,
    confirm: Option<FileOp>,
    script_warn: bool,
    allow_close: bool,
    title: String,
    /// Set when the user asks to return to the launcher; the outer app drains it.
    go_home: bool,
    settings: crate::settings::Settings,
    settings_open: bool,
    /// Applied on the first frame (visuals need the egui context).
    settings_applied: bool,
}

const SC_UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const SC_REDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
const SC_REDO2: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
const SC_SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SC_DUPLICATE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::D);
const SC_PLAY: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F5);
const SC_FIND: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
const SC_FONT_INC: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Equals);
const SC_FONT_DEC: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Minus);

impl EditorApp {
    pub fn new(world: World, path: Option<PathBuf>) -> Self {
        Self {
            world,
            ui: UiState::default(),
            history: History::default(),
            icons: Icons::lucide(),
            textures: TextureCache::default(),
            editor: ScriptEditor::default(),
            anim: crate::animation_editor::AnimationEditor::default(),
            anim_cache: Default::default(),
            tile_cache: Default::default(),
            worldgen_cache: Default::default(),
            play: None,
            logs: Vec::new(),
            path,
            dirty: false,
            persist_playtest_data: false,
            confirm: None,
            script_warn: false,
            allow_close: false,
            title: String::new(),
            go_home: false,
            settings: crate::settings::Settings::load(),
            settings_open: false,
            settings_applied: false,
        }
    }

    /// Push the persisted settings into live state. Visuals need the egui
    /// context, so this runs on the first frame and whenever settings change.
    fn apply_settings(&mut self, ctx: &egui::Context) {
        ctx.set_visuals(if self.settings.theme_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        self.editor.font_size = self.settings.font_size;
        self.ui.grid_size = self.settings.grid_size;
        self.ui.grid_snap = self.settings.grid_snap;
        self.ui.angle_snap = self.settings.angle_snap;
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
        let mut changed = false;
        let mut reset = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                    ui.strong("Appearance");
                    changed |= ui.checkbox(&mut self.settings.theme_dark, "Dark theme").changed();
                    ui.add_space(10.0);

                    ui.strong("Script editor");
                    ui.horizontal(|ui| {
                        ui.label("Font size");
                        changed |= ui
                            .add(egui::Slider::new(&mut self.settings.font_size, 9.0..=28.0))
                            .changed();
                    });
                    ui.add_space(4.0);
                    ui.label("Syntax colors");
                    let s = &mut self.settings.syntax;
                    changed |= color_row(ui, "Text", &mut s.text);
                    changed |= color_row(ui, "Keyword", &mut s.keyword);
                    changed |= color_row(ui, "String", &mut s.string);
                    changed |= color_row(ui, "Number", &mut s.number);
                    changed |= color_row(ui, "Comment", &mut s.comment);
                    changed |= color_row(ui, "Global", &mut s.global);
                    changed |= color_row(ui, "Service / Instance", &mut s.service);
                    changed |= color_row(ui, "Function", &mut s.function);
                    if ui.button("Reset colors to default").clicked() {
                        reset = true;
                    }
                    ui.add_space(10.0);

                    ui.strong("Grid & snapping");
                    changed |= ui
                        .checkbox(&mut self.settings.grid_snap, "Snap to grid by default")
                        .changed();
                    ui.horizontal(|ui| {
                        ui.label("Grid size");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.grid_size)
                                    .range(1.0..=512.0),
                            )
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Rotation snap");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.angle_snap)
                                    .range(1.0..=180.0)
                                    .suffix("°"),
                            )
                            .changed();
                    });
                });
            });
        if reset {
            self.settings.syntax = crate::settings::SyntaxTheme::default();
            changed = true;
        }
        self.settings_open = open;
        if changed {
            self.apply_settings(ctx);
            self.settings.save();
        }
    }

    /// The scene file currently open (if any), for the launcher's recent list.
    pub fn project_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Consume a pending "return to launcher" request.
    pub fn take_go_home(&mut self) -> bool {
        std::mem::take(&mut self.go_home)
    }

    fn playing(&self) -> bool {
        self.play.is_some()
    }

    fn project_root(&self) -> Option<PathBuf> {
        self.path
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
    }

    fn playtest_db_path(&self) -> Option<PathBuf> {
        self.project_root()
            .map(|root| root.join(".flux/data/playtest.sqlite"))
    }

    fn request_play(&mut self) {
        if self.editor.any_dirty() {
            self.script_warn = true;
        } else {
            self.start_play();
        }
    }

    fn save_active_script(&mut self) {
        if let Some(i) = self.editor.active_index() {
            match self.editor.save_tab(i) {
                Ok(()) => self.ui.status = "Script saved".to_string(),
                Err(e) => self.ui.status = format!("Save failed: {e}"),
            }
        }
    }

    fn playtest_backend(&self) -> DataBackend {
        match (self.persist_playtest_data, self.playtest_db_path()) {
            (true, Some(path)) => DataBackend::SqliteFile(path),
            _ => DataBackend::SqliteMemory,
        }
    }

    /// The current scene's path relative to the project root (its file name),
    /// used as `Scene.Path` and for `Scene:Reload`.
    fn scene_rel(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn play_root(&self) -> PathBuf {
        self.project_root()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    }

    fn start_play(&mut self) {
        self.editor.active = ActiveTab::Scene;
        let json = self.world.to_json();
        let root = self.play_root();
        let options = SessionOptions {
            data: self.playtest_backend(),
            scene: self.scene_rel(),
        };
        self.logs.clear();
        match Session::launch(&json, &root, options) {
            Ok(session) => {
                self.logs.extend(session.drain_logs());
                self.ui.selection = None;
                self.play = Some(session);
                self.ui.status = if self.persist_playtest_data {
                    "Playing (persistent data)".to_string()
                } else {
                    "Playing (temporary data)".to_string()
                };
            }
            Err(e) => self.ui.status = format!("Play failed: {e}"),
        }
    }

    /// Switch the running session to another scene (from `Scene:Load`), loaded
    /// from `rel` under the project root.
    fn load_play_scene(&mut self, rel: &str) {
        let root = self.play_root();
        let json = match std::fs::read_to_string(root.join(rel)) {
            Ok(j) => j,
            Err(e) => {
                self.logs.push(LogEntry {
                    level: LogLevel::Error,
                    message: format!("Scene:Load '{rel}': {e}"),
                });
                return;
            }
        };
        let options = SessionOptions {
            data: self.playtest_backend(),
            scene: rel.to_string(),
        };
        match Session::launch(&json, &root, options) {
            Ok(session) => {
                self.logs.extend(session.drain_logs());
                self.ui.selection = None;
                self.play = Some(session);
                self.ui.status = format!("Loaded scene {rel}");
            }
            Err(e) => self.logs.push(LogEntry {
                level: LogLevel::Error,
                message: format!("Scene:Load '{rel}': {e}"),
            }),
        }
    }

    fn clear_playtest_data(&mut self) {
        let Some(path) = self.playtest_db_path() else {
            self.ui.status = "Save the project first to clear playtest data".to_string();
            return;
        };
        let mut removed = false;
        for suffix in ["", "-wal", "-shm"] {
            let p = path.with_file_name(format!(
                "{}{suffix}",
                path.file_name().unwrap().to_string_lossy()
            ));
            if p.exists() {
                match std::fs::remove_file(&p) {
                    Ok(()) => removed = true,
                    Err(e) => {
                        self.ui.status = format!("Could not clear playtest data: {e}");
                        return;
                    }
                }
            }
        }
        self.ui.status = if removed {
            "Playtest data cleared".to_string()
        } else {
            "No playtest data to clear".to_string()
        };
    }

    fn stop_play(&mut self) {
        if let Some(session) = self.play.take() {
            self.logs.extend(session.drain_logs());
        }
        self.ui.selection = None;
        self.ui.status = "Stopped".to_string();
    }

    fn apply(&mut self, cmd: Command, merge: bool) {
        match self.history.apply(&mut self.world, cmd, merge) {
            Ok(map) => {
                self.remap_selection(map);
                self.dirty = true;
            }
            Err(e) => self.ui.status = e.to_string(),
        }
    }

    fn undo_one(&mut self) {
        if !self.history.can_undo() {
            return;
        }
        match self.history.undo(&mut self.world) {
            Ok(map) => {
                self.remap_selection(map);
                self.dirty = true;
            }
            Err(e) => self.ui.status = format!("undo failed: {e}"),
        }
    }

    fn redo_one(&mut self) {
        if !self.history.can_redo() {
            return;
        }
        match self.history.redo(&mut self.world) {
            Ok(map) => {
                self.remap_selection(map);
                self.dirty = true;
            }
            Err(e) => self.ui.status = format!("redo failed: {e}"),
        }
    }

    fn remap_selection(&mut self, map: Option<RemapMap>) {
        if let (Some(map), Some(sel)) = (map, self.ui.selection.as_mut()) {
            if let Some(new) = map.get(sel) {
                *sel = *new;
            }
        }
    }

    fn active_workspace(&self) -> InstanceId {
        match &self.play {
            Some(s) => s.world().borrow().workspace(),
            None => self.world.workspace(),
        }
    }

    fn active_scripts(&self) -> InstanceId {
        let scripts = match &self.play {
            Some(s) => s.world().borrow().scripts(),
            None => self.world.scripts(),
        };
        scripts.unwrap_or_else(|| self.active_workspace())
    }

    fn delete_selected(&mut self) {
        if let Some(id) = self.ui.selection {
            self.ui.queue.push(Pending {
                cmd: Command::delete(id),
                merge: false,
            });
        }
    }

    fn duplicate_selected(&mut self) {
        if let Some(cmd) = self
            .ui
            .selection
            .and_then(|id| Command::duplicate(&self.world, id))
        {
            self.ui.queue.push(Pending { cmd, merge: false });
        }
    }

    fn request(&mut self, ctx: &egui::Context, op: FileOp) {
        if self.dirty && matches!(op, FileOp::New | FileOp::Open | FileOp::Exit | FileOp::Home) {
            self.confirm = Some(op);
        } else {
            self.perform(ctx, op);
        }
    }

    fn perform(&mut self, ctx: &egui::Context, op: FileOp) {
        match op {
            FileOp::New => {
                self.replace_world(World::new(), None);
            }
            FileOp::Open => {
                let Some(path) = rfd::FileDialog::new()
                    .add_filter("Flux scene", &["json"])
                    .pick_file()
                else {
                    return;
                };
                match std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|s| World::from_json(&s).map_err(|e| e.to_string()))
                {
                    Ok(world) => {
                        self.replace_world(world, Some(path));
                        self.ui.status = "Opened".to_string();
                    }
                    Err(e) => self.ui.status = format!("Open failed: {e}"),
                }
            }
            FileOp::Save => {
                self.save(false);
            }
            FileOp::SaveAs => {
                self.save(true);
            }
            FileOp::Exit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            FileOp::Home => {
                self.play = None;
                self.go_home = true;
            }
        }
    }

    fn replace_world(&mut self, world: World, path: Option<PathBuf>) {
        self.play = None;
        self.world = world;
        self.path = path;
        self.dirty = false;
        self.history.clear();
        self.ui = UiState::default();
        self.logs.clear();
        self.textures.clear();
        self.anim_cache.clear();
        self.tile_cache.clear();
        self.worldgen_cache.clear();
        self.editor.clear();
        self.anim = crate::animation_editor::AnimationEditor::default();
    }

    fn save(&mut self, save_as: bool) -> bool {
        let path = match (&self.path, save_as) {
            (Some(p), false) => p.clone(),
            _ => {
                let Some(p) = rfd::FileDialog::new()
                    .add_filter("Flux scene", &["json"])
                    .set_file_name("scene.json")
                    .save_file()
                else {
                    return false;
                };
                p
            }
        };
        match std::fs::write(&path, self.world.to_json()) {
            Ok(()) => {
                self.path = Some(path);
                self.dirty = false;
                self.ui.status = "Saved".to_string();
                true
            }
            Err(e) => {
                self.ui.status = format!("Save failed: {e}");
                false
            }
        }
    }

    /// Generate a backing `.luau` file for a scriptable instance that has none.
    /// Prompts for a save location (rooted at the project), writes a starter
    /// file, points `SourcePath` at it (undoable), and opens it in the editor.
    fn create_source_for(&mut self, id: InstanceId) {
        if self.playing() {
            self.ui.status = "Stop playtesting before creating scripts".to_string();
            return;
        }
        let Some(root) = self.project_root() else {
            self.ui.status = "Save the project before creating scripts".to_string();
            return;
        };
        let is_module = match self.world.class_name(id) {
            Some("Module") => true,
            Some("Script") => false,
            _ => return,
        };
        let name = self.world.name(id).unwrap_or("Script").to_string();

        // Prefer the project's `scripts/` folder as the starting directory.
        let scripts = root.join("scripts");
        let start = if scripts.is_dir() {
            scripts
        } else {
            root.clone()
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Luau source", &["luau", "lua"])
            .set_directory(&start)
            .set_file_name(source_file_name(&name, is_module))
            .save_file()
        else {
            return;
        };

        // Asset paths are stored relative to the project root, forward-slashed.
        let Ok(relative) = path.strip_prefix(&root) else {
            self.ui.status = "Source file must be inside the project folder".to_string();
            return;
        };
        let rel = relative.to_string_lossy().replace('\\', "/");

        // Seed a starter file, but never clobber an existing one — just link it.
        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&path, default_source(&name, is_module)) {
                self.ui.status = format!("Create failed: {e}");
                return;
            }
        }

        let old = self
            .world
            .get_prop(id, "SourcePath")
            .cloned()
            .unwrap_or(Value::Asset(String::new()));
        self.apply(
            Command::set_prop(id, "SourcePath", old, Value::Asset(rel.clone())),
            false,
        );
        self.editor.open(&rel, &root, None);
        self.ui.status = format!("Created {rel}");
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.confirm.is_some() || self.script_warn {
            return;
        }
        let script_active = matches!(self.editor.active, ActiveTab::Script(_));

        // These work even while the code editor has keyboard focus.
        if ctx.input_mut(|i| i.consume_shortcut(&SC_SAVE)) {
            if script_active {
                self.save_active_script();
            } else {
                self.save(false);
            }
        }
        if script_active {
            if ctx.input_mut(|i| i.consume_shortcut(&SC_FIND)) {
                self.editor.open_find();
            }
            if ctx.input_mut(|i| i.consume_shortcut(&SC_FONT_INC)) {
                self.editor.bump_font(1.0);
            }
            if ctx.input_mut(|i| i.consume_shortcut(&SC_FONT_DEC)) {
                self.editor.bump_font(-1.0);
            }
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SC_PLAY)) {
            if self.playing() {
                self.stop_play();
            } else {
                self.request_play();
            }
        }

        // Scene-editing shortcuts are suppressed while typing in the editor.
        if self.playing() || ctx.wants_keyboard_input() {
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SC_UNDO)) {
            self.undo_one();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SC_REDO) || i.consume_shortcut(&SC_REDO2)) {
            self.redo_one();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SC_DUPLICATE)) {
            self.duplicate_selected();
        }
        if ctx.input(|i| i.key_pressed(Key::Delete)) {
            self.delete_selected();
        }

        // Transform tool selection (Unity/Godot-style Q/W/E/R).
        use crate::viewport::Tool;
        for (key, tool) in [
            (Key::Q, Tool::Select),
            (Key::W, Tool::Move),
            (Key::E, Tool::Resize),
            (Key::R, Tool::Rotate),
        ] {
            if ctx.input(|i| i.key_pressed(key)) {
                self.ui.tool = tool;
            }
        }

        // Escape cancels an in-progress transform drag by reverting its single
        // merged undo step; the drag is then ignored until the mouse releases.
        if ctx.input(|i| i.key_pressed(Key::Escape))
            && (self.ui.sprite_op.is_some() || self.ui.gui_op.is_some())
        {
            self.undo_one();
            self.ui.sprite_op = None;
            self.ui.gui_op = None;
            self.ui.suppress_drag = true;
        }
    }

    fn menu_bar(&mut self, ctx: &egui::Context) {
        let playing = self.playing();
        egui::TopBottomPanel::top("menubar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    ui.add_enabled_ui(!playing, |ui| {
                        if ui.button("New").clicked() {
                            self.request(ctx, FileOp::New);
                            ui.close();
                        }
                        if ui.button("Open…").clicked() {
                            self.request(ctx, FileOp::Open);
                            ui.close();
                        }
                        if ui.button("Recent Projects…").clicked() {
                            self.request(ctx, FileOp::Home);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Save\tCtrl+S").clicked() {
                            self.request(ctx, FileOp::Save);
                            ui.close();
                        }
                        if ui.button("Save As…").clicked() {
                            self.request(ctx, FileOp::SaveAs);
                            ui.close();
                        }
                    });
                    ui.separator();
                    if ui.button("Settings…").clicked() {
                        self.settings_open = true;
                        ui.close();
                    }
                    if ui.button("Exit").clicked() {
                        self.request(ctx, FileOp::Exit);
                        ui.close();
                    }
                });
                ui.menu_button("Edit", |ui| {
                    ui.add_enabled_ui(!playing, |ui| {
                        if ui
                            .add_enabled(self.history.can_undo(), egui::Button::new("Undo\tCtrl+Z"))
                            .clicked()
                        {
                            self.undo_one();
                            ui.close();
                        }
                        if ui
                            .add_enabled(self.history.can_redo(), egui::Button::new("Redo\tCtrl+Y"))
                            .clicked()
                        {
                            self.redo_one();
                            ui.close();
                        }
                        ui.separator();
                        let has_sel = self.ui.selection.is_some();
                        if ui
                            .add_enabled(has_sel, egui::Button::new("Duplicate\tCtrl+D"))
                            .clicked()
                        {
                            self.duplicate_selected();
                            ui.close();
                        }
                        if ui
                            .add_enabled(has_sel, egui::Button::new("Delete\tDel"))
                            .clicked()
                        {
                            self.delete_selected();
                            ui.close();
                        }
                    });
                });
                ui.menu_button("Insert", |ui| {
                    for class in registry().creatable_classes() {
                        if ui.button(class.name).clicked() {
                            // With nothing selected, Scripts/Modules default into
                            // the Scripts container; everything else into Workspace.
                            let parent = self.ui.selection.unwrap_or_else(|| {
                                if matches!(class.name, "Script" | "Module") {
                                    self.active_scripts()
                                } else {
                                    self.active_workspace()
                                }
                            });
                            self.ui.queue.push(Pending {
                                cmd: Command::create(class.name, parent),
                                merge: false,
                            });
                            ui.close();
                        }
                    }
                });
                ui.menu_button("Playtest", |ui| {
                    ui.add_enabled_ui(!playing, |ui| {
                        ui.checkbox(&mut self.persist_playtest_data, "Persist playtest data")
                            .on_hover_text(
                                "On: DataStore writes to <project>/.flux/data/playtest.sqlite.\n\
                             Off: a temporary in-memory database, discarded on Stop.",
                            );
                        if ui.button("Clear Playtest Data").clicked() {
                            self.clear_playtest_data();
                            ui.close();
                        }
                    });
                });
            });
        });
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let playing = self.playing();

                if self
                    .icons
                    .icon(Icon::Project)
                    .size(18.0)
                    .disabled(playing)
                    .button(ui)
                    .on_hover_text("Recent projects")
                    .clicked()
                    && !playing
                {
                    self.request(ctx, FileOp::Home);
                }
                ui.separator();

                if self
                    .icons
                    .icon(Icon::New)
                    .size(18.0)
                    .disabled(playing)
                    .button(ui)
                    .on_hover_text("New")
                    .clicked()
                    && !playing
                {
                    self.request(ctx, FileOp::New);
                }
                if self
                    .icons
                    .icon(Icon::Open)
                    .size(18.0)
                    .disabled(playing)
                    .button(ui)
                    .on_hover_text("Open")
                    .clicked()
                    && !playing
                {
                    self.request(ctx, FileOp::Open);
                }
                if self
                    .icons
                    .icon(Icon::Save)
                    .size(18.0)
                    .disabled(playing)
                    .button(ui)
                    .on_hover_text("Save")
                    .clicked()
                    && !playing
                {
                    self.save(false);
                }

                ui.separator();

                if self
                    .icons
                    .icon(Icon::Undo)
                    .size(18.0)
                    .disabled(playing || !self.history.can_undo())
                    .button(ui)
                    .on_hover_text("Undo")
                    .clicked()
                {
                    self.undo_one();
                }
                if self
                    .icons
                    .icon(Icon::Redo)
                    .size(18.0)
                    .disabled(playing || !self.history.can_redo())
                    .button(ui)
                    .on_hover_text("Redo")
                    .clicked()
                {
                    self.redo_one();
                }

                ui.separator();

                if playing {
                    if self
                        .icons
                        .icon(Icon::Stop)
                        .size(18.0)
                        .role(IconRole::Error)
                        .button(ui)
                        .on_hover_text("Stop (F5)")
                        .clicked()
                    {
                        self.stop_play();
                    }
                    self.icons
                        .icon(Icon::Play)
                        .size(16.0)
                        .role(IconRole::Success)
                        .show(ui);
                    ui.colored_label(egui::Color32::from_rgb(120, 220, 120), "PLAYING");
                    ui.weak("edits are not saved");
                } else {
                    if self
                        .icons
                        .icon(Icon::Play)
                        .size(18.0)
                        .role(IconRole::Success)
                        .button(ui)
                        .on_hover_text("Play (F5)")
                        .clicked()
                    {
                        self.request_play();
                    }
                }

                if !playing {
                    ui.separator();
                    use crate::viewport::Tool;
                    for (tool, label, hint) in [
                        (Tool::Select, "Select", "Select (Q)"),
                        (Tool::Move, "Move", "Move (W)"),
                        (Tool::Resize, "Scale", "Resize/Scale (E)"),
                        (Tool::Rotate, "Rotate", "Rotate (R)"),
                    ] {
                        if ui
                            .selectable_label(self.ui.tool == tool, label)
                            .on_hover_text(hint)
                            .clicked()
                        {
                            self.ui.tool = tool;
                        }
                    }
                    ui.separator();
                    if self.ui.tool == Tool::Rotate {
                        // Rotation snap increment (applied while holding Shift).
                        ui.add(
                            egui::DragValue::new(&mut self.ui.angle_snap)
                                .speed(1.0)
                                .range(1.0..=180.0)
                                .suffix("°"),
                        )
                        .on_hover_text("Hold Shift while rotating to snap to this increment");
                    } else {
                        ui.checkbox(&mut self.ui.grid_snap, "Grid")
                            .on_hover_text("Snap Move to the grid");
                        if self.ui.grid_snap {
                            ui.add(
                                egui::DragValue::new(&mut self.ui.grid_size)
                                    .speed(1.0)
                                    .range(1.0..=512.0)
                                    .prefix("size "),
                            );
                        }
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let dark = ui.visuals().dark_mode;
                    let toggle = self
                        .icons
                        .icon(Icon::Light)
                        .size(18.0)
                        .role(if dark {
                            IconRole::Muted
                        } else {
                            IconRole::Accent
                        })
                        .button(ui)
                        .on_hover_text("Toggle light/dark theme");
                    if toggle.clicked() {
                        self.settings.theme_dark = !self.settings.theme_dark;
                        self.apply_settings(ctx);
                        self.settings.save();
                    }
                });
            });
        });
    }

    fn output_panel(&mut self, ctx: &egui::Context) {
        let mut clear = false;
        let mut open_request: Option<(String, Option<(usize, usize)>)> = None;
        egui::TopBottomPanel::bottom("output")
            .resizable(true)
            .default_height(110.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    self.icons.icon(Icon::Console).size(16.0).show(ui);
                    ui.strong("Output");
                    if ui.small_button("Clear").clicked() {
                        clear = true;
                    }
                });
                ui.separator();
                let icons = &self.icons;
                let logs = &self.logs;
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for entry in logs {
                            let (icon, role, color) = match entry.level {
                                LogLevel::Info => {
                                    (Icon::Info, IconRole::Info, egui::Color32::from_gray(200))
                                }
                                LogLevel::Warn => (
                                    Icon::Warning,
                                    IconRole::Warning,
                                    egui::Color32::from_rgb(230, 190, 80),
                                ),
                                LogLevel::Error => (
                                    Icon::Error,
                                    IconRole::Error,
                                    egui::Color32::from_rgb(235, 100, 100),
                                ),
                            };
                            ui.horizontal(|ui| {
                                icons.icon(icon).size(14.0).role(role).show(ui);
                                let location = (entry.level == LogLevel::Error)
                                    .then(|| {
                                        crate::script_editor::parse_error_location(&entry.message)
                                    })
                                    .flatten();
                                if let Some((path, line, col)) = location {
                                    if ui
                                        .add(
                                            egui::Label::new(
                                                egui::RichText::new(&entry.message)
                                                    .color(color)
                                                    .underline(),
                                            )
                                            .sense(egui::Sense::click()),
                                        )
                                        .on_hover_text("Open in Script Editor")
                                        .clicked()
                                    {
                                        open_request = Some((path, Some((line, col))));
                                    }
                                } else {
                                    ui.colored_label(color, &entry.message);
                                }
                            });
                        }
                    });
            });
        if clear {
            self.logs.clear();
        }
        if let Some(req) = open_request {
            self.ui.open_script = Some(req);
        }
    }

    fn asset_browser(&mut self, ctx: &egui::Context) {
        let root = self.project_root();
        let play_rc = self.play.as_ref().map(|s| s.world());
        let Self {
            world,
            ui,
            textures,
            icons,
            ..
        } = &mut *self;
        let guard;
        let active: &World = match &play_rc {
            Some(rc) => {
                guard = rc.borrow();
                &guard
            }
            None => world,
        };
        egui::TopBottomPanel::bottom("assets")
            .resizable(true)
            .default_height(150.0)
            .show(ctx, |panel| {
                crate::assets_panel::show(panel, root.as_deref(), active, ui, textures, icons);
            });
    }

    fn confirm_modal(&mut self, ctx: &egui::Context) {
        let Some(op) = self.confirm else { return };
        egui::Modal::new(egui::Id::new("confirm_unsaved")).show(ctx, |ui| {
            ui.label("This scene has unsaved changes.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    self.confirm = None;
                    if self.save(false) {
                        self.perform(ctx, op);
                    }
                }
                if ui.button("Don't Save").clicked() {
                    self.confirm = None;
                    self.dirty = false;
                    self.perform(ctx, op);
                }
                if ui.button("Cancel").clicked() {
                    self.confirm = None;
                }
            });
        });
    }

    fn script_warn_modal(&mut self, ctx: &egui::Context) {
        if !self.script_warn {
            return;
        }
        egui::Modal::new(egui::Id::new("scripts_unsaved")).show(ctx, |ui| {
            ui.label("Some scripts have unsaved changes. Save before playtesting?");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save & Play").clicked() {
                    self.script_warn = false;
                    self.editor.save_all_dirty();
                    self.start_play();
                }
                if ui.button("Play Anyway").clicked() {
                    self.script_warn = false;
                    self.start_play();
                }
                if ui.button("Cancel").clicked() {
                    self.script_warn = false;
                }
            });
        });
    }

    fn close_tab_modal(&mut self, ctx: &egui::Context) {
        let Some(i) = self.editor.pending_close else {
            return;
        };
        let name = self
            .editor
            .tabs
            .get(i)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        egui::Modal::new(egui::Id::new("close_script_unsaved")).show(ctx, |ui| {
            ui.label(format!("{name} has unsaved changes."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    let _ = self.editor.save_tab(i);
                    self.editor.close(i);
                    self.editor.pending_close = None;
                }
                if ui.button("Don't Save").clicked() {
                    self.editor.close(i);
                    self.editor.pending_close = None;
                }
                if ui.button("Cancel").clicked() {
                    self.editor.pending_close = None;
                }
            });
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let name = self.scene_name();
                ui.label(format!("{name}{}", if self.dirty { " *" } else { "" }));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(&self.ui.status);
                });
            });
        });
    }

    /// The scene's display name: the file name without its `.scene.json`/`.json`
    /// extension (e.g. `main.scene.json` -> `main`).
    fn scene_name(&self) -> String {
        let Some(file) = self.path.as_ref().and_then(|p| p.file_name()) else {
            return "Untitled".to_string();
        };
        let file = file.to_string_lossy();
        if file.to_ascii_lowercase().ends_with(".scene.json") {
            file[..file.len() - ".scene.json".len()].to_string()
        } else if let Some((stem, _)) = file.rsplit_once('.') {
            stem.to_string()
        } else {
            file.into_owned()
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let name = self.scene_name();
        let title = format!("Flux Editor — {name}{}", if self.dirty { " *" } else { "" });
        if title != self.title {
            self.title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    fn step_play(&mut self, ctx: &egui::Context) {
        let vp = self.ui.viewport_rect;
        let Some(session) = &mut self.play else {
            return;
        };
        let dt = (ctx.input(|i| i.stable_dt) as f64).min(0.1);
        let keys: HashSet<String> = if ctx.wants_keyboard_input() {
            HashSet::new()
        } else {
            ctx.input(|i| i.keys_down.iter().map(|k| format!("{k:?}")).collect())
        };
        let (mouse_pos, mouse_buttons, scroll, over) = ctx.input(|i| {
            let pos = i.pointer.latest_pos();
            let over = pos.is_some_and(|p| vp.contains(p));
            let mut buttons = HashSet::new();
            if over {
                if i.pointer.primary_down() {
                    buttons.insert("Left".to_string());
                }
                if i.pointer.secondary_down() {
                    buttons.insert("Right".to_string());
                }
                if i.pointer.middle_down() {
                    buttons.insert("Middle".to_string());
                }
            }
            let rel = pos.map(|p| p - vp.min).unwrap_or_default();
            let scroll = if over { i.raw_scroll_delta.y } else { 0.0 };
            (glam::Vec2::new(rel.x, rel.y), buttons, scroll, over)
        });
        let input = InputFrame {
            keys,
            mouse_pos,
            mouse_buttons,
            viewport: glam::Vec2::new(vp.width(), vp.height()),
            scroll,
            pointer_over: over,
        };
        session.step(dt, &input);
        self.logs.extend(session.drain_logs());
        // A `Scene:Load`/`Scene:Reload` from Luau swaps in a fresh session.
        let scene_request = session.take_scene_request();
        ctx.request_repaint();
        if let Some(rel) = scene_request {
            self.load_play_scene(&rel);
        }
    }
}

/// The first Script/Module instance whose source is the file `rel` — what
/// `script` resolves to for hierarchy-aware completion in that file.
fn script_instance(world: &World, rel: &str) -> Option<InstanceId> {
    world.descendants(world.root()).into_iter().find(|&id| {
        matches!(world.class_name(id), Some("Script") | Some("Module"))
            && matches!(world.get_prop(id, "SourcePath"), Some(Value::Asset(p)) if p == rel)
    })
}

/// A "[swatch] Label" row for the settings syntax-color list; returns whether
/// the color changed this frame.
fn color_row(ui: &mut egui::Ui, label: &str, c: &mut [u8; 3]) -> bool {
    ui.horizontal(|ui| {
        let changed = ui.color_edit_button_srgb(c).changed();
        ui.label(label);
        changed
    })
    .inner
}

/// A safe default file name for a generated source file. `*.module.luau` is
/// recognised as a Module, a plain `*.luau` as a Script (see `flux_render`).
fn source_file_name(instance_name: &str, is_module: bool) -> String {
    let stem: String = instance_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = if stem.trim_matches(['_', '-']).is_empty() {
        if is_module { "Module" } else { "Script" }.to_string()
    } else {
        stem
    };
    if is_module {
        format!("{stem}.module.luau")
    } else {
        format!("{stem}.luau")
    }
}

/// Starter content for a freshly generated source file.
fn default_source(name: &str, is_module: bool) -> String {
    if is_module {
        format!("--!strict\n-- {name} module\n\nlocal module = {{}}\n\nreturn module\n")
    } else {
        format!("--!strict\n-- {name} script\n\nprint(\"{name} running\")\n")
    }
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(FileOp::Exit);
        }

        if !self.settings_applied {
            self.settings_applied = true;
            self.apply_settings(ctx);
        }
        self.icons.sync_theme_from(&ctx.style().visuals);
        if let Some(root) = self.project_root() {
            self.textures.poll_hot_reload(ctx, &root);
        }
        self.step_play(ctx);
        // Keep the edit-world's tilemap grids current (cheap when unchanged).
        // The play session syncs its own world in `Session::step`.
        {
            let root = self
                .project_root()
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            flux_core::tilemap::sync(
                &mut self.world,
                &mut self.tile_cache,
                &mut self.worldgen_cache,
                &root,
            );
        }
        self.handle_shortcuts(ctx);
        self.menu_bar(ctx);
        self.toolbar(ctx);
        self.status_bar(ctx);
        self.output_panel(ctx);
        self.asset_browser(ctx);

        let playing = self.playing();
        let root = self.project_root();
        {
            let play_rc = self.play.as_ref().map(|s| s.world());
            let Self {
                world,
                ui,
                icons,
                textures,
                anim_cache,
                tile_cache,
                editor,
                settings,
                ..
            } = &mut *self;
            let guard;
            let active: &World = match &play_rc {
                Some(rc) => {
                    guard = rc.borrow();
                    &guard
                }
                None => world,
            };
            egui::SidePanel::left("explorer")
                .default_width(240.0)
                .show(ctx, |panel| {
                    panel.horizontal(|panel| {
                        icons.icon(Icon::Hierarchy).size(16.0).show(panel);
                        panel.heading("Explorer");
                    });
                    panel.separator();
                    egui::ScrollArea::vertical().show(panel, |panel| {
                        crate::explorer::show(panel, active, ui, icons);
                    });
                });
            egui::SidePanel::right("properties")
                .default_width(280.0)
                .show(ctx, |panel| {
                    panel.horizontal(|panel| {
                        icons.icon(Icon::Inspector).size(16.0).show(panel);
                        panel.heading("Properties");
                    });
                    panel.separator();
                    egui::ScrollArea::vertical().show(panel, |panel| {
                        crate::properties::show(panel, active, ui, root.as_deref(), anim_cache, icons);
                    });
                });
            egui::CentralPanel::default().show(ctx, |panel| {
                crate::script_editor::tab_strip(panel, editor, icons);
                panel.separator();
                match editor.active {
                    ActiveTab::Scene => {
                        crate::viewport::show(
                            panel,
                            active,
                            ui,
                            playing,
                            root.as_deref(),
                            textures,
                            anim_cache,
                            tile_cache,
                        );
                    }
                    ActiveTab::Script(i) => {
                        // Resolve `script` to the instance the open file is
                        // attached to, so completion can walk the hierarchy.
                        let script_id =
                            editor.tabs.get(i).and_then(|t| script_instance(active, &t.rel));
                        let nav = crate::script_editor::SceneNav {
                            world: active,
                            script: script_id,
                        };
                        if let Some(tab) = editor.tabs.get_mut(i) {
                            crate::script_editor::code_area(
                                panel,
                                tab,
                                &mut editor.font_size,
                                &mut editor.find,
                                &mut editor.assist,
                                icons,
                                Some(&nav),
                                &settings.syntax,
                            );
                        }
                    }
                }
            });
        }

        let pending = std::mem::take(&mut self.ui.queue);
        if let Some(session) = &self.play {
            let rc = session.world();
            let mut w = rc.borrow_mut();
            let mut err = None;
            for p in pending {
                if let Err(e) = apply_ephemeral(&mut w, p.cmd) {
                    err = Some(e.to_string());
                }
            }
            drop(w);
            if let Some(e) = err {
                self.ui.status = e;
            }
        } else {
            for p in pending {
                self.apply(p.cmd, p.merge);
            }
        }

        if let Some(id) = self.ui.selection {
            let alive = match &self.play {
                Some(s) => s.world().borrow().contains(id),
                None => self.world.contains(id),
            };
            if !alive {
                self.ui.selection = None;
            }
        }

        if let Some((rel, line)) = self.ui.open_script.take() {
            match self.project_root() {
                Some(root) => self.editor.open(&rel, &root, line),
                None => self.ui.status = "Save the project before opening scripts".to_string(),
            }
        }

        if let Some(rel) = self.ui.open_animation.take() {
            match self.project_root() {
                Some(root) => match std::fs::read_to_string(root.join(&rel)) {
                    Ok(json) => self.anim.open_doc(&rel, &json),
                    Err(e) => self.ui.status = format!("Open failed: {e}"),
                },
                None => self.ui.status = "Save the project before editing animations".to_string(),
            }
        }
        if let Some(root) = self.project_root() {
            self.anim.show(ctx, &mut self.textures, &root, &self.icons);
        }

        if let Some(id) = self.ui.create_source.take() {
            self.create_source_for(id);
        }

        self.settings_window(ctx);
        self.confirm_modal(ctx);
        self.script_warn_modal(ctx);
        self.close_tab_modal(ctx);
        self.update_title(ctx);
    }
}
