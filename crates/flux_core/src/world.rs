use indexmap::IndexMap;
use slotmap::{SecondaryMap, SlotMap};

use crate::class::{ClassId, registry};
use crate::error::CoreError;
use crate::tilemap::TileGrid;
use crate::value::Value;

slotmap::new_key_type! {
    pub struct InstanceId;
}

pub(crate) struct Instance {
    pub(crate) class: ClassId,
    pub(crate) name: String,
    pub(crate) parent: Option<InstanceId>,
    pub(crate) children: Vec<InstanceId>,
    pub(crate) props: IndexMap<&'static str, Value>,
}

pub struct World {
    pub(crate) instances: SlotMap<InstanceId, Instance>,
    pub(crate) root: InstanceId,
    /// Derived per-`Tilemap` grid data, keyed by instance. Transient — never
    /// serialized; regenerated from each tilemap's config by
    /// [`crate::tilemap::sync`]. See the tilemap module for why the grid lives
    /// on the world rather than in the scene tree.
    tilemaps: SecondaryMap<InstanceId, TileGrid>,
}

impl World {
    pub(crate) fn empty_game() -> Self {
        let mut instances = SlotMap::with_key();
        let class = registry().find("Game").unwrap();
        let info = registry().info(class);
        let props = info
            .props
            .iter()
            .map(|p| (p.name, p.default.clone()))
            .collect();
        let root = instances.insert(Instance {
            class,
            name: "Game".to_string(),
            parent: None,
            children: Vec::new(),
            props,
        });
        Self {
            instances,
            root,
            tilemaps: SecondaryMap::new(),
        }
    }

    pub fn new() -> Self {
        let mut world = Self::empty_game();
        let root = world.root;
        let workspace = world.spawn("Workspace", root);
        let camera = world.spawn("Camera2D", workspace);
        world
            .set_prop(workspace, "CurrentCamera", Value::InstanceRef(Some(camera)))
            .unwrap();
        world.spawn("Storage", root);
        world.spawn("Scripts", root);
        world.spawn("Gui", root);
        world
    }

    /// Create any standard singleton service container missing from the tree.
    /// Called after loading so older scenes gain newer services (e.g. `Scripts`)
    /// automatically, the way engine services are implicit.
    pub fn ensure_services(&mut self) {
        let root = self.root;
        for name in ["Storage", "Scripts", "Gui"] {
            if self.service(name).is_none() {
                self.spawn(name, root);
            }
        }
    }

    pub(crate) fn spawn(&mut self, class_name: &str, parent: InstanceId) -> InstanceId {
        let class = registry().find(class_name).unwrap();
        self.spawn_raw(class, parent)
    }

    pub(crate) fn spawn_raw(&mut self, class: ClassId, parent: InstanceId) -> InstanceId {
        let info = registry().info(class);
        let props = info
            .props
            .iter()
            .map(|p| (p.name, p.default.clone()))
            .collect();
        let id = self.instances.insert(Instance {
            class,
            name: info.name.to_string(),
            parent: Some(parent),
            children: Vec::new(),
            props,
        });
        self.instances[parent].children.push(id);
        id
    }

    pub fn create(&mut self, class_name: &str, parent: InstanceId) -> Result<InstanceId, CoreError> {
        let class = registry()
            .find(class_name)
            .ok_or_else(|| CoreError::UnknownClass(class_name.to_string()))?;
        let info = registry().info(class);
        if !info.creatable {
            return Err(CoreError::NotCreatable(class_name.to_string()));
        }
        if !self.instances.contains_key(parent) {
            return Err(CoreError::InstanceNotFound);
        }
        Ok(self.spawn_raw(class, parent))
    }

    pub fn destroy(&mut self, id: InstanceId) -> Result<(), CoreError> {
        let inst = self.instances.get(id).ok_or(CoreError::InstanceNotFound)?;
        if id == self.root || registry().info(inst.class).service {
            return Err(CoreError::CannotModifyService);
        }
        if let Some(parent) = inst.parent {
            self.instances[parent].children.retain(|&c| c != id);
        }
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            self.tilemaps.remove(cur);
            if let Some(inst) = self.instances.remove(cur) {
                stack.extend(inst.children);
            }
        }
        Ok(())
    }

    pub fn reparent(&mut self, id: InstanceId, new_parent: InstanceId) -> Result<(), CoreError> {
        let index = self.children(new_parent).len();
        self.reparent_at(id, new_parent, index)
    }

    pub fn reparent_at(
        &mut self,
        id: InstanceId,
        new_parent: InstanceId,
        index: usize,
    ) -> Result<(), CoreError> {
        if !self.instances.contains_key(id) || !self.instances.contains_key(new_parent) {
            return Err(CoreError::InstanceNotFound);
        }
        if id == self.root || registry().info(self.instances[id].class).service {
            return Err(CoreError::CannotModifyService);
        }
        let mut cur = Some(new_parent);
        while let Some(c) = cur {
            if c == id {
                return Err(CoreError::WouldCreateCycle);
            }
            cur = self.instances[c].parent;
        }
        if let Some(old_parent) = self.instances[id].parent {
            self.instances[old_parent].children.retain(|&c| c != id);
        }
        let children = &mut self.instances[new_parent].children;
        let index = index.min(children.len());
        children.insert(index, id);
        self.instances[id].parent = Some(new_parent);
        Ok(())
    }

    pub fn child_index(&self, id: InstanceId) -> Option<usize> {
        let parent = self.parent(id)?;
        self.children(parent).iter().position(|&c| c == id)
    }

    pub fn get_prop(&self, id: InstanceId, prop: &str) -> Option<&Value> {
        self.instances.get(id)?.props.get(prop)
    }

    pub fn set_prop(&mut self, id: InstanceId, prop: &str, value: Value) -> Result<(), CoreError> {
        let inst = self.instances.get_mut(id).ok_or(CoreError::InstanceNotFound)?;
        let Some(slot) = inst.props.get_mut(prop) else {
            return Err(CoreError::UnknownProperty(prop.to_string()));
        };
        if slot.ty() != value.ty() {
            return Err(CoreError::TypeMismatch {
                prop: prop.to_string(),
                expected: slot.ty(),
                got: value.ty(),
            });
        }
        *slot = value;
        Ok(())
    }

    /// The derived tile grid for a `Tilemap` instance, if [`crate::tilemap::sync`]
    /// has generated one.
    pub fn tile_grid(&self, id: InstanceId) -> Option<&TileGrid> {
        self.tilemaps.get(id)
    }

    /// Mutable access to a `Tilemap`'s derived grid, for runtime edits (mining,
    /// building). Mutations persist for the session; [`crate::tilemap::sync`]
    /// only regenerates on a config/seed change, not over these edits.
    pub fn tile_grid_mut(&mut self, id: InstanceId) -> Option<&mut TileGrid> {
        self.tilemaps.get_mut(id)
    }

    /// Store (or replace) a tilemap's derived grid. Only meaningful for live
    /// `Tilemap` instances; used by [`crate::tilemap::sync`].
    pub(crate) fn set_tile_grid(&mut self, id: InstanceId, grid: TileGrid) {
        if self.instances.contains_key(id) {
            self.tilemaps.insert(id, grid);
        }
    }

    pub fn root(&self) -> InstanceId {
        self.root
    }

    pub fn contains(&self, id: InstanceId) -> bool {
        self.instances.contains_key(id)
    }

    pub fn class_of(&self, id: InstanceId) -> Option<ClassId> {
        self.instances.get(id).map(|i| i.class)
    }

    pub fn class_name(&self, id: InstanceId) -> Option<&'static str> {
        self.instances.get(id).map(|i| registry().info(i.class).name)
    }

    pub fn name(&self, id: InstanceId) -> Option<&str> {
        self.instances.get(id).map(|i| i.name.as_str())
    }

    pub fn set_name(&mut self, id: InstanceId, name: impl Into<String>) -> Result<(), CoreError> {
        let inst = self.instances.get_mut(id).ok_or(CoreError::InstanceNotFound)?;
        if registry().info(inst.class).service {
            return Err(CoreError::CannotModifyService);
        }
        inst.name = name.into();
        Ok(())
    }

    pub(crate) fn set_name_raw(&mut self, id: InstanceId, name: String) {
        self.instances[id].name = name;
    }

    pub fn detach(&mut self, id: InstanceId) -> Result<(), CoreError> {
        if !self.instances.contains_key(id) {
            return Err(CoreError::InstanceNotFound);
        }
        if id == self.root || registry().info(self.instances[id].class).service {
            return Err(CoreError::CannotModifyService);
        }
        if let Some(parent) = self.instances[id].parent {
            self.instances[parent].children.retain(|&c| c != id);
        }
        self.instances[id].parent = None;
        Ok(())
    }

    pub fn parent(&self, id: InstanceId) -> Option<InstanceId> {
        self.instances.get(id).and_then(|i| i.parent)
    }

    pub fn children(&self, id: InstanceId) -> &[InstanceId] {
        self.instances
            .get(id)
            .map(|i| i.children.as_slice())
            .unwrap_or(&[])
    }

    pub fn props(&self, id: InstanceId) -> impl Iterator<Item = (&'static str, &Value)> {
        self.instances
            .get(id)
            .into_iter()
            .flat_map(|i| i.props.iter().map(|(k, v)| (*k, v)))
    }

    pub fn find_first_child(&self, id: InstanceId, name: &str) -> Option<InstanceId> {
        self.children(id)
            .iter()
            .copied()
            .find(|&c| self.instances[c].name == name)
    }

    pub fn descendants(&self, id: InstanceId) -> Vec<InstanceId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if !self.contains(cur) {
                continue;
            }
            out.push(cur);
            for &c in self.children(cur).iter().rev() {
                stack.push(c);
            }
        }
        out
    }

    pub fn service(&self, class_name: &str) -> Option<InstanceId> {
        let class = registry().find(class_name)?;
        self.children(self.root)
            .iter()
            .copied()
            .find(|&c| self.instances[c].class == class)
    }

    pub fn workspace(&self) -> InstanceId {
        self.service("Workspace").unwrap()
    }

    pub fn gui(&self) -> Option<InstanceId> {
        self.service("Gui")
    }

    pub fn scripts(&self) -> Option<InstanceId> {
        self.service("Scripts")
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}
