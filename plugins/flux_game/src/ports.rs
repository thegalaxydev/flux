//! Typed machine connection ports.
//!
//! A building type declares WHERE it exchanges resources and WHAT kind: a
//! reactor takes uranium (item) on its west edge, water (liquid) on the
//! north, vents steam (gas) east and waste (item) south. Ports are authored
//! in the building catalog relative to the un-rotated footprint and rotate
//! with the building's `Direction`, using the same 0=+x, 1=+y, 2=-x, 3=-y
//! convention as conveyors.
//!
//! Transport systems consult ports instead of assuming every edge is open:
//! conveyors may only feed item inputs, pipes only liquid/gas ports.
//! `Power`/`Heat` kinds parse and display but are not yet simulated (power
//! remains the per-map balance; heat lives in the reactor dynamics).

use serde::{Deserialize, Serialize};

use flux_core::{InstanceId, Value, World};

use crate::building::BuildingDef;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortKind {
    Item,
    Liquid,
    Gas,
    Power,
    Heat,
}

impl PortKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "item" => PortKind::Item,
            "liquid" => PortKind::Liquid,
            "gas" => PortKind::Gas,
            "power" => PortKind::Power,
            "heat" => PortKind::Heat,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            PortKind::Item => "item",
            PortKind::Liquid => "liquid",
            PortKind::Gas => "gas",
            PortKind::Power => "power",
            PortKind::Heat => "heat",
        }
    }

    /// Fluids (liquid/gas) share pipe plumbing.
    pub fn is_fluid(self) -> bool {
        matches!(self, PortKind::Liquid | PortKind::Gas)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortFlow {
    In,
    Out,
    InOut,
}

impl PortFlow {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "in" => PortFlow::In,
            "out" => PortFlow::Out,
            "inout" => PortFlow::InOut,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            PortFlow::In => "in",
            PortFlow::Out => "out",
            PortFlow::InOut => "inout",
        }
    }

    pub fn takes_input(self) -> bool {
        matches!(self, PortFlow::In | PortFlow::InOut)
    }

    pub fn gives_output(self) -> bool {
        matches!(self, PortFlow::Out | PortFlow::InOut)
    }
}

/// Authoring schema (inside a building's `ports` array in `*.buildings.json`).
#[derive(Serialize, Deserialize, Clone)]
pub struct PortDoc {
    pub id: String,
    /// item | liquid | gas | power | heat
    #[serde(default = "default_kind")]
    pub kind: String,
    /// in | out | inout
    #[serde(default = "default_flow")]
    pub flow: String,
    /// Local (pre-rotation) edge: n | e | s | w.
    pub side: String,
    /// Position along that edge, 0-based from the edge's low corner.
    #[serde(default)]
    pub offset: u32,
    /// Resource ids this port accepts/emits; empty = any.
    #[serde(default)]
    pub accepts: Vec<String>,
    /// Max simultaneous connections (conveyors) — 0 = unlimited.
    #[serde(default)]
    pub limit: u32,
    /// Tank slot this port serves (fluid ports).
    #[serde(default)]
    pub tank: String,
    /// Fluid units per second through this port.
    #[serde(default = "default_throughput")]
    pub throughput: f32,
}

fn default_kind() -> String {
    "item".into()
}
fn default_flow() -> String {
    "in".into()
}
fn default_throughput() -> f32 {
    20.0
}

/// Parsed runtime port (still local — resolve against a placed building).
#[derive(Clone, Debug)]
pub struct Port {
    pub id: String,
    pub kind: PortKind,
    pub flow: PortFlow,
    /// Local edge as a direction index (0=+x/e, 1=+y/s, 2=-x/w, 3=-y/n).
    pub side: u8,
    pub offset: u32,
    pub accepts: Vec<String>,
    pub limit: u32,
    pub tank: String,
    pub throughput: f32,
}

impl Port {
    pub fn from_doc(doc: &PortDoc) -> Option<Port> {
        let side = match doc.side.as_str() {
            "e" => 0,
            "s" => 1,
            "w" => 2,
            "n" => 3,
            _ => return None,
        };
        Some(Port {
            id: doc.id.clone(),
            kind: PortKind::parse(&doc.kind)?,
            flow: PortFlow::parse(&doc.flow)?,
            side,
            offset: doc.offset,
            accepts: doc.accepts.clone(),
            limit: doc.limit,
            tank: doc.tank.clone(),
            throughput: doc.throughput.max(0.0),
        })
    }

    pub fn accepts_resource(&self, resource: &str) -> bool {
        self.accepts.is_empty() || self.accepts.iter().any(|a| a == resource)
    }
}

/// A port resolved against a placed building: the boundary cell it occupies
/// (inside the footprint) and the outward cell it faces.
#[derive(Clone, Debug)]
pub struct ResolvedPort {
    pub port: Port,
    pub cell: (i32, i32),
    pub facing: (i32, i32),
}

/// Resolve a def's ports for a building at `(col, row)` rotated by `dir`.
/// The authored side rotates with the building (side 'e' on a dir-1 building
/// faces +y), matching the conveyor `Direction` convention.
pub fn resolve(def: &BuildingDef, col: i32, row: i32, dir: u8) -> Vec<ResolvedPort> {
    let (w, h) = (def.width as i32, def.height as i32);
    def.ports
        .iter()
        .map(|p| {
            let side = (p.side + dir) % 4;
            let len = match side {
                0 | 2 => h,
                _ => w,
            };
            let i = (p.offset as i32).min(len - 1);
            let (cell, facing) = match side {
                0 => ((col + w - 1, row + i), (col + w, row + i)), // +x / east
                1 => ((col + i, row + h - 1), (col + i, row + h)), // +y / south
                2 => ((col, row + i), (col - 1, row + i)),         // -x / west
                _ => ((col + i, row), (col + i, row - 1)),         // -y / north
            };
            ResolvedPort { port: p.clone(), cell, facing }
        })
        .collect()
}

/// Baked ports component: resolved ports stored per placed building so
/// transport systems and the overlay can read them without a catalog. Derived
/// data — never registered with the save hook; (re)baked at placement and
/// lazily for loaded worlds by `FactorySystem`.
pub struct BakedPorts(pub Vec<ResolvedPort>);

/// Read a building's baked ports, if any.
pub fn of(world: &World, b: InstanceId) -> Option<&BakedPorts> {
    world.component::<BakedPorts>(b)
}

/// Bake (or rebake) a building's ports from its def + placement props.
pub fn bake(world: &mut World, b: InstanceId, def: &BuildingDef) {
    let (col, row) = match world.get_prop(b, "Cell") {
        Some(Value::Vec2(v)) => (v.x as i32, v.y as i32),
        _ => (0, 0),
    };
    let dir = match world.get_prop(b, "Direction") {
        Some(Value::Number(n)) => *n as u8,
        _ => 0,
    };
    world.set_component::<BakedPorts>(b, BakedPorts(resolve(def, col, row, dir)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building::BuildingCatalog;

    const CATALOG: &str = r#"{ "buildings": [
        { "id": "machine", "size": [3, 2], "ports": [
            { "id": "fuel_in",  "kind": "item",   "flow": "in",  "side": "w", "offset": 0, "accepts": ["uranium"], "limit": 1 },
            { "id": "water_in", "kind": "liquid", "flow": "in",  "side": "n", "offset": 1, "tank": "water" },
            { "id": "steam_out","kind": "gas",    "flow": "out", "side": "e", "offset": 1, "tank": "steam" },
            { "id": "waste_out","kind": "item",   "flow": "out", "side": "s", "offset": 2 }
        ]}
    ]}"#;

    #[test]
    fn ports_parse_and_resolve_unrotated() {
        let cat = BuildingCatalog::parse(CATALOG).unwrap();
        let def = cat.get("machine").unwrap();
        assert_eq!(def.ports.len(), 4);

        // 3x2 footprint at (10, 20), dir 0.
        let r = resolve(def, 10, 20, 0);
        let by_id = |id: &str| r.iter().find(|p| p.port.id == id).unwrap();

        let fuel = by_id("fuel_in"); // west edge, offset 0
        assert_eq!((fuel.cell, fuel.facing), ((10, 20), (9, 20)));
        assert_eq!(fuel.port.kind, PortKind::Item);
        assert!(fuel.port.accepts_resource("uranium"));
        assert!(!fuel.port.accepts_resource("coal"));

        let water = by_id("water_in"); // north edge, offset 1
        assert_eq!((water.cell, water.facing), ((11, 20), (11, 19)));

        let steam = by_id("steam_out"); // east edge, offset 1 (h=2 -> clamped 1)
        assert_eq!((steam.cell, steam.facing), ((12, 21), (13, 21)));

        let waste = by_id("waste_out"); // south edge, offset 2
        assert_eq!((waste.cell, waste.facing), ((12, 21), (12, 22)));
    }

    #[test]
    fn ports_rotate_with_direction() {
        let cat = BuildingCatalog::parse(CATALOG).unwrap();
        let def = cat.get("machine").unwrap();

        // dir 1 rotates every side one step clockwise: w -> n, n -> e, e -> s.
        let r = resolve(def, 0, 0, 1);
        let by_id = |id: &str| r.iter().find(|p| p.port.id == id).unwrap();

        let fuel = by_id("fuel_in"); // w + 1 = n edge (side 3), offset 0
        assert_eq!((fuel.cell, fuel.facing), ((0, 0), (0, -1)));
        let water = by_id("water_in"); // n + 1 = e edge (side 0), offset 1
        assert_eq!((water.cell, water.facing), ((2, 1), (3, 1)));
        let steam = by_id("steam_out"); // e + 1 = s edge (side 1), offset 1
        assert_eq!((steam.cell, steam.facing), ((1, 1), (1, 2)));

        // Full circle: dir 0 and dir 4 resolve identically.
        let a = resolve(def, 5, 5, 0);
        let b = resolve(def, 5, 5, 4);
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.cell, y.cell);
            assert_eq!(x.facing, y.facing);
        }
    }
}
