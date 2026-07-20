//! The game's Lua API, registered through `flux_script`'s plugin seams:
//! `Tilemap:PlaceBuilding/CanPlace/GetBuildingAt/RemoveBuilding/GetPower` and
//! `Building:GetItem/AddItem/TakeItem/ItemTotal/GetInventory`.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{FromLuaMulti, IntoLuaMulti, Lua};

use flux_core::{InstanceId, Value, World};
use flux_script::api::{self, LuaInstance};

use crate::building::{self, BuildingCatalog, BuildingCatalogCache};
use crate::factory::Inventory;

/// The building catalog cache, kept in Lua app-data (set per session).
type BuildingCacheHandle = Rc<RefCell<BuildingCatalogCache>>;

fn is_tilemap(w: &World, id: InstanceId) -> bool {
    w.class_name(id) == Some("Tilemap")
}
fn is_building(w: &World, id: InstanceId) -> bool {
    w.class_name(id) == Some("Building")
}
fn err(msg: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(msg.into())
}

fn catalog_of(lua: &Lua, w: &World, id: InstanceId) -> Option<Rc<BuildingCatalog>> {
    let path = match w.get_prop(id, "Buildings") {
        Some(Value::Asset(s)) if !s.is_empty() => s.clone(),
        _ => return None,
    };
    let cache = lua.app_data_ref::<BuildingCacheHandle>()?.clone();
    let root = api::asset_root(lua);
    let cat = cache.borrow_mut().get(&path, &root);
    cat
}

pub(crate) fn install() {
    // Each session gets its own building-catalog cache in app-data.
    api::register_session_init(|lua| {
        lua.set_app_data::<BuildingCacheHandle>(Rc::new(RefCell::new(
            BuildingCatalogCache::default(),
        )));
    });

    api::register_method("CanPlace", |lua, id, args| {
        let (ty, col, row) = <(String, i32, i32)>::from_lua_multi(args, lua)?;
        let rc = api::world(lua);
        let w = rc.borrow();
        if !is_tilemap(&w, id) {
            return Err(err("CanPlace can only be called on a Tilemap"));
        }
        let ok = catalog_of(lua, &w, id)
            .and_then(|cat| cat.get(&ty).map(|def| building::can_place(&w, id, def, col, row)))
            .unwrap_or(false);
        ok.into_lua_multi(lua)
    });

    api::register_method("PlaceBuilding", |lua, id, args| {
        let (ty, col, row) = <(String, i32, i32)>::from_lua_multi(args, lua)?;
        let cat = {
            let rc = api::world(lua);
            let w = rc.borrow();
            if !is_tilemap(&w, id) {
                return Err(err("PlaceBuilding can only be called on a Tilemap"));
            }
            catalog_of(lua, &w, id)
                .ok_or_else(|| err("PlaceBuilding: Tilemap has no Buildings catalog"))?
        };
        let def = cat
            .get(&ty)
            .ok_or_else(|| err(format!("PlaceBuilding: unknown building '{ty}'")))?;
        let rc = api::world(lua);
        let mut w = rc.borrow_mut();
        building::place(&mut w, id, def, col, row)
            .map(LuaInstance)
            .into_lua_multi(lua)
    });

    api::register_method("BuildingTypes", |lua, id, args| {
        <()>::from_lua_multi(args, lua)?;
        let rc = api::world(lua);
        let w = rc.borrow();
        if !is_tilemap(&w, id) {
            return Err(err("BuildingTypes can only be called on a Tilemap"));
        }
        let list = lua.create_table()?;
        if let Some(cat) = catalog_of(lua, &w, id) {
            for def in cat.defs() {
                let entry = lua.create_table()?;
                entry.set("id", def.id.clone())?;
                entry.set("name", def.name.clone())?;
                entry.set("category", def.category.clone())?;
                entry.set("cost", def.cost)?;
                list.push(entry)?;
            }
        }
        list.into_lua_multi(lua)
    });

    api::register_method("GetBuildingAt", |lua, id, args| {
        let (col, row) = <(i32, i32)>::from_lua_multi(args, lua)?;
        let rc = api::world(lua);
        let w = rc.borrow();
        if !is_tilemap(&w, id) {
            return Err(err("GetBuildingAt can only be called on a Tilemap"));
        }
        building::building_at(&w, id, col, row)
            .map(LuaInstance)
            .into_lua_multi(lua)
    });

    api::register_method("RemoveBuilding", |lua, id, args| {
        let (col, row) = <(i32, i32)>::from_lua_multi(args, lua)?;
        let rc = api::world(lua);
        let mut w = rc.borrow_mut();
        if !is_tilemap(&w, id) {
            return Err(err("RemoveBuilding can only be called on a Tilemap"));
        }
        building::remove_at(&mut w, id, col, row).into_lua_multi(lua)
    });

    api::register_method("GetPower", |lua, id, args| {
        <()>::from_lua_multi(args, lua)?;
        let rc = api::world(lua);
        let w = rc.borrow();
        if !is_tilemap(&w, id) {
            return Err(err("GetPower can only be called on a Tilemap"));
        }
        let num = |name| match w.get_prop(id, name) {
            Some(Value::Number(n)) => *n,
            _ => 0.0,
        };
        (num("_PowerProduced"), num("_PowerConsumed")).into_lua_multi(lua)
    });

    api::register_method("GetItem", |lua, id, args| {
        let item = <String>::from_lua_multi(args, lua)?;
        let rc = api::world(lua);
        let w = rc.borrow();
        if !is_building(&w, id) {
            return Err(err("GetItem can only be called on a Building"));
        }
        w.component::<Inventory>(id)
            .map(|i| i.count(&item))
            .unwrap_or(0)
            .into_lua_multi(lua)
    });

    api::register_method("AddItem", |lua, id, args| {
        let (item, n) = <(String, u32)>::from_lua_multi(args, lua)?;
        let rc = api::world(lua);
        let mut w = rc.borrow_mut();
        if !is_building(&w, id) {
            return Err(err("AddItem can only be called on a Building"));
        }
        w.component_mut::<Inventory>(id)
            .map(|i| i.add(&item, n))
            .unwrap_or(0)
            .into_lua_multi(lua)
    });

    api::register_method("TakeItem", |lua, id, args| {
        let (item, n) = <(String, u32)>::from_lua_multi(args, lua)?;
        let rc = api::world(lua);
        let mut w = rc.borrow_mut();
        if !is_building(&w, id) {
            return Err(err("TakeItem can only be called on a Building"));
        }
        w.component_mut::<Inventory>(id)
            .map(|i| i.take(&item, n))
            .unwrap_or(0)
            .into_lua_multi(lua)
    });

    api::register_method("ItemTotal", |lua, id, args| {
        <()>::from_lua_multi(args, lua)?;
        let rc = api::world(lua);
        let w = rc.borrow();
        if !is_building(&w, id) {
            return Err(err("ItemTotal can only be called on a Building"));
        }
        w.component::<Inventory>(id)
            .map(|i| i.total())
            .unwrap_or(0)
            .into_lua_multi(lua)
    });

    api::register_method("GetInventory", |lua, id, args| {
        <()>::from_lua_multi(args, lua)?;
        let rc = api::world(lua);
        let w = rc.borrow();
        if !is_building(&w, id) {
            return Err(err("GetInventory can only be called on a Building"));
        }
        let table = lua.create_table()?;
        if let Some(inv) = w.component::<Inventory>(id) {
            for (item, count) in inv.iter() {
                table.set(item, count)?;
            }
        }
        table.into_lua_multi(lua)
    });
}
