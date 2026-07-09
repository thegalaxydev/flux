use std::rc::Rc;

use flux_data::{DataError, PersistenceProvider};
use mlua::{Function, IntoLua, Lua, UserData, UserDataMethods, Value as LuaValue};
use serde_json::Value as Json;

pub type Provider = Rc<dyn PersistenceProvider>;

pub(crate) struct LuaDataStoreService {
    pub provider: Provider,
}

impl UserData for LuaDataStoreService {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("GetDataStore", |_, this, (name, scope): (String, Option<String>)| {
            Ok(LuaDataStore {
                provider: this.provider.clone(),
                scope: scope.unwrap_or_else(|| "global".to_string()),
                name,
            })
        });
    }
}

pub(crate) struct LuaDataStore {
    provider: Provider,
    scope: String,
    name: String,
}

impl UserData for LuaDataStore {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("GetAsync", |lua, this, key: String| {
            let value = this.provider.get(&this.scope, &this.name, &key).map_err(de)?;
            match value {
                Some(json) => json_to_lua(lua, &json),
                None => Ok(LuaValue::Nil),
            }
        });
        m.add_method("SetAsync", |_, this, (key, value): (String, LuaValue)| {
            let json = to_json(&value).map_err(mlua::Error::RuntimeError)?;
            this.provider.set(&this.scope, &this.name, &key, &json).map_err(de)?;
            Ok(())
        });
        m.add_method("RemoveAsync", |lua, this, key: String| {
            let old = this.provider.remove(&this.scope, &this.name, &key).map_err(de)?;
            match old {
                Some(json) => json_to_lua(lua, &json),
                None => Ok(LuaValue::Nil),
            }
        });
        m.add_method(
            "IncrementAsync",
            |_, this, (key, delta): (String, Option<f64>)| {
                this.provider
                    .increment(&this.scope, &this.name, &key, delta.unwrap_or(1.0))
                    .map_err(de)
            },
        );
        m.add_method("UpdateAsync", |lua, this, (key, callback): (String, Function)| {
            let value = this
                .provider
                .update(&this.scope, &this.name, &key, &mut |current| {
                    let arg = match current.as_ref() {
                        Some(json) => {
                            json_to_lua(lua, json).map_err(|e| DataError::Callback(e.to_string()))?
                        }
                        None => LuaValue::Nil,
                    };
                    let ret: LuaValue = callback
                        .call(arg)
                        .map_err(|e| DataError::Callback(e.to_string()))?;
                    match ret {
                        LuaValue::Nil => Ok(None),
                        other => Ok(Some(to_json(&other).map_err(DataError::Callback)?)),
                    }
                })
                .map_err(de)?;
            match value {
                Some(json) => json_to_lua(lua, &json),
                None => Ok(LuaValue::Nil),
            }
        });
        m.add_method("ListKeysAsync", |_, this, ()| {
            this.provider.list_keys(&this.scope, &this.name).map_err(de)
        });
    }
}

fn de(e: DataError) -> mlua::Error {
    mlua::Error::RuntimeError(e.to_string())
}

fn to_json(value: &LuaValue) -> Result<Json, String> {
    value_to_json(value, &mut Vec::new())
}

fn value_to_json(value: &LuaValue, stack: &mut Vec<usize>) -> Result<Json, String> {
    match value {
        LuaValue::Nil => Ok(Json::Null),
        LuaValue::Boolean(b) => Ok(Json::Bool(*b)),
        LuaValue::Integer(i) => Ok(Json::Number((*i).into())),
        LuaValue::Number(n) => serde_json::Number::from_f64(*n)
            .map(Json::Number)
            .ok_or_else(|| "cannot store a non-finite number in a DataStore".to_string()),
        LuaValue::String(s) => Ok(Json::String(s.to_string_lossy())),
        LuaValue::Table(t) => table_to_json(t, stack),
        other => Err(format!(
            "cannot store a {} value in a DataStore",
            other.type_name()
        )),
    }
}

fn table_to_json(t: &mlua::Table, stack: &mut Vec<usize>) -> Result<Json, String> {
    let ptr = t.to_pointer() as usize;
    if stack.contains(&ptr) {
        return Err("cannot store a cyclic table in a DataStore".to_string());
    }
    stack.push(ptr);

    let n = t.raw_len();
    let mut total = 0usize;
    let mut is_seq = n > 0;
    for pair in t.clone().pairs::<LuaValue, LuaValue>() {
        let (k, _) = pair.map_err(|e| e.to_string())?;
        total += 1;
        let contiguous = match &k {
            LuaValue::Integer(i) => *i >= 1 && (*i as usize) <= n,
            LuaValue::Number(f) => f.fract() == 0.0 && *f >= 1.0 && (*f as usize) <= n,
            _ => false,
        };
        if !contiguous {
            is_seq = false;
        }
    }

    let result = if is_seq && total == n {
        let mut arr = Vec::with_capacity(n);
        for i in 1..=n {
            let v: LuaValue = t.get(i).map_err(|e| e.to_string())?;
            arr.push(value_to_json(&v, stack)?);
        }
        Json::Array(arr)
    } else {
        let mut map = serde_json::Map::new();
        for pair in t.clone().pairs::<LuaValue, LuaValue>() {
            let (k, v) = pair.map_err(|e| e.to_string())?;
            let key = match k {
                LuaValue::String(s) => s.to_string_lossy(),
                LuaValue::Integer(i) => i.to_string(),
                LuaValue::Number(f) => f.to_string(),
                other => {
                    return Err(format!(
                        "cannot use a {} as a table key in a DataStore",
                        other.type_name()
                    ));
                }
            };
            map.insert(key, value_to_json(&v, stack)?);
        }
        Json::Object(map)
    };

    stack.pop();
    Ok(result)
}

fn json_to_lua(lua: &Lua, value: &Json) -> mlua::Result<LuaValue> {
    match value {
        Json::Null => Ok(LuaValue::Nil),
        Json::Bool(b) => Ok(LuaValue::Boolean(*b)),
        Json::Number(n) => match n.as_i64() {
            Some(i) => i.into_lua(lua),
            None => n.as_f64().unwrap_or(0.0).into_lua(lua),
        },
        Json::String(s) => s.as_str().into_lua(lua),
        Json::Array(a) => {
            let t = lua.create_table()?;
            for (i, e) in a.iter().enumerate() {
                t.set(i + 1, json_to_lua(lua, e)?)?;
            }
            Ok(LuaValue::Table(t))
        }
        Json::Object(o) => {
            let t = lua.create_table()?;
            for (k, e) in o {
                t.set(k.as_str(), json_to_lua(lua, e)?)?;
            }
            Ok(LuaValue::Table(t))
        }
    }
}
