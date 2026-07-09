mod datastore;
mod enums;
mod instance;
mod scheduler;
mod signal;
mod types;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use flux_core::{InstanceId, Value, World, registry};
use glam::Vec2;
use mlua::{Function, Lua, MultiValue, Table};

pub use datastore::Provider;
pub use instance::LuaInstance;

use signal::Signal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
}

#[derive(Clone, Default)]
pub struct Log(Rc<RefCell<Vec<LogEntry>>>);

impl Log {
    pub fn push(&self, level: LogLevel, message: String) {
        self.0.borrow_mut().push(LogEntry { level, message });
    }

    pub fn info(&self, message: String) {
        self.push(LogLevel::Info, message);
    }

    pub fn warn(&self, message: String) {
        self.push(LogLevel::Warn, message);
    }

    pub fn error(&self, message: String) {
        self.push(LogLevel::Error, message);
    }

    pub fn drain(&self) -> Vec<LogEntry> {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

#[derive(Default, Clone)]
pub struct InputFrame {
    pub keys: HashSet<String>,
    pub mouse_pos: Vec2,
    pub mouse_buttons: HashSet<String>,
    pub viewport: Vec2,
}

#[derive(Default)]
pub struct InputState {
    pub keys: HashSet<String>,
    pub mouse_pos: Vec2,
    pub mouse_buttons: HashSet<String>,
    pub viewport: Vec2,
}

pub(crate) type WorldHandle = Rc<RefCell<World>>;
pub(crate) type ButtonSignals = Rc<RefCell<HashMap<InstanceId, Signal>>>;

pub(crate) fn world_handle(lua: &Lua) -> WorldHandle {
    lua.app_data_ref::<WorldHandle>()
        .expect("world app data missing")
        .clone()
}

pub struct ScriptHost {
    lua: Lua,
    world: WorldHandle,
    scheduler: Rc<RefCell<scheduler::Scheduler>>,
    heartbeat: signal::Signal,
    input: Rc<RefCell<InputState>>,
    button_signals: ButtonSignals,
    prev_left: bool,
    log: Log,
}

impl ScriptHost {
    pub fn new(world: World, script_root: &Path, provider: Provider) -> Result<Self, String> {
        let lua = Lua::new();
        let world: WorldHandle = Rc::new(RefCell::new(world));
        let scheduler = Rc::new(RefCell::new(scheduler::Scheduler::default()));
        let heartbeat = signal::Signal::default();
        let input = Rc::new(RefCell::new(InputState::default()));
        let button_signals: ButtonSignals = Rc::new(RefCell::new(HashMap::new()));
        let log = Log::default();

        lua.set_app_data(world.clone());
        lua.set_app_data(input.clone());
        lua.set_app_data(heartbeat.clone());
        lua.set_app_data(button_signals.clone());
        lua.set_app_data(provider);
        lua.set_app_data(log.clone());

        setup_globals(&lua, &world, &scheduler, &log).map_err(|e| e.to_string())?;

        let host = Self {
            lua,
            world,
            scheduler,
            heartbeat,
            input,
            button_signals,
            prev_left: false,
            log,
        };
        host.start_scripts(script_root);
        Ok(host)
    }

    pub fn world(&self) -> WorldHandle {
        self.world.clone()
    }

    pub fn push_error(&self, message: String) {
        self.log.error(message);
    }

    pub fn step(&mut self, dt: f64, input: &InputFrame) {
        {
            let mut state = self.input.borrow_mut();
            state.keys = input.keys.clone();
            state.mouse_pos = input.mouse_pos;
            state.mouse_buttons = input.mouse_buttons.clone();
            state.viewport = input.viewport;
        }
        scheduler::step(&self.lua, &self.scheduler, &self.log, dt);
        signal::fire(&self.lua, &self.scheduler, &self.log, &self.heartbeat, dt);
        self.process_gui_clicks(input);
    }

    fn process_gui_clicks(&mut self, input: &InputFrame) {
        let left_down = input.mouse_buttons.contains("Left");
        let clicked = left_down && !self.prev_left;
        self.prev_left = left_down;
        if !clicked {
            return;
        }
        let Some(button_class) = registry().find("Button") else {
            return;
        };
        let target = {
            let w = self.world.borrow();
            let Some(gui) = w.gui() else { return };
            let screen = flux_core::Rect2::from_screen(input.viewport);
            let point = input.mouse_pos;
            let mut hit: Option<(InstanceId, f64)> = None;
            for id in w.descendants(gui) {
                let Some(class) = w.class_of(id) else { continue };
                if !registry().is_a(class, button_class) {
                    continue;
                }
                if !matches!(w.get_prop(id, "Visible"), Some(Value::Bool(true))) {
                    continue;
                }
                let Some(rect) = flux_core::gui::absolute_rect(&w, id, screen) else {
                    continue;
                };
                if !rect.contains(point) {
                    continue;
                }
                // A click on a region clipped away by an ancestor doesn't count.
                match flux_core::gui::clip_rect(&w, id, screen) {
                    Some(clip) if clip.contains(point) => {}
                    _ => continue,
                }
                let z = match w.get_prop(id, "ZIndex") {
                    Some(Value::Number(z)) => *z,
                    _ => 0.0,
                };
                if hit.map(|(_, hz)| z >= hz).unwrap_or(true) {
                    hit = Some((id, z));
                }
            }
            hit.map(|(id, _)| id)
        };
        if let Some(id) = target {
            let signal = self.button_signals.borrow().get(&id).cloned();
            if let Some(signal) = signal {
                signal::fire(&self.lua, &self.scheduler, &self.log, &signal, ());
            }
        }
    }

    pub fn drain_logs(&self) -> Vec<LogEntry> {
        self.log.drain()
    }

    fn start_scripts(&self, script_root: &Path) {
        let scripts: Vec<(InstanceId, String, String)> = {
            let w = self.world.borrow();
            w.descendants(w.root())
                .into_iter()
                .filter(|&id| w.class_name(id) == Some("Script"))
                .filter(|&id| matches!(w.get_prop(id, "Enabled"), Some(Value::Bool(true))))
                .filter_map(|id| match w.get_prop(id, "SourcePath") {
                    Some(Value::Asset(p)) if !p.is_empty() => {
                        Some((id, p.clone(), w.name(id).unwrap_or("Script").to_string()))
                    }
                    _ => None,
                })
                .collect()
        };
        for (id, rel, name) in scripts {
            let full = script_root.join(&rel);
            let src = match std::fs::read_to_string(&full) {
                Ok(s) => s,
                Err(e) => {
                    self.log
                        .error(format!("{name}: cannot read '{}': {e}", full.display()));
                    continue;
                }
            };
            if let Err(e) = self.run_script(id, &src, &rel) {
                self.log.error(format!("{rel}: {e}"));
            }
        }
    }

    fn run_script(&self, id: InstanceId, src: &str, chunk_name: &str) -> mlua::Result<()> {
        let env = self.lua.create_table()?;
        env.set("script", LuaInstance(id))?;
        let mt = self.lua.create_table()?;
        mt.set("__index", self.lua.globals())?;
        env.set_metatable(Some(mt))?;
        let func = self
            .lua
            .load(src)
            .set_name(format!("@{chunk_name}"))
            .set_environment(env)
            .into_function()?;
        let thread = self.lua.create_thread(func)?;
        scheduler::resume_thread(
            &self.lua,
            &self.scheduler,
            &self.log,
            thread,
            MultiValue::new(),
        );
        Ok(())
    }
}

fn setup_globals(
    lua: &Lua,
    world: &WorldHandle,
    scheduler: &Rc<RefCell<scheduler::Scheduler>>,
    log: &Log,
) -> mlua::Result<()> {
    let g = lua.globals();
    let (root, ws) = {
        let w = world.borrow();
        (w.root(), w.workspace())
    };
    g.set("game", LuaInstance(root))?;
    g.set("workspace", LuaInstance(ws))?;
    g.set("Vec2", types::vec2_table(lua)?)?;
    g.set("Color", types::color_table(lua)?)?;
    g.set("UDim", types::udim_table(lua)?)?;
    g.set("UDim2", types::udim2_table(lua)?)?;
    g.set("Enum", enums::enum_table(lua)?)?;
    g.set("task", scheduler::task_table(lua, scheduler.clone())?)?;
    g.set("Input", input_table(lua)?)?;

    let log_info = log.clone();
    g.set(
        "print",
        lua.create_function(move |lua, args: MultiValue| {
            log_info.info(join_args(lua, args)?);
            Ok(())
        })?,
    )?;
    let log_warn = log.clone();
    g.set(
        "warn",
        lua.create_function(move |lua, args: MultiValue| {
            log_warn.warn(join_args(lua, args)?);
            Ok(())
        })?,
    )?;
    Ok(())
}

pub(crate) fn input_handle(lua: &Lua) -> Rc<RefCell<InputState>> {
    lua.app_data_ref::<Rc<RefCell<InputState>>>()
        .expect("input app data missing")
        .clone()
}

fn input_table(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "IsKeyDown",
        lua.create_function(|lua, key: mlua::Value| {
            let token = enums::resolve_input_token(&key).ok_or_else(|| {
                mlua::Error::RuntimeError(
                    "IsKeyDown expects an Enum.KeyCode or key name string".to_string(),
                )
            })?;
            Ok(input_handle(lua).borrow().keys.contains(&token))
        })?,
    )?;
    t.set(
        "IsMouseDown",
        lua.create_function(|lua, button: mlua::Value| {
            let token = enums::resolve_input_token(&button).ok_or_else(|| {
                mlua::Error::RuntimeError(
                    "IsMouseDown expects an Enum.UserInputType or button name string".to_string(),
                )
            })?;
            Ok(input_handle(lua).borrow().mouse_buttons.contains(&token))
        })?,
    )?;
    t.set(
        "MousePosition",
        lua.create_function(|lua, ()| {
            let p = input_handle(lua).borrow().mouse_pos;
            Ok(types::LuaVec2(p))
        })?,
    )?;
    t.set(
        "ViewportSize",
        lua.create_function(|lua, ()| {
            let v = input_handle(lua).borrow().viewport;
            Ok(types::LuaVec2(v))
        })?,
    )?;
    Ok(t)
}

fn join_args(lua: &Lua, args: MultiValue) -> mlua::Result<String> {
    let tostring: Function = lua.globals().get("tostring")?;
    let mut parts = Vec::new();
    for v in args {
        parts.push(tostring.call::<String>(v)?);
    }
    Ok(parts.join(" "))
}
