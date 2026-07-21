use std::collections::HashMap;

use indexmap::IndexMap;

use crate::class::{ClassId, registry};
use crate::error::CoreError;
use crate::value::Value;
use crate::world::{Instance, InstanceId, World};

#[derive(Clone)]
pub struct Subtree {
    nodes: Vec<SubtreeNode>,
}

#[derive(Clone)]
struct SubtreeNode {
    old_id: InstanceId,
    class: ClassId,
    name: String,
    props: IndexMap<&'static str, Value>,
    attributes: IndexMap<String, Value>,
    tags: Vec<String>,
    parent_slot: Option<usize>,
}

impl Subtree {
    pub fn root_id(&self) -> InstanceId {
        self.nodes[0].old_id
    }

    pub fn remap(&mut self, map: &HashMap<InstanceId, InstanceId>) {
        for node in &mut self.nodes {
            if let Some(new) = map.get(&node.old_id) {
                node.old_id = *new;
            }
            let prop_values = node.props.iter_mut().map(|(_, v)| v);
            let attr_values = node.attributes.iter_mut().map(|(_, v)| v);
            for value in prop_values.chain(attr_values) {
                if let Value::InstanceRef(Some(target)) = value {
                    if let Some(new) = map.get(target) {
                        *target = *new;
                    }
                }
            }
        }
    }
}

impl World {
    pub fn snapshot_subtree(&self, id: InstanceId) -> Option<Subtree> {
        if !self.contains(id) {
            return None;
        }
        let mut nodes = Vec::new();
        let mut slot_of: HashMap<InstanceId, usize> = HashMap::new();
        for cur in self.descendants(id) {
            let parent_slot = if cur == id {
                None
            } else {
                self.parent(cur).and_then(|p| slot_of.get(&p).copied())
            };
            slot_of.insert(cur, nodes.len());
            nodes.push(SubtreeNode {
                old_id: cur,
                class: self.class_of(cur).unwrap(),
                name: self.name(cur).unwrap().to_string(),
                props: self.props(cur).map(|(k, v)| (k, v.clone())).collect(),
                attributes: self.attributes(cur).map(|(k, v)| (k.to_string(), v.clone())).collect(),
                tags: self.tags(cur).map(str::to_string).collect(),
                parent_slot,
            });
        }
        Some(Subtree { nodes })
    }

    pub fn restore_subtree(
        &mut self,
        parent: InstanceId,
        index: usize,
        subtree: &Subtree,
    ) -> Result<HashMap<InstanceId, InstanceId>, CoreError> {
        let map = self.restore_nodes(Some(parent), subtree)?;
        let root_new = map[&subtree.root_id()];
        let children = &mut self.instances[parent].children;
        children.retain(|&c| c != root_new);
        let index = index.min(children.len());
        children.insert(index, root_new);
        Ok(map)
    }

    pub fn restore_subtree_detached(
        &mut self,
        subtree: &Subtree,
    ) -> Result<HashMap<InstanceId, InstanceId>, CoreError> {
        self.restore_nodes(None, subtree)
    }

    fn restore_nodes(
        &mut self,
        parent: Option<InstanceId>,
        subtree: &Subtree,
    ) -> Result<HashMap<InstanceId, InstanceId>, CoreError> {
        if let Some(p) = parent {
            if !self.contains(p) {
                return Err(CoreError::InstanceNotFound);
            }
        }
        let mut map = HashMap::new();
        let mut new_ids = Vec::with_capacity(subtree.nodes.len());
        for node in &subtree.nodes {
            let nid = match node.parent_slot {
                Some(slot) => self.spawn_raw(node.class, new_ids[slot]),
                None => match parent {
                    Some(p) => self.spawn_raw(node.class, p),
                    None => {
                        let info = registry().info(node.class);
                        let props = info
                            .props
                            .iter()
                            .map(|pd| (pd.name, pd.default.clone()))
                            .collect();
                        self.instances.insert(Instance {
                            class: node.class,
                            name: info.name.to_string(),
                            parent: None,
                            children: Vec::new(),
                            props,
                        })
                    }
                },
            };
            self.set_name_raw(nid, node.name.clone());
            for (k, v) in &node.props {
                self.set_prop(nid, k, v.clone())?;
            }
            for (k, v) in &node.attributes {
                self.set_attribute(nid, k, Some(v.clone()))?;
            }
            for tag in &node.tags {
                self.add_tag(nid, tag);
            }
            map.insert(node.old_id, nid);
            new_ids.push(nid);
        }
        for &nid in &new_ids {
            let fixes: Vec<(&'static str, InstanceId)> = self
                .props(nid)
                .filter_map(|(k, v)| match v {
                    Value::InstanceRef(Some(t)) => map.get(t).map(|nt| (k, *nt)),
                    _ => None,
                })
                .collect();
            for (k, nt) in fixes {
                self.set_prop(nid, k, Value::InstanceRef(Some(nt)))?;
            }
            // Object attributes pointing INSIDE the copied subtree follow the
            // copy; refs to outside instances are left pointing at the originals.
            let attr_fixes: Vec<(String, InstanceId)> = self
                .attributes(nid)
                .filter_map(|(k, v)| match v {
                    Value::InstanceRef(Some(t)) => map.get(t).map(|nt| (k.to_string(), *nt)),
                    _ => None,
                })
                .collect();
            for (k, nt) in attr_fixes {
                self.set_attribute(nid, &k, Some(Value::InstanceRef(Some(nt))))?;
            }
        }
        Ok(map)
    }
}
