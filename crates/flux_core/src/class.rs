use std::collections::HashMap;
use std::sync::LazyLock;

use glam::Vec2;

use crate::value::{Color, UDim2, Value, ValueType};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClassId(usize);

#[derive(Clone, Debug)]
pub struct PropDef {
    pub name: &'static str,
    pub ty: ValueType,
    pub default: Value,
}

fn prop(name: &'static str, default: Value) -> PropDef {
    PropDef {
        name,
        ty: default.ty(),
        default,
    }
}

pub struct ClassInfo {
    pub name: &'static str,
    pub superclass: Option<ClassId>,
    pub creatable: bool,
    pub service: bool,
    pub props: Vec<PropDef>,
}

pub struct ClassRegistry {
    classes: Vec<ClassInfo>,
    by_name: HashMap<&'static str, ClassId>,
}

impl ClassRegistry {
    pub fn find(&self, name: &str) -> Option<ClassId> {
        self.by_name.get(name).copied()
    }

    pub fn info(&self, id: ClassId) -> &ClassInfo {
        &self.classes[id.0]
    }

    pub fn is_a(&self, id: ClassId, ancestor: ClassId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.classes[c.0].superclass;
        }
        false
    }

    pub fn creatable_classes(&self) -> impl Iterator<Item = &ClassInfo> {
        self.classes.iter().filter(|c| c.creatable)
    }

    fn add(
        &mut self,
        name: &'static str,
        superclass: Option<&str>,
        creatable: bool,
        service: bool,
        own_props: Vec<PropDef>,
    ) {
        let superclass = superclass.map(|s| self.by_name[s]);
        let mut props = superclass
            .map(|s| self.classes[s.0].props.clone())
            .unwrap_or_default();
        props.extend(own_props);
        let id = ClassId(self.classes.len());
        self.classes.push(ClassInfo {
            name,
            superclass,
            creatable,
            service,
            props,
        });
        self.by_name.insert(name, id);
    }

    fn build() -> Self {
        let mut reg = Self {
            classes: Vec::new(),
            by_name: HashMap::new(),
        };
        reg.add("Instance", None, false, false, vec![]);
        reg.add("Game", Some("Instance"), false, true, vec![]);
        reg.add(
            "Workspace",
            Some("Instance"),
            false,
            true,
            vec![prop("CurrentCamera", Value::InstanceRef(None))],
        );
        reg.add("Storage", Some("Instance"), false, true, vec![]);
        // Top-level home for Scripts that aren't attached to a specific object.
        reg.add("Scripts", Some("Instance"), false, true, vec![]);
        reg.add("Folder", Some("Instance"), true, false, vec![]);
        reg.add(
            "Node2D",
            Some("Instance"),
            true,
            false,
            vec![
                prop("Position", Value::Vec2(Vec2::ZERO)),
                prop("Rotation", Value::Number(0.0)),
                prop("Scale", Value::Vec2(Vec2::ONE)),
                prop("ZIndex", Value::Number(0.0)),
                prop("Visible", Value::Bool(true)),
                prop("Locked", Value::Bool(false)),
            ],
        );
        reg.add(
            "Sprite",
            Some("Node2D"),
            true,
            false,
            vec![
                prop("Texture", Value::Asset(String::new())),
                prop("Size", Value::Vec2(Vec2::new(100.0, 100.0))),
                prop("Pivot", Value::Vec2(Vec2::new(0.5, 0.5))),
                prop("Tint", Value::Color(Color::WHITE)),
                prop("FlipX", Value::Bool(false)),
                prop("FlipY", Value::Bool(false)),
            ],
        );
        reg.add(
            "Camera2D",
            Some("Instance"),
            true,
            false,
            vec![
                prop("Position", Value::Vec2(Vec2::ZERO)),
                prop("Zoom", Value::Number(1.0)),
            ],
        );
        reg.add(
            "Script",
            Some("Instance"),
            true,
            false,
            vec![
                prop("SourcePath", Value::Asset(String::new())),
                prop("Enabled", Value::Bool(true)),
            ],
        );
        // A Module is loaded on demand via `require`, not run automatically.
        reg.add(
            "Module",
            Some("Instance"),
            true,
            false,
            vec![prop("SourcePath", Value::Asset(String::new()))],
        );
        reg.add("Gui", Some("Instance"), false, true, vec![]);
        reg.add(
            "GuiObject",
            Some("Instance"),
            false,
            false,
            vec![
                prop("Position", Value::UDim2(UDim2::new(0.0, 20.0, 0.0, 20.0))),
                prop("Size", Value::UDim2(UDim2::new(0.0, 160.0, 0.0, 32.0))),
                prop("AnchorPoint", Value::Vec2(Vec2::ZERO)),
                prop("BackgroundColor", Value::Color(Color::new(0.14, 0.15, 0.19, 1.0))),
                prop("BackgroundTransparency", Value::Number(0.0)),
                prop("Visible", Value::Bool(true)),
                prop("ClipsDescendants", Value::Bool(false)),
                prop("ZIndex", Value::Number(0.0)),
            ],
        );
        reg.add("Frame", Some("GuiObject"), true, false, vec![]);
        reg.add(
            "Label",
            Some("GuiObject"),
            true,
            false,
            vec![
                prop("Text", Value::String("Label".to_string())),
                prop("TextColor", Value::Color(Color::WHITE)),
                prop("TextSize", Value::Number(16.0)),
            ],
        );
        reg.add(
            "Button",
            Some("GuiObject"),
            true,
            false,
            vec![
                prop("Text", Value::String("Button".to_string())),
                prop("TextColor", Value::Color(Color::WHITE)),
                prop("TextSize", Value::Number(16.0)),
            ],
        );
        reg
    }
}

static REGISTRY: LazyLock<ClassRegistry> = LazyLock::new(ClassRegistry::build);

pub fn registry() -> &'static ClassRegistry {
    &REGISTRY
}
