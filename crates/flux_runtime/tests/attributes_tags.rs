//! Roblox-style attributes + tags through the Lua API: set/get/enumerate,
//! nil-removal, type inference, tag queries via CollectionService, and the
//! attributes-are-data rule (instances rejected).

use std::path::Path;

use flux_runtime::{LogLevel, Session};

const SCRIPT: &str = r#"
local ws = workspace
local cs = game:GetService("CollectionService")

ws:SetAttribute("Money", 42)
ws:SetAttribute("Label", "hi")
ws:SetAttribute("Spot", Vec2.new(1, 2))
print("attr num " .. tostring(ws:GetAttribute("Money")))
print("attr str " .. tostring(ws:GetAttribute("Label")))
local s = ws:GetAttribute("Spot")
print("attr vec " .. tostring(s.X == 1 and s.Y == 2))

-- Removal is explicit; nil never deletes an attribute.
ws:RemoveAttribute("Money")
print("attr gone " .. tostring(ws:GetAttribute("Money") == nil))
local nilRemove = pcall(function()
	ws:SetAttribute("Label", nil)
end)
print("attr nil refused " .. tostring(not nilRemove))
local n = 0
for _, _ in pairs(ws:GetAttributes()) do
	n += 1
end
print("attr count " .. n)

ws:AddTag("zone")
print("tag has " .. tostring(ws:HasTag("zone")))
local tagged = cs:GetTagged("zone")
print("tag query " .. tostring(#tagged == 1 and tagged[1] == ws))
ws:RemoveTag("zone")
print("tag removed " .. tostring(not ws:HasTag("zone")))

-- Object attributes point at instances; a destroyed target reads as nil.
local cam = ws:FindFirstChild("Camera")
ws:SetAttribute("MainCamera", cam)
print("attr object " .. tostring(cam ~= nil and ws:GetAttribute("MainCamera") == cam))
local temp = cam:Clone()
temp.Parent = ws
ws:SetAttribute("Doomed", temp)
temp:Destroy()
print("attr object dangling " .. tostring(ws:GetAttribute("Doomed") == nil))

-- nil CLEARS an Object attribute (keeps it, pointing at nothing): clearing
-- again still succeeds, but after RemoveAttribute nil is refused.
ws:SetAttribute("MainCamera", nil)
local clearAgain = pcall(function()
	ws:SetAttribute("MainCamera", nil)
end)
print("attr object cleared " .. tostring(ws:GetAttribute("MainCamera") == nil and clearAgain))
ws:RemoveAttribute("MainCamera")
local clearRemoved = pcall(function()
	ws:SetAttribute("MainCamera", nil)
end)
print("attr object removed " .. tostring(not clearRemoved))
"#;

fn scene(script_rel: &str) -> String {
    format!(
        r#"{{
  "version": 1,
  "root": {{
    "class": "Game", "name": "Game",
    "children": [
      {{ "class": "Workspace", "name": "Workspace", "children": [
        {{ "class": "Camera2D", "name": "Camera" }}
      ] }},
      {{ "class": "Storage", "name": "Storage" }},
      {{ "class": "Gui", "name": "Gui" }},
      {{ "class": "Scripts", "name": "Scripts", "children": [
        {{ "class": "Script", "name": "Test",
           "props": {{ "SourcePath": {{ "t": "Asset", "v": "{script_rel}" }} }} }}
      ] }}
    ]
  }}
}}"#
    )
}

#[test]
fn lua_attributes_and_tags_work() {
    let dir = std::env::temp_dir().join("flux_attr_tag_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("test.luau"), SCRIPT).unwrap();

    let session = Session::from_scene_json(&scene("test.luau"), Path::new(&dir)).unwrap();
    let logs: Vec<String> = session
        .drain_logs()
        .into_iter()
        .map(|l| match l.level {
            LogLevel::Info => l.message,
            other => format!("[{other:?}] {}", l.message),
        })
        .collect();
    let has = |s: &str| logs.iter().any(|m| m == s);
    assert!(has("attr num 42"), "{logs:?}");
    assert!(has("attr str hi"), "{logs:?}");
    assert!(has("attr vec true"), "{logs:?}");
    assert!(has("attr gone true"), "{logs:?}");
    assert!(has("attr nil refused true"), "{logs:?}");
    assert!(has("attr count 2"), "{logs:?}");
    assert!(has("tag has true"), "{logs:?}");
    assert!(has("tag query true"), "{logs:?}");
    assert!(has("tag removed true"), "{logs:?}");
    assert!(has("attr object true"), "{logs:?}");
    assert!(has("attr object dangling true"), "{logs:?}");
    assert!(has("attr object cleared true"), "{logs:?}");
    assert!(has("attr object removed true"), "{logs:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
