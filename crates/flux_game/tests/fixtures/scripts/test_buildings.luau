local map = workspace:FindFirstChild("Map")

print("canplace " .. tostring(map:CanPlace("smelter", 2, 2)))

local b = map:PlaceBuilding("smelter", 2, 2)
print("placed " .. tostring(b ~= nil))
print("class " .. tostring(b.ClassName))
print("type " .. tostring(b.Type))
print("at33 " .. tostring(map:GetBuildingAt(3, 3) == b))

-- Overlapping placement is refused.
print("blocked " .. tostring(map:PlaceBuilding("smelter", 3, 3) == nil))

print("removed " .. tostring(map:RemoveBuilding(3, 3)))
print("gone " .. tostring(map:GetBuildingAt(2, 2) == nil))

-- Camera screen<->world round-trip.
local cam = workspace.CurrentCamera
local wp = cam:ScreenToWorld(Vec2.new(400, 300))
local sp = cam:WorldToScreen(wp)
print("cam " .. tostring(math.abs(sp.X - 400) < 0.01 and math.abs(sp.Y - 300) < 0.01))

-- Directional placement: the 4th arg lands in the Direction prop.
local belt = map:PlaceBuilding("belt", 6, 6, 2)
print("beltdir " .. tostring(belt.Direction == 2))

-- Placement ghost: appears, follows, clears; never registers as a building.
map:SetGhost("smelter", 8, 8)
print("ghost " .. tostring(map:FindFirstChild("_Ghost") ~= nil))
print("ghostfree " .. tostring(map:GetBuildingAt(8, 8) == nil))
map:SetGhost(nil)
print("ghostgone " .. tostring(map:FindFirstChild("_Ghost") == nil))

-- Selection prop round-trips instances and nil.
map._Selected = belt
print("sel " .. tostring(map._Selected == belt))
map._Selected = nil
print("selnil " .. tostring(map._Selected == nil))
