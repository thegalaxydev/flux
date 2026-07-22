local map = workspace:FindFirstChild("Map")

local w, h = map:GetMapSize()
print("size " .. w .. "x" .. h)

map:SetTile(1, 1, "grass")
print("tile " .. tostring(map:GetTile(1, 1)))

map:SetOre(1, 1, "coal", 100)
local ore, amt = map:GetOre(1, 1)
print("ore " .. tostring(ore) .. " " .. tostring(amt))

print("mined " .. map:MineOre(1, 1, 30))
local _, left = map:GetOre(1, 1)
print("left " .. tostring(left))

-- Mine the rest; the deposit should clear (GetOre -> nil).
map:MineOre(1, 1, 1000)
print("after " .. tostring(map:GetOre(1, 1)))

-- Iso round-trip through the tilemap's own geometry.
local wp = map:TileToWorld(2, 3)
local c, r = map:WorldToTile(wp)
print("roundtrip " .. c .. "," .. r)
