use std::collections::HashMap;

use flux_core::{CoreError, InstanceId, Subtree, Value, World, registry};

pub type RemapMap = HashMap<InstanceId, InstanceId>;

pub enum Command {
    SetProp {
        id: InstanceId,
        prop: &'static str,
        old: Value,
        new: Value,
    },
    /// Several props of one instance set atomically as a single undo step. Used
    /// by editor transforms (move/resize/rotate) so a whole drag is one step.
    SetProps {
        id: InstanceId,
        entries: Vec<(&'static str, Value, Value)>,
    },
    Rename {
        id: InstanceId,
        old: String,
        new: String,
    },
    Create {
        class_name: &'static str,
        parent: InstanceId,
        created: Option<InstanceId>,
    },
    /// Create an instance and set some props/attributes in one undo step (e.g.
    /// drop an asset to make a Sprite with a Texture, or a Tilemap carrying a
    /// plugin catalog attribute).
    CreateWith {
        class_name: &'static str,
        parent: InstanceId,
        props: Vec<(&'static str, Value)>,
        attrs: Vec<(String, Value)>,
        created: Option<InstanceId>,
    },
    Delete {
        id: InstanceId,
        parent: InstanceId,
        index: usize,
        snapshot: Option<Subtree>,
    },
    Insert {
        parent: InstanceId,
        index: usize,
        subtree: Subtree,
        created: Option<InstanceId>,
    },
    Reparent {
        id: InstanceId,
        new_parent: InstanceId,
        index: Option<usize>,
        old_parent: InstanceId,
        old_index: usize,
    },
    /// Set (`Some`) or remove (`None`) a free-form attribute.
    SetAttribute {
        id: InstanceId,
        name: String,
        old: Option<Value>,
        new: Option<Value>,
    },
    AddTag {
        id: InstanceId,
        tag: String,
    },
    RemoveTag {
        id: InstanceId,
        tag: String,
    },
}

impl Command {
    pub fn set_prop(id: InstanceId, prop: &'static str, old: Value, new: Value) -> Self {
        Command::SetProp { id, prop, old, new }
    }

    pub fn set_props(id: InstanceId, entries: Vec<(&'static str, Value, Value)>) -> Self {
        Command::SetProps { id, entries }
    }

    pub fn rename(id: InstanceId, old: String, new: String) -> Self {
        Command::Rename { id, old, new }
    }

    pub fn create(class_name: &'static str, parent: InstanceId) -> Self {
        Command::Create {
            class_name,
            parent,
            created: None,
        }
    }

    pub fn create_with(
        class_name: &'static str,
        parent: InstanceId,
        props: Vec<(&'static str, Value)>,
    ) -> Self {
        Command::CreateWith {
            class_name,
            parent,
            props,
            attrs: Vec::new(),
            created: None,
        }
    }

    pub fn create_with_attrs(
        class_name: &'static str,
        parent: InstanceId,
        props: Vec<(&'static str, Value)>,
        attrs: Vec<(String, Value)>,
    ) -> Self {
        Command::CreateWith {
            class_name,
            parent,
            props,
            attrs,
            created: None,
        }
    }

    pub fn delete(id: InstanceId) -> Self {
        Command::Delete {
            id,
            parent: id,
            index: 0,
            snapshot: None,
        }
    }

    pub fn duplicate(world: &World, id: InstanceId) -> Option<Self> {
        let parent = world.parent(id)?;
        if registry().info(world.class_of(id)?).service {
            return None;
        }
        Some(Command::Insert {
            parent,
            index: world.child_index(id)? + 1,
            subtree: world.snapshot_subtree(id)?,
            created: None,
        })
    }

    pub fn set_attribute(id: InstanceId, name: String, old: Option<Value>, new: Option<Value>) -> Self {
        Command::SetAttribute { id, name, old, new }
    }

    pub fn reparent_at(id: InstanceId, new_parent: InstanceId, index: usize) -> Self {
        Command::Reparent {
            id,
            new_parent,
            index: Some(index),
            old_parent: id,
            old_index: 0,
        }
    }

    fn exec(&mut self, w: &mut World) -> Result<Option<RemapMap>, CoreError> {
        match self {
            Command::SetProp { id, prop, new, .. } => {
                w.set_prop(*id, prop, new.clone())?;
                Ok(None)
            }
            Command::SetProps { id, entries } => {
                for (prop, _, new) in entries.iter() {
                    w.set_prop(*id, prop, new.clone())?;
                }
                Ok(None)
            }
            Command::Rename { id, new, .. } => {
                w.set_name(*id, new.clone())?;
                Ok(None)
            }
            Command::Create {
                class_name,
                parent,
                created,
            } => {
                let id = w.create(class_name, *parent)?;
                let map = created.map(|prev| RemapMap::from([(prev, id)]));
                *created = Some(id);
                Ok(map)
            }
            Command::CreateWith {
                class_name,
                parent,
                props,
                attrs,
                created,
            } => {
                let id = w.create(class_name, *parent)?;
                for (prop, value) in props.iter() {
                    w.set_prop(id, prop, value.clone())?;
                }
                for (name, value) in attrs.iter() {
                    w.set_attribute(id, name, value.clone())?;
                }
                let map = created.map(|prev| RemapMap::from([(prev, id)]));
                *created = Some(id);
                Ok(map)
            }
            Command::Delete {
                id,
                parent,
                index,
                snapshot,
            } => {
                *parent = w.parent(*id).ok_or(CoreError::InstanceNotFound)?;
                *index = w.child_index(*id).unwrap_or(0);
                *snapshot = w.snapshot_subtree(*id);
                w.destroy(*id)?;
                Ok(None)
            }
            Command::Insert {
                parent,
                index,
                subtree,
                created,
            } => {
                let map = w.restore_subtree(*parent, *index, subtree)?;
                let out = created.is_some().then(|| map.clone());
                let root_new = map[&subtree.root_id()];
                subtree.remap(&map);
                *created = Some(root_new);
                Ok(out)
            }
            Command::Reparent {
                id,
                new_parent,
                index,
                old_parent,
                old_index,
            } => {
                *old_parent = w.parent(*id).ok_or(CoreError::InstanceNotFound)?;
                *old_index = w.child_index(*id).unwrap_or(0);
                match index {
                    Some(i) => w.reparent_at(*id, *new_parent, *i)?,
                    None => w.reparent(*id, *new_parent)?,
                }
                Ok(None)
            }
            Command::SetAttribute { id, name, new, .. } => {
                match new.clone() {
                    Some(v) => w.set_attribute(*id, name, v)?,
                    None => w.remove_attribute(*id, name),
                }
                Ok(None)
            }
            Command::AddTag { id, tag } => {
                w.add_tag(*id, tag);
                Ok(None)
            }
            Command::RemoveTag { id, tag } => {
                w.remove_tag(*id, tag);
                Ok(None)
            }
        }
    }

    fn unexec(&mut self, w: &mut World) -> Result<Option<RemapMap>, CoreError> {
        match self {
            Command::SetProp { id, prop, old, .. } => {
                w.set_prop(*id, prop, old.clone())?;
                Ok(None)
            }
            Command::SetProps { id, entries } => {
                for (prop, old, _) in entries.iter().rev() {
                    w.set_prop(*id, prop, old.clone())?;
                }
                Ok(None)
            }
            Command::Rename { id, old, .. } => {
                w.set_name(*id, old.clone())?;
                Ok(None)
            }
            Command::Create { created, .. } => {
                w.destroy(created.ok_or(CoreError::InstanceNotFound)?)?;
                Ok(None)
            }
            Command::CreateWith { created, .. } => {
                w.destroy(created.ok_or(CoreError::InstanceNotFound)?)?;
                Ok(None)
            }
            Command::Delete {
                parent,
                index,
                snapshot,
                ..
            } => {
                let snap = snapshot.as_ref().ok_or(CoreError::InstanceNotFound)?;
                let map = w.restore_subtree(*parent, *index, snap)?;
                snapshot.as_mut().unwrap().remap(&map);
                Ok(Some(map))
            }
            Command::Insert { created, .. } => {
                w.destroy(created.ok_or(CoreError::InstanceNotFound)?)?;
                Ok(None)
            }
            Command::Reparent {
                id,
                old_parent,
                old_index,
                ..
            } => {
                w.reparent_at(*id, *old_parent, *old_index)?;
                Ok(None)
            }
            Command::SetAttribute { id, name, old, .. } => {
                match old.clone() {
                    Some(v) => w.set_attribute(*id, name, v)?,
                    None => w.remove_attribute(*id, name),
                }
                Ok(None)
            }
            Command::AddTag { id, tag } => {
                w.remove_tag(*id, tag);
                Ok(None)
            }
            Command::RemoveTag { id, tag } => {
                w.add_tag(*id, tag);
                Ok(None)
            }
        }
    }

    fn remap(&mut self, map: &RemapMap) {
        let fix = |id: &mut InstanceId| {
            if let Some(new) = map.get(id) {
                *id = *new;
            }
        };
        let fix_value = |v: &mut Value| {
            if let Value::InstanceRef(Some(t)) = v {
                if let Some(new) = map.get(t) {
                    *t = *new;
                }
            }
        };
        match self {
            Command::SetProp { id, old, new, .. } => {
                fix(id);
                fix_value(old);
                fix_value(new);
            }
            Command::SetProps { id, entries } => {
                fix(id);
                for (_, old, new) in entries.iter_mut() {
                    fix_value(old);
                    fix_value(new);
                }
            }
            Command::Rename { id, .. } => fix(id),
            Command::Create {
                parent, created, ..
            } => {
                fix(parent);
                if let Some(c) = created {
                    fix(c);
                }
            }
            Command::CreateWith {
                parent,
                props,
                created,
                ..
            } => {
                fix(parent);
                for (_, value) in props.iter_mut() {
                    fix_value(value);
                }
                if let Some(c) = created {
                    fix(c);
                }
            }
            Command::Delete {
                id,
                parent,
                snapshot,
                ..
            } => {
                fix(id);
                fix(parent);
                if let Some(s) = snapshot {
                    s.remap(map);
                }
            }
            Command::Insert {
                parent,
                subtree,
                created,
                ..
            } => {
                fix(parent);
                subtree.remap(map);
                if let Some(c) = created {
                    fix(c);
                }
            }
            Command::Reparent {
                id,
                new_parent,
                old_parent,
                ..
            } => {
                fix(id);
                fix(new_parent);
                fix(old_parent);
            }
            Command::SetAttribute { id, .. } => fix(id),
            Command::AddTag { id, .. } | Command::RemoveTag { id, .. } => fix(id),
        }
    }

    fn try_merge(&mut self, next: &Command) -> bool {
        match (self, next) {
            (
                Command::SetProp { id, prop, new, .. },
                Command::SetProp {
                    id: id2,
                    prop: prop2,
                    new: new2,
                    ..
                },
            ) if id == id2 && prop == prop2 => {
                *new = new2.clone();
                true
            }
            (
                Command::SetProps { id, entries },
                Command::SetProps {
                    id: id2,
                    entries: entries2,
                },
            ) if id == id2
                && entries.len() == entries2.len()
                && entries
                    .iter()
                    .zip(entries2.iter())
                    .all(|(a, b)| a.0 == b.0) =>
            {
                // Same instance and same prop set: fold the newer target values in.
                for (dst, src) in entries.iter_mut().zip(entries2.iter()) {
                    dst.2 = src.2.clone();
                }
                true
            }
            (
                Command::Rename { id, new, .. },
                Command::Rename {
                    id: id2, new: new2, ..
                },
            ) if id == id2 => {
                *new = new2.clone();
                true
            }
            (
                Command::SetAttribute { id, name, new, .. },
                Command::SetAttribute {
                    id: id2,
                    name: name2,
                    new: new2,
                    ..
                },
            ) if id == id2 && name == name2 => {
                *new = new2.clone();
                true
            }
            _ => false,
        }
    }
}

pub fn apply_ephemeral(world: &mut World, mut cmd: Command) -> Result<(), CoreError> {
    cmd.exec(world).map(|_| ())
}

#[derive(Default)]
pub struct History {
    undo: Vec<Command>,
    redo: Vec<Command>,
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn apply(
        &mut self,
        world: &mut World,
        mut cmd: Command,
        merge: bool,
    ) -> Result<Option<RemapMap>, CoreError> {
        let map = cmd.exec(world)?;
        if let Some(m) = &map {
            self.remap(m);
        }
        let merged = merge && self.undo.last_mut().is_some_and(|top| top.try_merge(&cmd));
        if !merged {
            self.undo.push(cmd);
        }
        self.redo.clear();
        Ok(map)
    }

    pub fn undo(&mut self, world: &mut World) -> Result<Option<RemapMap>, CoreError> {
        let Some(mut cmd) = self.undo.pop() else {
            return Ok(None);
        };
        match cmd.unexec(world) {
            Ok(map) => {
                self.redo.push(cmd);
                if let Some(m) = &map {
                    self.remap(m);
                }
                Ok(map)
            }
            Err(e) => {
                self.undo.push(cmd);
                Err(e)
            }
        }
    }

    pub fn redo(&mut self, world: &mut World) -> Result<Option<RemapMap>, CoreError> {
        let Some(mut cmd) = self.redo.pop() else {
            return Ok(None);
        };
        match cmd.exec(world) {
            Ok(map) => {
                self.undo.push(cmd);
                if let Some(m) = &map {
                    self.remap(m);
                }
                Ok(map)
            }
            Err(e) => {
                self.redo.push(cmd);
                Err(e)
            }
        }
    }

    fn remap(&mut self, map: &RemapMap) {
        for cmd in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            cmd.remap(map);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    fn sprite_child(w: &World, parent: InstanceId, name: &str) -> InstanceId {
        w.find_first_child(parent, name).unwrap()
    }

    #[test]
    fn redo_across_create_remaps_later_commands() {
        let mut w = World::new();
        let mut h = History::default();
        let ws = w.workspace();

        h.apply(&mut w, Command::create("Sprite", ws), false).unwrap();
        let sprite = sprite_child(&w, ws, "Sprite");
        h.apply(
            &mut w,
            Command::set_prop(
                sprite,
                "Position",
                Value::Vec2(Vec2::ZERO),
                Value::Vec2(Vec2::new(9.0, 9.0)),
            ),
            false,
        )
        .unwrap();
        h.apply(
            &mut w,
            Command::rename(sprite, "Sprite".into(), "Hero".into()),
            false,
        )
        .unwrap();

        h.undo(&mut w).unwrap();
        h.undo(&mut w).unwrap();
        h.undo(&mut w).unwrap();
        assert!(w.find_first_child(ws, "Sprite").is_none());
        assert!(w.find_first_child(ws, "Hero").is_none());

        h.redo(&mut w).unwrap();
        h.redo(&mut w).unwrap();
        h.redo(&mut w).unwrap();
        let hero = sprite_child(&w, ws, "Hero");
        assert_eq!(
            w.get_prop(hero, "Position"),
            Some(&Value::Vec2(Vec2::new(9.0, 9.0)))
        );
    }

    #[test]
    fn undo_delete_restores_and_remaps_earlier_commands() {
        let mut w = World::new();
        let mut h = History::default();
        let ws = w.workspace();

        h.apply(&mut w, Command::create("Folder", ws), false).unwrap();
        let folder = sprite_child(&w, ws, "Folder");
        h.apply(&mut w, Command::create("Sprite", folder), false).unwrap();
        let sprite = sprite_child(&w, folder, "Sprite");
        h.apply(
            &mut w,
            Command::set_prop(
                sprite,
                "Position",
                Value::Vec2(Vec2::ZERO),
                Value::Vec2(Vec2::new(3.0, 4.0)),
            ),
            false,
        )
        .unwrap();
        h.apply(&mut w, Command::delete(folder), false).unwrap();
        assert!(!w.contains(folder));

        h.undo(&mut w).unwrap();
        let folder2 = sprite_child(&w, ws, "Folder");
        let sprite2 = sprite_child(&w, folder2, "Sprite");
        assert_eq!(
            w.get_prop(sprite2, "Position"),
            Some(&Value::Vec2(Vec2::new(3.0, 4.0)))
        );

        h.undo(&mut w).unwrap();
        assert_eq!(
            w.get_prop(sprite2, "Position"),
            Some(&Value::Vec2(Vec2::ZERO))
        );

        h.undo(&mut w).unwrap();
        assert!(!w.contains(sprite2));
        h.undo(&mut w).unwrap();
        assert!(w.find_first_child(ws, "Folder").is_none());

        h.redo(&mut w).unwrap();
        h.redo(&mut w).unwrap();
        h.redo(&mut w).unwrap();
        h.redo(&mut w).unwrap();
        assert!(w.find_first_child(ws, "Folder").is_none());
    }

    #[test]
    fn duplicate_undo_redo() {
        let mut w = World::new();
        let mut h = History::default();
        let ws = w.workspace();

        h.apply(&mut w, Command::create("Sprite", ws), false).unwrap();
        let sprite = sprite_child(&w, ws, "Sprite");
        let dup = Command::duplicate(&w, sprite).unwrap();
        h.apply(&mut w, dup, false).unwrap();
        let count = |w: &World| {
            w.children(ws)
                .iter()
                .filter(|&&c| w.class_name(c) == Some("Sprite"))
                .count()
        };
        assert_eq!(count(&w), 2);

        h.undo(&mut w).unwrap();
        assert_eq!(count(&w), 1);
        h.redo(&mut w).unwrap();
        assert_eq!(count(&w), 2);
        h.undo(&mut w).unwrap();
        h.undo(&mut w).unwrap();
        assert_eq!(count(&w), 0);
    }

    #[test]
    fn merged_setprops_is_one_undo_step() {
        // A drag pushes many merged SetProps (Position + Size); the whole drag
        // must collapse to a single undo step that restores the original values.
        let mut w = World::new();
        let mut h = History::default();
        let ws = w.workspace();
        h.apply(&mut w, Command::create("Sprite", ws), false).unwrap();
        let sprite = sprite_child(&w, ws, "Sprite");
        let pos0 = w.get_prop(sprite, "Position").unwrap().clone();
        let size0 = w.get_prop(sprite, "Size").unwrap().clone();

        for i in 1..=6 {
            h.apply(
                &mut w,
                Command::set_props(
                    sprite,
                    vec![
                        ("Position", pos0.clone(), Value::Vec2(Vec2::new(i as f32, 0.0))),
                        ("Size", size0.clone(), Value::Vec2(Vec2::new(10.0 * i as f32, 20.0))),
                    ],
                ),
                i > 1,
            )
            .unwrap();
        }
        assert_eq!(
            w.get_prop(sprite, "Position"),
            Some(&Value::Vec2(Vec2::new(6.0, 0.0)))
        );

        // One undo reverts the entire drag (both props) to the originals...
        h.undo(&mut w).unwrap();
        assert_eq!(w.get_prop(sprite, "Position"), Some(&pos0));
        assert_eq!(w.get_prop(sprite, "Size"), Some(&size0));
        // ...and the only remaining undo step is the Create.
        h.undo(&mut w).unwrap();
        assert!(w.find_first_child(ws, "Sprite").is_none());

        // Redo restores the create then the whole transform in one step.
        h.redo(&mut w).unwrap();
        h.redo(&mut w).unwrap();
        let s2 = sprite_child(&w, ws, "Sprite");
        assert_eq!(
            w.get_prop(s2, "Position"),
            Some(&Value::Vec2(Vec2::new(6.0, 0.0)))
        );
    }

    #[test]
    fn merged_setprop_undoes_to_original() {
        let mut w = World::new();
        let mut h = History::default();
        let ws = w.workspace();
        h.apply(&mut w, Command::create("Sprite", ws), false).unwrap();
        let sprite = sprite_child(&w, ws, "Sprite");

        for i in 1..=5 {
            let prev = w.get_prop(sprite, "Position").unwrap().clone();
            h.apply(
                &mut w,
                Command::set_prop(
                    sprite,
                    "Position",
                    prev,
                    Value::Vec2(Vec2::new(i as f32, 0.0)),
                ),
                i > 1,
            )
            .unwrap();
        }
        assert_eq!(
            w.get_prop(sprite, "Position"),
            Some(&Value::Vec2(Vec2::new(5.0, 0.0)))
        );
        h.undo(&mut w).unwrap();
        assert_eq!(
            w.get_prop(sprite, "Position"),
            Some(&Value::Vec2(Vec2::ZERO))
        );
    }
}
