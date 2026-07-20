use flux_core::{CoreError, InstanceId, Value, ValueType, World, registry};
use mlua::{IntoLua, Lua, MetaMethod, UserData, UserDataMethods, UserDataRef, Value as LuaValue};

use crate::signal::{LuaSignal, Signal};
use crate::types::{LuaColor, LuaRect, LuaUDim2, LuaVec2, as_color, as_rect, as_udim2, as_vec2};
use crate::{input_handle, world_handle};

#[derive(Clone, Copy, PartialEq)]
pub struct LuaInstance(pub InstanceId);

/// The `Scene` global: switch scenes and read the current scene's name.
#[derive(Clone)]
pub(crate) struct LuaScene(pub(crate) crate::SceneHandle);

/// Scene display name from a relative path: the file name without its
/// `.scene.json`/`.json` extension (`levels/hub.scene.json` -> `hub`).
fn scene_name(rel: &str) -> String {
    let file = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
    if file.to_ascii_lowercase().ends_with(".scene.json") {
        file[..file.len() - ".scene.json".len()].to_string()
    } else if let Some((stem, _)) = file.rsplit_once('.') {
        stem.to_string()
    } else {
        file.to_string()
    }
}

impl UserData for LuaScene {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("Load", |_, this, path: String| {
            this.0.borrow_mut().request = Some(path);
            Ok(())
        });
        m.add_method("Reload", |_, this, ()| {
            let current = this.0.borrow().current.clone();
            this.0.borrow_mut().request = Some(current);
            Ok(())
        });
        m.add_meta_method(MetaMethod::Index, |lua, this, key: String| match key.as_str() {
            "Name" => scene_name(&this.0.borrow().current).into_lua(lua),
            "Path" => this.0.borrow().current.clone().into_lua(lua),
            other => Err(mlua::Error::RuntimeError(format!(
                "{other} is not a valid member of Scene"
            ))),
        });
    }
}

fn destroyed() -> mlua::Error {
    mlua::Error::RuntimeError("attempt to use a destroyed Instance".to_string())
}

fn lua_err(e: CoreError) -> mlua::Error {
    mlua::Error::RuntimeError(e.to_string())
}

fn check(w: &World, id: InstanceId) -> mlua::Result<()> {
    if w.contains(id) {
        Ok(())
    } else {
        Err(destroyed())
    }
}

fn is_button(w: &World, id: InstanceId) -> bool {
    matches!(
        (w.class_of(id), registry().find("Button")),
        (Some(c), Some(b)) if registry().is_a(c, b)
    )
}

fn is_animated_sprite(w: &World, id: InstanceId) -> bool {
    w.class_name(id) == Some("AnimatedSprite")
}

fn anim_only(method: &str) -> mlua::Error {
    mlua::Error::RuntimeError(format!("{method} can only be called on an AnimatedSprite"))
}

fn is_tilemap(w: &World, id: InstanceId) -> bool {
    w.class_name(id) == Some("Tilemap")
}

fn tilemap_only(method: &str) -> mlua::Error {
    mlua::Error::RuntimeError(format!("{method} can only be called on a Tilemap"))
}

/// Resolve a `Tilemap`'s `TileSet` asset via the shared (Lua app-data) cache, so
/// tile string ids can map to/from palette indices.
fn tileset_of(
    lua: &Lua,
    w: &World,
    id: InstanceId,
) -> Option<std::rc::Rc<flux_core::tilemap::TileSet>> {
    let path = match w.get_prop(id, "TileSet") {
        Some(Value::Asset(s)) if !s.is_empty() => s.clone(),
        _ => return None,
    };
    let cache = crate::tile_cache_handle(lua);
    let root = crate::asset_root(lua);
    let ts = cache.borrow_mut().get(&path, &root);
    ts
}

fn is_camera(w: &World, id: InstanceId) -> bool {
    w.class_name(id) == Some("Camera2D")
}

fn is_building(w: &World, id: InstanceId) -> bool {
    w.class_name(id) == Some("Building")
}

fn building_only(method: &str) -> mlua::Error {
    mlua::Error::RuntimeError(format!("{method} can only be called on a Building"))
}

/// A camera's `(Position, Zoom)` for screen<->world conversion.
fn camera_view(w: &World, id: InstanceId) -> (glam::Vec2, f32) {
    let pos = match w.get_prop(id, "Position") {
        Some(Value::Vec2(p)) => *p,
        _ => glam::Vec2::ZERO,
    };
    let zoom = match w.get_prop(id, "Zoom") {
        Some(Value::Number(z)) => (*z as f32).max(1e-3),
        _ => 1.0,
    };
    (pos, zoom)
}

/// Resolve a `Tilemap`'s `Buildings` catalog via the shared (Lua app-data) cache.
fn catalog_of(
    lua: &Lua,
    w: &World,
    id: InstanceId,
) -> Option<std::rc::Rc<flux_core::building::BuildingCatalog>> {
    let path = match w.get_prop(id, "Buildings") {
        Some(Value::Asset(s)) if !s.is_empty() => s.clone(),
        _ => return None,
    };
    let cache = crate::building_cache_handle(lua);
    let root = crate::asset_root(lua);
    let cat = cache.borrow_mut().get(&path, &root);
    cat
}

/// A `Tilemap`'s footprint (`tile_width`, `tile_height`) and world `Position`.
fn tilemap_geom(w: &World, id: InstanceId) -> (f32, f32, glam::Vec2) {
    let num = |name, d: f32| match w.get_prop(id, name) {
        Some(Value::Number(n)) => *n as f32,
        _ => d,
    };
    let pos = match w.get_prop(id, "Position") {
        Some(Value::Vec2(v)) => *v,
        _ => glam::Vec2::ZERO,
    };
    (num("TileWidth", 64.0).max(1.0), num("TileHeight", 32.0).max(1.0), pos)
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
            if name == "SaveService" {
                return crate::save::LuaSaveService.into_lua(lua);
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
        // ---- AnimatedSprite controls ----
        m.add_method(
            "Play",
            |lua, this, (animation, restart): (String, Option<bool>)| {
                let rc = world_handle(lua);
                let mut w = rc.borrow_mut();
                check(&w, this.0)?;
                if !is_animated_sprite(&w, this.0) {
                    return Err(anim_only("Play"));
                }
                flux_core::animation::play(&mut w, this.0, &animation, restart.unwrap_or(false));
                Ok(())
            },
        );
        m.add_method("Stop", |lua, this, ()| {
            let rc = world_handle(lua);
            let mut w = rc.borrow_mut();
            check(&w, this.0)?;
            if !is_animated_sprite(&w, this.0) {
                return Err(anim_only("Stop"));
            }
            flux_core::animation::stop(&mut w, this.0);
            Ok(())
        });
        m.add_method("Pause", |lua, this, ()| {
            let rc = world_handle(lua);
            let mut w = rc.borrow_mut();
            check(&w, this.0)?;
            if !is_animated_sprite(&w, this.0) {
                return Err(anim_only("Pause"));
            }
            flux_core::animation::pause(&mut w, this.0);
            Ok(())
        });
        m.add_method("Resume", |lua, this, ()| {
            let rc = world_handle(lua);
            let mut w = rc.borrow_mut();
            check(&w, this.0)?;
            if !is_animated_sprite(&w, this.0) {
                return Err(anim_only("Resume"));
            }
            flux_core::animation::resume(&mut w, this.0);
            Ok(())
        });
        // ---- Tilemap tile access ----
        m.add_method("GetMapSize", |lua, this, ()| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            if !is_tilemap(&w, this.0) {
                return Err(tilemap_only("GetMapSize"));
            }
            let (width, height) = match w.tile_grid(this.0) {
                Some(g) => (g.width(), g.height()),
                None => (0, 0),
            };
            Ok((width, height))
        });
        m.add_method("GetTile", |lua, this, (x, y): (i32, i32)| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            if !is_tilemap(&w, this.0) {
                return Err(tilemap_only("GetTile"));
            }
            let Some(cell) = w.tile_grid(this.0).and_then(|g| g.cell(x, y)) else {
                return Ok(None);
            };
            Ok(tileset_of(lua, &w, this.0).and_then(|ts| ts.tile(cell.tile).map(|t| t.id.clone())))
        });
        m.add_method("SetTile", |lua, this, (x, y, id): (i32, i32, String)| {
            // Resolve the id -> index first (drops the tileset/world borrows),
            // then take a mutable world borrow to write.
            let index = {
                let rc = world_handle(lua);
                let w = rc.borrow();
                check(&w, this.0)?;
                if !is_tilemap(&w, this.0) {
                    return Err(tilemap_only("SetTile"));
                }
                let ts = tileset_of(lua, &w, this.0).ok_or_else(|| {
                    mlua::Error::RuntimeError("SetTile: Tilemap has no TileSet".to_string())
                })?;
                ts.index_of(&id).ok_or_else(|| {
                    mlua::Error::RuntimeError(format!("SetTile: unknown tile id '{id}'"))
                })?
            };
            let rc = world_handle(lua);
            let mut w = rc.borrow_mut();
            Ok(w.tile_grid_mut(this.0).map(|g| g.set_tile(x, y, index)).unwrap_or(false))
        });
        m.add_method("GetOre", |lua, this, (x, y): (i32, i32)| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            if !is_tilemap(&w, this.0) {
                return Err(tilemap_only("GetOre"));
            }
            match w.tile_grid(this.0).and_then(|g| g.cell(x, y)) {
                Some(cell) if cell.has_ore() => {
                    let id = tileset_of(lua, &w, this.0)
                        .and_then(|ts| ts.tile(cell.ore).map(|t| t.id.clone()));
                    Ok((id, cell.ore_amount as u32))
                }
                _ => Ok((None, 0)),
            }
        });
        m.add_method(
            "SetOre",
            |lua, this, (x, y, id, amount): (i32, i32, Option<String>, Option<f64>)| {
                let ore = match &id {
                    None => flux_core::tilemap::NO_ORE,
                    Some(id) => {
                        let rc = world_handle(lua);
                        let w = rc.borrow();
                        check(&w, this.0)?;
                        if !is_tilemap(&w, this.0) {
                            return Err(tilemap_only("SetOre"));
                        }
                        let ts = tileset_of(lua, &w, this.0).ok_or_else(|| {
                            mlua::Error::RuntimeError("SetOre: Tilemap has no TileSet".to_string())
                        })?;
                        ts.index_of(id).ok_or_else(|| {
                            mlua::Error::RuntimeError(format!("SetOre: unknown ore id '{id}'"))
                        })?
                    }
                };
                let amount = amount.unwrap_or(0.0).clamp(0.0, u16::MAX as f64) as u16;
                let rc = world_handle(lua);
                let mut w = rc.borrow_mut();
                check(&w, this.0)?;
                if !is_tilemap(&w, this.0) {
                    return Err(tilemap_only("SetOre"));
                }
                Ok(w.tile_grid_mut(this.0).map(|g| g.set_ore(x, y, ore, amount)).unwrap_or(false))
            },
        );
        m.add_method("MineOre", |lua, this, (x, y, amount): (i32, i32, u32)| {
            let rc = world_handle(lua);
            let mut w = rc.borrow_mut();
            check(&w, this.0)?;
            if !is_tilemap(&w, this.0) {
                return Err(tilemap_only("MineOre"));
            }
            let amount = amount.min(u16::MAX as u32) as u16;
            let removed = w
                .tile_grid_mut(this.0)
                .map(|g| g.mine(x, y, amount))
                .unwrap_or(0);
            Ok(removed as u32)
        });
        m.add_method("TileToWorld", |lua, this, (col, row): (i32, i32)| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            if !is_tilemap(&w, this.0) {
                return Err(tilemap_only("TileToWorld"));
            }
            let (tw, th, pos) = tilemap_geom(&w, this.0);
            let p = pos + flux_core::tilemap::tile_to_world(col, row, tw, th);
            LuaVec2(p).into_lua(lua)
        });
        m.add_method("WorldToTile", |lua, this, v: LuaValue| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            if !is_tilemap(&w, this.0) {
                return Err(tilemap_only("WorldToTile"));
            }
            let world_p = as_vec2(&v).ok_or_else(|| {
                mlua::Error::RuntimeError("WorldToTile expects a Vec2".to_string())
            })?;
            let (tw, th, pos) = tilemap_geom(&w, this.0);
            let (col, row) = flux_core::tilemap::world_to_tile(world_p - pos, tw, th);
            Ok((col, row))
        });
        // ---- Tilemap building placement ----
        m.add_method("CanPlace", |lua, this, (ty, col, row): (String, i32, i32)| {
            let cat = {
                let rc = world_handle(lua);
                let w = rc.borrow();
                check(&w, this.0)?;
                if !is_tilemap(&w, this.0) {
                    return Err(tilemap_only("CanPlace"));
                }
                match catalog_of(lua, &w, this.0) {
                    Some(c) => c,
                    None => return Ok(false),
                }
            };
            let Some(def) = cat.get(&ty) else {
                return Ok(false);
            };
            let rc = world_handle(lua);
            let w = rc.borrow();
            Ok(flux_core::building::can_place(&w, this.0, def, col, row))
        });
        m.add_method(
            "PlaceBuilding",
            |lua, this, (ty, col, row): (String, i32, i32)| {
                let cat = {
                    let rc = world_handle(lua);
                    let w = rc.borrow();
                    check(&w, this.0)?;
                    if !is_tilemap(&w, this.0) {
                        return Err(tilemap_only("PlaceBuilding"));
                    }
                    catalog_of(lua, &w, this.0).ok_or_else(|| {
                        mlua::Error::RuntimeError(
                            "PlaceBuilding: Tilemap has no Buildings catalog".to_string(),
                        )
                    })?
                };
                let def = cat.get(&ty).ok_or_else(|| {
                    mlua::Error::RuntimeError(format!("PlaceBuilding: unknown building '{ty}'"))
                })?;
                let rc = world_handle(lua);
                let mut w = rc.borrow_mut();
                Ok(flux_core::building::place(&mut w, this.0, def, col, row).map(LuaInstance))
            },
        );
        m.add_method("GetBuildingAt", |lua, this, (col, row): (i32, i32)| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            if !is_tilemap(&w, this.0) {
                return Err(tilemap_only("GetBuildingAt"));
            }
            Ok(flux_core::building::building_at(&w, this.0, col, row).map(LuaInstance))
        });
        m.add_method("RemoveBuilding", |lua, this, (col, row): (i32, i32)| {
            let rc = world_handle(lua);
            let mut w = rc.borrow_mut();
            check(&w, this.0)?;
            if !is_tilemap(&w, this.0) {
                return Err(tilemap_only("RemoveBuilding"));
            }
            Ok(flux_core::building::remove_at(&mut w, this.0, col, row))
        });
        // ---- Building inventory ----
        m.add_method("GetItem", |lua, this, item: String| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            if !is_building(&w, this.0) {
                return Err(building_only("GetItem"));
            }
            Ok(w.inventory(this.0).map(|i| i.count(&item)).unwrap_or(0))
        });
        m.add_method("AddItem", |lua, this, (item, n): (String, u32)| {
            let rc = world_handle(lua);
            let mut w = rc.borrow_mut();
            check(&w, this.0)?;
            if !is_building(&w, this.0) {
                return Err(building_only("AddItem"));
            }
            Ok(w.inventory_mut(this.0).map(|i| i.add(&item, n)).unwrap_or(0))
        });
        m.add_method("TakeItem", |lua, this, (item, n): (String, u32)| {
            let rc = world_handle(lua);
            let mut w = rc.borrow_mut();
            check(&w, this.0)?;
            if !is_building(&w, this.0) {
                return Err(building_only("TakeItem"));
            }
            Ok(w.inventory_mut(this.0).map(|i| i.take(&item, n)).unwrap_or(0))
        });
        m.add_method("ItemTotal", |lua, this, ()| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            if !is_building(&w, this.0) {
                return Err(building_only("ItemTotal"));
            }
            Ok(w.inventory(this.0).map(|i| i.total()).unwrap_or(0))
        });
        m.add_method("GetInventory", |lua, this, ()| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            if !is_building(&w, this.0) {
                return Err(building_only("GetInventory"));
            }
            let table = lua.create_table()?;
            if let Some(inv) = w.inventory(this.0) {
                for (item, count) in inv.iter() {
                    table.set(item, count)?;
                }
            }
            Ok(table)
        });
        // ---- Camera2D screen<->world conversion ----
        m.add_method("ScreenToWorld", |lua, this, v: LuaValue| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            if !is_camera(&w, this.0) {
                return Err(mlua::Error::RuntimeError(
                    "ScreenToWorld can only be called on a Camera2D".to_string(),
                ));
            }
            let s = as_vec2(&v).ok_or_else(|| {
                mlua::Error::RuntimeError("ScreenToWorld expects a Vec2".to_string())
            })?;
            let (pos, zoom) = camera_view(&w, this.0);
            let viewport = input_handle(lua).borrow().viewport;
            LuaVec2(pos + (s - viewport * 0.5) / zoom).into_lua(lua)
        });
        m.add_method("WorldToScreen", |lua, this, v: LuaValue| {
            let rc = world_handle(lua);
            let w = rc.borrow();
            check(&w, this.0)?;
            if !is_camera(&w, this.0) {
                return Err(mlua::Error::RuntimeError(
                    "WorldToScreen can only be called on a Camera2D".to_string(),
                ));
            }
            let world = as_vec2(&v).ok_or_else(|| {
                mlua::Error::RuntimeError("WorldToScreen expects a Vec2".to_string())
            })?;
            let (pos, zoom) = camera_view(&w, this.0);
            let viewport = input_handle(lua).borrow().viewport;
            LuaVec2((world - pos) * zoom + viewport * 0.5).into_lua(lua)
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
        "IsPlaying" if is_animated_sprite(&w, id) => Ok(LuaValue::Boolean(matches!(
            w.get_prop(id, "Playing"),
            Some(Value::Bool(true))
        ))),
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
            let v = if key == "AbsolutePosition" {
                rect.min
            } else {
                rect.size
            };
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
        Value::Rect(r) => LuaRect(*r).into_lua(lua),
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
        ValueType::Rect => as_rect(v).map(Value::Rect).ok_or(got),
        ValueType::InstanceRef => match v {
            LuaValue::Nil => Ok(Value::InstanceRef(None)),
            v => as_instance(v)
                .map(|i| Value::InstanceRef(Some(i.0)))
                .ok_or(got),
        },
    }
}
