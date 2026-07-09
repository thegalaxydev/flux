use flux_core::{CoreError, InstanceId, Value, ValueType, World, registry};
use mlua::{
    IntoLua, Lua, MetaMethod, UserData, UserDataMethods, UserDataRef, Value as LuaValue,
};

use crate::signal::{LuaSignal, Signal};
use crate::types::{LuaColor, LuaUDim2, LuaVec2, as_color, as_udim2, as_vec2};
use crate::{input_handle, world_handle};

#[derive(Clone, Copy, PartialEq)]
pub struct LuaInstance(pub InstanceId);

fn destroyed() -> mlua::Error {
    mlua::Error::RuntimeError("attempt to use a destroyed Instance".to_string())
}

fn lua_err(e: CoreError) -> mlua::Error {
    mlua::Error::RuntimeError(e.to_string())
}

fn check(w: &World, id: InstanceId) -> mlua::Result<()> {
    if w.contains(id) { Ok(()) } else { Err(destroyed()) }
}

fn is_button(w: &World, id: InstanceId) -> bool {
    matches!(
        (w.class_of(id), registry().find("Button")),
        (Some(c), Some(b)) if registry().is_a(c, b)
    )
}

fn aabb(w: &World, id: InstanceId) -> Option<(glam::Vec2, glam::Vec2)> {
    let Some(Value::Vec2(pos)) = w.get_prop(id, "Position") else {
        return None;
    };
    let Some(Value::Vec2(size)) = w.get_prop(id, "Size") else {
        return None;
    };
    let half = *size * 0.5;
    Some((*pos - half, *pos + half))
}

impl UserData for LuaInstance {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("GetService", |lua, this, name: String| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            if w.class_name(this.0) != Some("Game") {
                return Err(mlua::Error::RuntimeError(
                    "GetService can only be called on game".to_string(),
                ));
            }
            if name == "DataStoreService" {
                let provider = lua
                    .app_data_ref::<crate::datastore::Provider>()
                    .expect("provider app data missing")
                    .clone();
                return crate::datastore::LuaDataStoreService { provider }.into_lua(lua);
            }
            match w.service(&name) {
                Some(id) => LuaInstance(id).into_lua(lua),
                None => Err(mlua::Error::RuntimeError(format!(
                    "'{name}' is not a valid service"
                ))),
            }
        });
        m.add_method("FindFirstChild", |lua, this, name: String| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            Ok(w.find_first_child(this.0, &name).map(LuaInstance))
        });
        m.add_method("GetChildren", |lua, this, ()| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            Ok(w.children(this.0)
                .iter()
                .map(|&c| LuaInstance(c))
                .collect::<Vec<_>>())
        });
        m.add_method("GetDescendants", |lua, this, ()| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            Ok(w.descendants(this.0)
                .into_iter()
                .skip(1)
                .map(LuaInstance)
                .collect::<Vec<_>>())
        });
        m.add_method("IsA", |lua, this, class: String| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            let reg = registry();
            Ok(match (w.class_of(this.0), reg.find(&class)) {
                (Some(c), Some(target)) => reg.is_a(c, target),
                _ => false,
            })
        });
        m.add_method("Destroy", |lua, this, ()| {
            let rc = world_handle(lua);
            let mut w = rc.borrow_mut();
            check(&w, this.0)?;
            w.destroy(this.0).map_err(lua_err)
        });
        m.add_method("Clone", |lua, this, ()| {
            let rc = world_handle(lua);
            let mut w = rc.borrow_mut();
            check(&w, this.0)?;
            let snap = w.snapshot_subtree(this.0).ok_or_else(destroyed)?;
            let map = w.restore_subtree_detached(&snap).map_err(lua_err)?;
            Ok(LuaInstance(map[&this.0]))
        });
        m.add_method("GetTouchingSprites", |lua, this, ()| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            let sprite = registry().find("Sprite");
            let is_sprite = |id| matches!((w.class_of(id), sprite), (Some(c), Some(s)) if registry().is_a(c, s));
            if !is_sprite(this.0) {
                return Ok(Vec::new());
            }
            let Some((min_a, max_a)) = aabb(&w, this.0) else {
                return Ok(Vec::new());
            };
            let mut out = Vec::new();
            for id in w.descendants(w.workspace()) {
                if id == this.0 || !is_sprite(id) {
                    continue;
                }
                if let Some((min_b, max_b)) = aabb(&w, id) {
                    let overlap = min_a.x <= max_b.x
                        && max_a.x >= min_b.x
                        && min_a.y <= max_b.y
                        && max_a.y >= min_b.y;
                    if overlap {
                        out.push(LuaInstance(id));
                    }
                }
            }
            Ok(out)
        });
        m.add_meta_method(MetaMethod::Index, |lua, this, key: String| {
            index(lua, this.0, &key)
        });
        m.add_meta_method(
            MetaMethod::NewIndex,
            |lua, this, (key, value): (String, LuaValue)| newindex(lua, this.0, &key, value),
        );
        m.add_meta_method(MetaMethod::ToString, |lua, this, ()| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            Ok(w.name(this.0).unwrap_or("<destroyed>").to_string())
        });
        m.add_meta_method(MetaMethod::Eq, |_, a, b: UserDataRef<LuaInstance>| {
            Ok(a.0 == b.0)
        });
    }
}

fn index(lua: &Lua, id: InstanceId, key: &str) -> mlua::Result<LuaValue> {
    let rc = world_handle(lua);
    let w = rc.borrow();
    check(&w, id)?;
    match key {
        "Name" => w.name(id).unwrap().into_lua(lua),
        "ClassName" => w.class_name(id).unwrap().into_lua(lua),
        "Parent" => match w.parent(id) {
            Some(p) => LuaInstance(p).into_lua(lua),
            None => Ok(LuaValue::Nil),
        },
        "Heartbeat" if w.class_name(id) == Some("Game") => {
            let signal = lua
                .app_data_ref::<Signal>()
                .expect("heartbeat app data")
                .clone();
            LuaSignal(signal).into_lua(lua)
        }
        "Activated" if is_button(&w, id) => {
            let signals = lua
                .app_data_ref::<crate::ButtonSignals>()
                .expect("button signals app data")
                .clone();
            let signal = signals.borrow_mut().entry(id).or_default().clone();
            LuaSignal(signal).into_lua(lua)
        }
        "AbsolutePosition" | "AbsoluteSize" if flux_core::gui::is_gui_object(&w, id) => {
            let screen = flux_core::Rect2::from_screen(input_handle(lua).borrow().viewport);
            let rect = flux_core::gui::absolute_rect(&w, id, screen)
                .unwrap_or(flux_core::Rect2::new(glam::Vec2::ZERO, glam::Vec2::ZERO));
            let v = if key == "AbsolutePosition" { rect.min } else { rect.size };
            LuaVec2(v).into_lua(lua)
        }
        _ => {
            if let Some(v) = w.get_prop(id, key) {
                value_to_lua(lua, &w, v)
            } else if let Some(child) = w.find_first_child(id, key) {
                LuaInstance(child).into_lua(lua)
            } else {
                Err(mlua::Error::RuntimeError(format!(
                    "{key} is not a valid member of {}",
                    w.class_name(id).unwrap_or("Instance")
                )))
            }
        }
    }
}

fn newindex(lua: &Lua, id: InstanceId, key: &str, value: LuaValue) -> mlua::Result<()> {
    let rc = world_handle(lua);
    let mut w = rc.borrow_mut();
    check(&w, id)?;
    match key {
        "Name" => {
            let name = value
                .as_string()
                .map(|s| s.to_string_lossy())
                .ok_or_else(|| mlua::Error::RuntimeError("Name must be a string".to_string()))?;
            w.set_name(id, name).map_err(lua_err)
        }
        "Parent" => match value {
            LuaValue::Nil => w.detach(id).map_err(lua_err),
            v => {
                let target = as_instance(&v).ok_or_else(|| {
                    mlua::Error::RuntimeError("Parent must be an Instance or nil".to_string())
                })?;
                if !w.contains(target.0) {
                    return Err(destroyed());
                }
                w.reparent(id, target.0).map_err(lua_err)
            }
        },
        _ => {
            let Some(current) = w.get_prop(id, key) else {
                return Err(mlua::Error::RuntimeError(format!(
                    "{key} is not a valid member of {}",
                    w.class_name(id).unwrap_or("Instance")
                )));
            };
            let expected = current.ty();
            let new = lua_to_value(expected, &value).map_err(|got| {
                mlua::Error::RuntimeError(format!(
                    "invalid value for {key}: expected {expected:?}, got {got}"
                ))
            })?;
            w.set_prop(id, key, new).map_err(lua_err)
        }
    }
}

pub(crate) fn as_instance(v: &LuaValue) -> Option<LuaInstance> {
    v.as_userdata()
        .and_then(|u| u.borrow::<LuaInstance>().ok())
        .map(|r| *r)
}

fn value_to_lua(lua: &Lua, w: &World, v: &Value) -> mlua::Result<LuaValue> {
    match v {
        Value::Bool(b) => Ok(LuaValue::Boolean(*b)),
        Value::Number(n) => n.into_lua(lua),
        Value::String(s) | Value::Asset(s) => s.as_str().into_lua(lua),
        Value::Vec2(v) => LuaVec2(*v).into_lua(lua),
        Value::UDim2(u) => LuaUDim2(*u).into_lua(lua),
        Value::Color(c) => LuaColor(*c).into_lua(lua),
        Value::InstanceRef(Some(t)) if w.contains(*t) => LuaInstance(*t).into_lua(lua),
        Value::InstanceRef(_) => Ok(LuaValue::Nil),
    }
}

fn lua_to_value(expected: ValueType, v: &LuaValue) -> Result<Value, &'static str> {
    let got = v.type_name();
    match expected {
        ValueType::Bool => v.as_boolean().map(Value::Bool).ok_or(got),
        // Accept both Luau number and integer values (a whole-number literal like
        // `0` arrives as an integer).
        ValueType::Number => v
            .as_number()
            .or_else(|| v.as_integer().map(|i| i as f64))
            .map(Value::Number)
            .ok_or(got),
        ValueType::String => v
            .as_string()
            .map(|s| Value::String(s.to_string_lossy()))
            .ok_or(got),
        ValueType::Asset => v
            .as_string()
            .map(|s| Value::Asset(s.to_string_lossy()))
            .ok_or(got),
        ValueType::Vec2 => as_vec2(v).map(Value::Vec2).ok_or(got),
        ValueType::UDim2 => as_udim2(v).map(Value::UDim2).ok_or(got),
        ValueType::Color => as_color(v).map(Value::Color).ok_or(got),
        ValueType::InstanceRef => match v {
            LuaValue::Nil => Ok(Value::InstanceRef(None)),
            v => as_instance(v)
                .map(|i| Value::InstanceRef(Some(i.0)))
                .ok_or(got),
        },
    }
}
