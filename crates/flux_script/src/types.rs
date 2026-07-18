use flux_core::{Color, Rect, UDim, UDim2};
use glam::Vec2;
use mlua::{
    Lua, MetaMethod, MultiValue, Table, UserData, UserDataFields, UserDataMethods, UserDataRef,
    Value as LuaValue,
};

#[derive(Clone, Copy)]
pub struct LuaVec2(pub Vec2);

pub fn as_vec2(v: &LuaValue) -> Option<Vec2> {
    v.as_userdata()
        .and_then(|u| u.borrow::<LuaVec2>().ok())
        .map(|r| r.0)
}

pub fn as_color(v: &LuaValue) -> Option<Color> {
    v.as_userdata()
        .and_then(|u| u.borrow::<LuaColor>().ok())
        .map(|r| r.0)
}

pub fn as_udim2(v: &LuaValue) -> Option<UDim2> {
    v.as_userdata()
        .and_then(|u| u.borrow::<LuaUDim2>().ok())
        .map(|r| r.0)
}

pub fn as_rect(v: &LuaValue) -> Option<Rect> {
    v.as_userdata()
        .and_then(|u| u.borrow::<LuaRect>().ok())
        .map(|r| r.0)
}

impl UserData for LuaVec2 {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("X", |_, v| Ok(v.0.x));
        f.add_field_method_get("Y", |_, v| Ok(v.0.y));
        f.add_field_method_get("Magnitude", |_, v| Ok(v.0.length()));
        f.add_field_method_get("Unit", |_, v| Ok(LuaVec2(v.0.normalize_or_zero())));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::Add, |_, a, b: UserDataRef<LuaVec2>| {
            Ok(LuaVec2(a.0 + b.0))
        });
        m.add_meta_method(MetaMethod::Sub, |_, a, b: UserDataRef<LuaVec2>| {
            Ok(LuaVec2(a.0 - b.0))
        });
        m.add_meta_function(MetaMethod::Mul, |_, (a, b): (LuaValue, LuaValue)| {
            let out = match (as_vec2(&a), a.as_number(), as_vec2(&b), b.as_number()) {
                (Some(va), _, Some(vb), _) => va * vb,
                (Some(va), _, None, Some(n)) => va * n as f32,
                (None, Some(n), Some(vb), _) => vb * n as f32,
                _ => {
                    return Err(mlua::Error::RuntimeError(
                        "invalid operands for Vec2 multiplication".to_string(),
                    ));
                }
            };
            Ok(LuaVec2(out))
        });
        m.add_meta_method(MetaMethod::Div, |_, a, b: f32| Ok(LuaVec2(a.0 / b)));
        m.add_meta_method(MetaMethod::Unm, |_, a, _: MultiValue| Ok(LuaVec2(-a.0)));
        m.add_meta_method(MetaMethod::Eq, |_, a, b: UserDataRef<LuaVec2>| {
            Ok(a.0 == b.0)
        });
        m.add_meta_method(MetaMethod::ToString, |_, a, ()| {
            Ok(format!("{}, {}", a.0.x, a.0.y))
        });
    }
}

pub fn vec2_table(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "new",
        lua.create_function(|_, (x, y): (Option<f32>, Option<f32>)| {
            Ok(LuaVec2(Vec2::new(x.unwrap_or(0.0), y.unwrap_or(0.0))))
        })?,
    )?;
    t.set("zero", LuaVec2(Vec2::ZERO))?;
    t.set("one", LuaVec2(Vec2::ONE))?;
    Ok(t)
}

#[derive(Clone, Copy)]
pub struct LuaRect(pub Rect);

impl UserData for LuaRect {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("X", |_, r| Ok(r.0.x));
        f.add_field_method_get("Y", |_, r| Ok(r.0.y));
        f.add_field_method_get("Width", |_, r| Ok(r.0.w));
        f.add_field_method_get("Height", |_, r| Ok(r.0.h));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::Eq, |_, a, b: UserDataRef<LuaRect>| Ok(a.0 == b.0));
        m.add_meta_method(MetaMethod::ToString, |_, r, ()| {
            Ok(format!("{}, {}, {}, {}", r.0.x, r.0.y, r.0.w, r.0.h))
        });
    }
}

pub fn rect_table(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "new",
        lua.create_function(
            |_, (x, y, w, h): (Option<f32>, Option<f32>, Option<f32>, Option<f32>)| {
                Ok(LuaRect(Rect::new(
                    x.unwrap_or(0.0),
                    y.unwrap_or(0.0),
                    w.unwrap_or(0.0),
                    h.unwrap_or(0.0),
                )))
            },
        )?,
    )?;
    Ok(t)
}

#[derive(Clone, Copy)]
pub struct LuaColor(pub Color);

impl UserData for LuaColor {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("R", |_, c| Ok(c.0.r));
        f.add_field_method_get("G", |_, c| Ok(c.0.g));
        f.add_field_method_get("B", |_, c| Ok(c.0.b));
        f.add_field_method_get("A", |_, c| Ok(c.0.a));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::Eq, |_, a, b: UserDataRef<LuaColor>| {
            Ok(a.0 == b.0)
        });
        m.add_meta_method(MetaMethod::ToString, |_, c, ()| {
            Ok(format!("{}, {}, {}, {}", c.0.r, c.0.g, c.0.b, c.0.a))
        });
    }
}

pub fn color_table(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "new",
        lua.create_function(
            |_, (r, g, b, a): (Option<f32>, Option<f32>, Option<f32>, Option<f32>)| {
                Ok(LuaColor(Color::new(
                    r.unwrap_or(0.0),
                    g.unwrap_or(0.0),
                    b.unwrap_or(0.0),
                    a.unwrap_or(1.0),
                )))
            },
        )?,
    )?;
    Ok(t)
}

#[derive(Clone, Copy)]
pub struct LuaUDim(pub UDim);

impl UserData for LuaUDim {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("Scale", |_, u| Ok(u.0.scale));
        f.add_field_method_get("Offset", |_, u| Ok(u.0.offset));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::Add, |_, a, b: UserDataRef<LuaUDim>| {
            Ok(LuaUDim(UDim::new(a.0.scale + b.0.scale, a.0.offset + b.0.offset)))
        });
        m.add_meta_method(MetaMethod::Sub, |_, a, b: UserDataRef<LuaUDim>| {
            Ok(LuaUDim(UDim::new(a.0.scale - b.0.scale, a.0.offset - b.0.offset)))
        });
        m.add_meta_method(MetaMethod::ToString, |_, u, ()| {
            Ok(format!("{}, {}", u.0.scale, u.0.offset))
        });
    }
}

pub fn udim_table(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "new",
        lua.create_function(|_, (scale, offset): (Option<f32>, Option<f32>)| {
            Ok(LuaUDim(UDim::new(scale.unwrap_or(0.0), offset.unwrap_or(0.0))))
        })?,
    )?;
    Ok(t)
}

#[derive(Clone, Copy)]
pub struct LuaUDim2(pub UDim2);

impl UserData for LuaUDim2 {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("X", |_, u| Ok(LuaUDim(u.0.x)));
        f.add_field_method_get("Y", |_, u| Ok(LuaUDim(u.0.y)));
        // Width/Height aliases read naturally when the UDim2 is a Size.
        f.add_field_method_get("Width", |_, u| Ok(LuaUDim(u.0.x)));
        f.add_field_method_get("Height", |_, u| Ok(LuaUDim(u.0.y)));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::Add, |_, a, b: UserDataRef<LuaUDim2>| {
            Ok(LuaUDim2(UDim2::new(
                a.0.x.scale + b.0.x.scale,
                a.0.x.offset + b.0.x.offset,
                a.0.y.scale + b.0.y.scale,
                a.0.y.offset + b.0.y.offset,
            )))
        });
        m.add_meta_method(MetaMethod::Sub, |_, a, b: UserDataRef<LuaUDim2>| {
            Ok(LuaUDim2(UDim2::new(
                a.0.x.scale - b.0.x.scale,
                a.0.x.offset - b.0.x.offset,
                a.0.y.scale - b.0.y.scale,
                a.0.y.offset - b.0.y.offset,
            )))
        });
        m.add_meta_method(MetaMethod::Eq, |_, a, b: UserDataRef<LuaUDim2>| {
            Ok(a.0 == b.0)
        });
        m.add_meta_method(MetaMethod::ToString, |_, u, ()| {
            Ok(format!(
                "{}, {}, {}, {}",
                u.0.x.scale, u.0.x.offset, u.0.y.scale, u.0.y.offset
            ))
        });
    }
}

pub fn udim2_table(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "new",
        lua.create_function(
            |_, (xs, xo, ys, yo): (Option<f32>, Option<f32>, Option<f32>, Option<f32>)| {
                Ok(LuaUDim2(UDim2::new(
                    xs.unwrap_or(0.0),
                    xo.unwrap_or(0.0),
                    ys.unwrap_or(0.0),
                    yo.unwrap_or(0.0),
                )))
            },
        )?,
    )?;
    t.set(
        "fromScale",
        lua.create_function(|_, (x, y): (Option<f32>, Option<f32>)| {
            Ok(LuaUDim2(UDim2::from_scale(x.unwrap_or(0.0), y.unwrap_or(0.0))))
        })?,
    )?;
    t.set(
        "fromOffset",
        lua.create_function(|_, (x, y): (Option<f32>, Option<f32>)| {
            Ok(LuaUDim2(UDim2::from_offset(x.unwrap_or(0.0), y.unwrap_or(0.0))))
        })?,
    )?;
    Ok(t)
}
