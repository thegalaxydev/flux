use std::cell::RefCell;
use std::rc::{Rc, Weak};

use mlua::{Function, IntoLuaMulti, Lua, UserData, UserDataMethods};

use crate::Log;
use crate::scheduler::{Scheduler, resume_thread};

type Listeners = Rc<RefCell<Vec<Option<Function>>>>;

#[derive(Clone, Default)]
pub struct Signal {
    listeners: Listeners,
}

#[derive(Clone)]
pub struct LuaSignal(pub Signal);

impl UserData for LuaSignal {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("Connect", |_, this, f: Function| {
            let mut listeners = this.0.listeners.borrow_mut();
            listeners.push(Some(f));
            Ok(LuaConnection {
                listeners: Rc::downgrade(&this.0.listeners),
                index: listeners.len() - 1,
            })
        });
    }
}

pub struct LuaConnection {
    listeners: Weak<RefCell<Vec<Option<Function>>>>,
    index: usize,
}

impl UserData for LuaConnection {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("Disconnect", |_, this, ()| {
            if let Some(listeners) = this.listeners.upgrade() {
                if let Some(slot) = listeners.borrow_mut().get_mut(this.index) {
                    *slot = None;
                }
            }
            Ok(())
        });
    }
}

pub fn fire(
    lua: &Lua,
    scheduler: &Rc<RefCell<Scheduler>>,
    log: &Log,
    signal: &Signal,
    args: impl IntoLuaMulti,
) {
    let fns: Vec<Function> = signal
        .listeners
        .borrow()
        .iter()
        .flatten()
        .cloned()
        .collect();
    if fns.is_empty() {
        return;
    }
    let args = match args.into_lua_multi(lua) {
        Ok(a) => a,
        Err(e) => {
            log.error(format!("{e}"));
            return;
        }
    };
    for f in fns {
        match lua.create_thread(f) {
            Ok(thread) => resume_thread(lua, scheduler, log, thread, args.clone()),
            Err(e) => log.error(format!("{e}")),
        }
    }
}
