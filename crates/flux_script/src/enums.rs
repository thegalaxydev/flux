//! Roblox-style `Enum` values exposed to scripts.
//!
//! Scripts refer to enum members as `Enum.KeyCode.A`, `Enum.UserInputType.MouseButton1`,
//! etc. Each member is an [`EnumItem`] userdata carrying a `Name`, a `Value`, and the
//! `EnumType` it belongs to — mirroring Roblox's `EnumItem`.
//!
//! Members that describe a physical input also carry an internal `token`: the string the
//! input layer records in [`crate::InputState`]. `KeyCode` tokens are the debug names of
//! egui's `Key` variants (see the player/editor input collectors); mouse tokens are
//! `"Left"`/`"Right"`/`"Middle"`. This lets `Input.IsKeyDown(Enum.KeyCode.A)` resolve to
//! the same lookup as the legacy `Input.IsKeyDown("A")`.

use std::rc::Rc;

use mlua::{
    Lua, MetaMethod, Table, UserData, UserDataFields, UserDataMethods, UserDataRef,
    Value as LuaValue,
};

/// A single enum member, e.g. `Enum.KeyCode.Space`.
#[derive(Clone)]
pub struct EnumItem {
    pub enum_type: String,
    pub name: String,
    pub value: i64,
    /// Input-matching token, when this member maps to a physical input.
    pub token: Option<String>,
}

impl UserData for EnumItem {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("Name", |_, i| Ok(i.name.clone()));
        f.add_field_method_get("Value", |_, i| Ok(i.value));
        f.add_field_method_get("EnumType", |_, i| Ok(i.enum_type.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, i, ()| {
            Ok(format!("Enum.{}.{}", i.enum_type, i.name))
        });
        m.add_meta_method(MetaMethod::Eq, |_, a, b: UserDataRef<EnumItem>| {
            Ok(a.enum_type == b.enum_type && a.value == b.value)
        });
    }
}

/// Resolve a Lua value (a `String` or a `KeyCode`/`UserInputType` [`EnumItem`]) to the
/// token recorded by the input layer. Returns `None` for other types.
pub fn resolve_input_token(v: &LuaValue) -> Option<String> {
    match v {
        LuaValue::String(s) => Some(s.to_str().ok()?.to_string()),
        LuaValue::UserData(u) => {
            let item = u.borrow::<EnumItem>().ok()?;
            Some(item.token.clone().unwrap_or_else(|| item.name.clone()))
        }
        _ => None,
    }
}

struct Def {
    name: String,
    value: i64,
    token: Option<String>,
}

fn def(name: impl Into<String>, value: i64) -> Def {
    Def {
        name: name.into(),
        value,
        token: None,
    }
}

/// A member whose `Name` doubles as its input token (letters, `Space`, arrows, ...).
fn key(name: &str, value: i64) -> Def {
    Def {
        name: name.to_string(),
        value,
        token: Some(name.to_string()),
    }
}

/// A member whose display name differs from the input token it matches.
fn keyed(name: impl Into<String>, value: i64, token: impl Into<String>) -> Def {
    Def {
        name: name.into(),
        value,
        token: Some(token.into()),
    }
}

fn keycode_defs() -> Vec<Def> {
    let mut defs = Vec::new();

    // Letters: Name and token are the egui variant ("A"); Value is the lowercase
    // ASCII codepoint, matching Roblox (Enum.KeyCode.A.Value == 97).
    for c in b'A'..=b'Z' {
        let name = (c as char).to_string();
        defs.push(key(&name, (c + 32) as i64));
    }

    // Digits: Roblox names them Zero..Nine; egui reports them as Num0..Num9.
    const DIGITS: [&str; 10] = [
        "Zero", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine",
    ];
    for (d, name) in DIGITS.iter().enumerate() {
        defs.push(keyed(*name, 48 + d as i64, format!("Num{d}")));
    }

    // Function keys F1..F12 (egui goes higher, but these are the common ones).
    for n in 1..=12 {
        defs.push(key(&format!("F{n}"), 1000 + n as i64));
    }

    // Whitespace / control keys. Roblox names on the left, egui token on the right.
    defs.push(keyed("Space", 32, "Space"));
    defs.push(keyed("Return", 13, "Enter"));
    defs.push(keyed("Tab", 9, "Tab"));
    defs.push(keyed("Backspace", 8, "Backspace"));
    defs.push(keyed("Escape", 27, "Escape"));
    defs.push(keyed("Delete", 127, "Delete"));
    defs.push(keyed("Insert", 45, "Insert"));
    defs.push(keyed("Home", 36, "Home"));
    defs.push(keyed("End", 35, "End"));
    defs.push(keyed("PageUp", 33, "PageUp"));
    defs.push(keyed("PageDown", 34, "PageDown"));

    // Arrow keys: Roblox uses Up/Down/Left/Right; egui uses Arrow* variants.
    defs.push(keyed("Up", 1017, "ArrowUp"));
    defs.push(keyed("Down", 1015, "ArrowDown"));
    defs.push(keyed("Left", 1013, "ArrowLeft"));
    defs.push(keyed("Right", 1012, "ArrowRight"));

    // Punctuation. Roblox names on the left, egui token on the right.
    defs.push(keyed("Minus", 45, "Minus"));
    defs.push(keyed("Equals", 61, "Equals"));
    defs.push(keyed("Plus", 43, "Plus"));
    defs.push(keyed("Comma", 44, "Comma"));
    defs.push(keyed("Period", 46, "Period"));
    defs.push(keyed("Slash", 47, "Slash"));
    defs.push(keyed("BackSlash", 92, "Backslash"));
    defs.push(keyed("Semicolon", 59, "Semicolon"));
    defs.push(keyed("Quote", 39, "Quote"));
    defs.push(keyed("Backquote", 96, "Backtick"));
    defs.push(keyed("LeftBracket", 91, "OpenBracket"));
    defs.push(keyed("RightBracket", 93, "CloseBracket"));

    defs
}

fn user_input_type_defs() -> Vec<Def> {
    vec![
        keyed("MouseButton1", 0, "Left"),
        keyed("MouseButton2", 1, "Right"),
        keyed("MouseButton3", 2, "Middle"),
        def("MouseMovement", 4),
        def("Keyboard", 8),
        def("Focus", 9),
        def("Touch", 10),
    ]
}

fn build_enum(lua: &Lua, type_name: &str, defs: Vec<Def>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for d in &defs {
        t.set(
            d.name.as_str(),
            EnumItem {
                enum_type: type_name.to_string(),
                name: d.name.clone(),
                value: d.value,
                token: d.token.clone(),
            },
        )?;
    }

    // `Enum.KeyCode:GetEnumItems()` returns a fresh array of every member.
    let items: Rc<Vec<(String, i64, Option<String>)>> = Rc::new(
        defs.into_iter()
            .map(|d| (d.name, d.value, d.token))
            .collect(),
    );
    let owner = type_name.to_string();
    t.set(
        "GetEnumItems",
        lua.create_function(move |lua, _: LuaValue| {
            let arr = lua.create_table()?;
            for (i, (name, value, token)) in items.iter().enumerate() {
                arr.set(
                    i + 1,
                    EnumItem {
                        enum_type: owner.clone(),
                        name: name.clone(),
                        value: *value,
                        token: token.clone(),
                    },
                )?;
            }
            Ok(arr)
        })?,
    )?;
    Ok(t)
}

/// Build the global `Enum` table. Unknown members raise a Roblox-style error.
pub fn enum_table(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("KeyCode", build_enum(lua, "KeyCode", keycode_defs())?)?;
    t.set(
        "UserInputType",
        build_enum(lua, "UserInputType", user_input_type_defs())?,
    )?;

    let mt = lua.create_table()?;
    mt.set(
        "__index",
        lua.create_function(|_, (_, key): (Table, String)| -> mlua::Result<LuaValue> {
            Err(mlua::Error::RuntimeError(format!(
                "'{key}' is not a valid Enum"
            )))
        })?,
    )?;
    t.set_metatable(Some(mt))?;
    Ok(t)
}
