local map = workspace:FindFirstChild("Map")
map:PlaceBuilding("smelter", 2, 2)
map:SetTile(0, 0, "stone")

local save = game:GetService("SaveService")
save:Save("slot1")
print("exists " .. tostring(save:Exists("slot1")))
print("listed " .. tostring(table.find(save:List(), "slot1") ~= nil))
save:Load("slot1")
