use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Function, IntoLuaMulti, Lua, MultiValue, Table, Thread, ThreadStatus};

use crate::Log;

#[derive(Default)]
pub struct Scheduler {
    pub now: f64,
    sleeping: Vec<Sleeper>,
    deferred: Vec<(Thread, MultiValue)>,
}

struct Sleeper {
    thread: Thread,
    wake_at: f64,
    started: f64,
}

pub fn resume_thread(
    _lua: &Lua,
    scheduler: &Rc<RefCell<Scheduler>>,
    log: &Log,
    thread: Thread,
    args: MultiValue,
) {
    match thread.resume::<MultiValue>(args) {
        Ok(vals) => {
            if thread.status() == ThreadStatus::Resumable {
                let secs = vals
                    .iter()
                    .next()
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .max(0.0);
                let mut s = scheduler.borrow_mut();
                let now = s.now;
                s.sleeping.push(Sleeper {
                    thread,
                    wake_at: now + secs,
                    started: now,
                });
            }
        }
        Err(e) => log.error(format!("{e}")),
    }
}

pub fn step(lua: &Lua, scheduler: &Rc<RefCell<Scheduler>>, log: &Log, dt: f64) {
    let (deferred, due) = {
        let mut s = scheduler.borrow_mut();
        s.now += dt;
        let now = s.now;
        let deferred = std::mem::take(&mut s.deferred);
        let mut due = Vec::new();
        let mut i = 0;
        while i < s.sleeping.len() {
            if s.sleeping[i].wake_at <= now {
                due.push(s.sleeping.swap_remove(i));
            } else {
                i += 1;
            }
        }
        (deferred, due)
    };
    for (thread, args) in deferred {
        resume_thread(lua, scheduler, log, thread, args);
    }
    let now = scheduler.borrow().now;
    for sleeper in due {
        let elapsed = now - sleeper.started;
        match elapsed.into_lua_multi(lua) {
            Ok(args) => resume_thread(lua, scheduler, log, sleeper.thread, args),
            Err(e) => log.error(format!("{e}")),
        }
    }
}

pub fn task_table(lua: &Lua, scheduler: Rc<RefCell<Scheduler>>) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    let wait: Function = lua
        .load("return function(t) return coroutine.yield(t or 0) end")
        .eval()?;
    t.set("wait", wait)?;

    let sch = scheduler.clone();
    t.set(
        "spawn",
        lua.create_function(move |lua, (f, args): (Function, MultiValue)| {
            let thread = lua.create_thread(f)?;
            let log = lua.app_data_ref::<Log>().expect("log app data").clone();
            resume_thread(lua, &sch, &log, thread.clone(), args);
            Ok(thread)
        })?,
    )?;

    let sch = scheduler;
    t.set(
        "defer",
        lua.create_function(move |lua, (f, args): (Function, MultiValue)| {
            let thread = lua.create_thread(f)?;
            sch.borrow_mut().deferred.push((thread.clone(), args));
            Ok(thread)
        })?,
    )?;

    Ok(t)
}
