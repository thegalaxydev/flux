//! The `SaveService`: named save slots for the running game.
//!
//! `game:GetService("SaveService")` returns this (like `DataStoreService`, it's
//! not a tree instance). `Save` writes the whole world — tree, placed buildings,
//! and modified tilemap grids — to `.flux/saves/<name>.save.json` via
//! [`flux_core::World::to_save_string`]. `Load` reuses the scene-swap machinery
//! ([`crate::SceneHandle`]) to reload that file, so the host applies it exactly
//! like a `Scene:Load`.

use std::path::PathBuf;

use mlua::{Lua, UserData, UserDataMethods};

pub(crate) struct LuaSaveService;

/// Reject anything that isn't a plain slot name so a save can't escape the saves
/// directory or collide with path separators.
fn sanitize(name: &str) -> mlua::Result<String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(name.to_string())
    } else {
        Err(mlua::Error::RuntimeError(format!(
            "invalid save name '{name}' (use letters, digits, '_' or '-')"
        )))
    }
}

fn rel(name: &str) -> String {
    format!(".flux/saves/{name}.save.json")
}

fn saves_dir(lua: &Lua) -> PathBuf {
    crate::asset_root(lua).join(".flux/saves")
}

impl UserData for LuaSaveService {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("Save", |lua, _, name: String| {
            let name = sanitize(&name)?;
            let json = crate::world_handle(lua).borrow().to_save_string();
            let dir = saves_dir(lua);
            std::fs::create_dir_all(&dir).map_err(io_err)?;
            std::fs::write(dir.join(format!("{name}.save.json")), json).map_err(io_err)?;
            Ok(())
        });
        m.add_method("Load", |lua, _, name: String| {
            let name = sanitize(&name)?;
            let path = crate::asset_root(lua).join(rel(&name));
            if !path.exists() {
                return Err(mlua::Error::RuntimeError(format!(
                    "no save named '{name}'"
                )));
            }
            // Defer the swap to the host, exactly like Scene:Load.
            crate::scene_handle(lua).borrow_mut().request = Some(rel(&name));
            Ok(())
        });
        m.add_method("Exists", |lua, _, name: String| {
            let name = sanitize(&name)?;
            Ok(crate::asset_root(lua).join(rel(&name)).exists())
        });
        m.add_method("Delete", |lua, _, name: String| {
            let name = sanitize(&name)?;
            let path = crate::asset_root(lua).join(rel(&name));
            Ok(std::fs::remove_file(path).is_ok())
        });
        m.add_method("List", |lua, _, ()| {
            let mut names = Vec::new();
            if let Ok(entries) = std::fs::read_dir(saves_dir(lua)) {
                for e in entries.flatten() {
                    let file = e.file_name();
                    let file = file.to_string_lossy();
                    if let Some(stem) = file.strip_suffix(".save.json") {
                        names.push(stem.to_string());
                    }
                }
            }
            names.sort();
            Ok(names)
        });
    }
}

fn io_err(e: std::io::Error) -> mlua::Error {
    mlua::Error::RuntimeError(format!("save failed: {e}"))
}
