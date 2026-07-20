local map = workspace:FindFirstChild("Map")

-- A rich coal deposit under the miner's origin cell, then a mine -> smelt ->
-- store chain of adjacent buildings.
map:SetOre(1, 1, "coal", 1000)
map:PlaceBuilding("miner", 1, 1) -- 2x2, origin (1,1) sits on the coal
map:PlaceBuilding("smelter", 1, 3) -- 2x2, shares an edge with the miner
map:PlaceBuilding("storage", 1, 5) -- 1x1, shares an edge with the smelter
