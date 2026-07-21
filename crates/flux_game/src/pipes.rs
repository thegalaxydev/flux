//! Fluid pipes: auto-connecting visuals now, network simulation in
//! [`crate::fluids`].
//!
//! Pipes are undirected 1x1 buildings (`pipe: true` in the catalog). Each
//! frame the system computes every pipe's connectivity mask — N=1, E=2, S=4,
//! W=8 in tile space — from its neighbours (other pipes, or machine
//! liquid/gas ports facing the pipe) and switches the sprite to the matching
//! `m<mask>` clip, so pipe runs, corners, junctions and crossings read
//! correctly without any authored direction.

use std::collections::HashMap;
use std::path::Path;

use flux_core::{InstanceId, Value, World};

use crate::building::BuildingCatalogCache;

/// The runtime system driving pipe visuals (fluid flow lives in fluids.rs).
#[derive(Default)]
pub struct PipeSystem {
    buildings: BuildingCatalogCache,
}

impl flux_runtime::System for PipeSystem {
    fn step(&mut self, world: &mut World, root: &Path, _dt: f32) {
        sync_visuals(world, &mut self.buildings, root);
    }
}

fn text(world: &World, id: InstanceId, name: &str) -> String {
    match world.get_prop(id, name) {
        Some(Value::String(s)) | Some(Value::Asset(s)) => s.clone(),
        _ => String::new(),
    }
}

fn cell_of(world: &World, id: InstanceId) -> (i32, i32) {
    match world.get_prop(id, "Cell") {
        Some(Value::Vec2(v)) => (v.x as i32, v.y as i32),
        _ => (0, 0),
    }
}

/// Neighbour offsets in mask-bit order: N(-y), E(+x), S(+y), W(-x).
const NEIGHBOURS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// Compute connectivity masks and switch each pipe's sprite clip on change.
pub fn sync_visuals(world: &mut World, buildings: &mut BuildingCatalogCache, root: &Path) {
    let maps: Vec<InstanceId> = world
        .descendants(world.workspace())
        .into_iter()
        .filter(|&id| world.class_name(id) == Some("Tilemap"))
        .collect();

    for tm in maps {
        let bc_path = crate::attr_text(world, tm, "Buildings");
        let Some(cat) = buildings.get(&bc_path, root) else {
            continue;
        };

        let ids: Vec<InstanceId> = world
            .children(tm)
            .iter()
            .copied()
            .filter(|&c| world.class_name(c) == Some("Building"))
            .collect();

        // Which cells hold pipes, and which cells host a fluid port facing where.
        let mut pipe_cells: HashMap<(i32, i32), InstanceId> = HashMap::new();
        let mut pipes: Vec<(InstanceId, (i32, i32))> = Vec::new();
        for &b in &ids {
            if cat.get(&text(world, b, "Type")).is_some_and(|d| d.pipe) {
                let c = cell_of(world, b);
                pipe_cells.insert(c, b);
                pipes.push((b, c));
            }
        }
        if pipes.is_empty() {
            continue;
        }
        // (port cell, facing cell) of every fluid port on the map.
        let mut fluid_ports: Vec<((i32, i32), (i32, i32))> = Vec::new();
        for &b in &ids {
            if let Some(baked) = crate::ports::of(world, b) {
                for rp in &baked.0 {
                    if rp.port.kind.is_fluid() {
                        fluid_ports.push((rp.cell, rp.facing));
                    }
                }
            }
        }

        for (b, (c, r)) in pipes {
            let mut mask = 0u8;
            for (bit, (dc, dr)) in NEIGHBOURS.iter().enumerate() {
                let n = (c + dc, r + dr);
                let connects = pipe_cells.contains_key(&n)
                    || fluid_ports.iter().any(|&(pc, pf)| pc == n && pf == (c, r));
                if connects {
                    mask |= 1 << bit;
                }
            }
            // Cache the mask in a transient prop; only replay the clip on change.
            let prev = match world.get_prop(b, "_Mask") {
                Some(Value::Number(n)) => *n as i32,
                _ => -1,
            };
            if prev != mask as i32 {
                let _ = world.set_prop(b, "_Mask", Value::Number(mask as f64));
                if let Some(sprite) = crate::building::sprite_of(world, b) {
                    flux_core::animation::play(world, sprite, &format!("m{mask}"), false);
                }
            }
        }
    }
}
