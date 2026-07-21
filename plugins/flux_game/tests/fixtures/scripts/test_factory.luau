local map = workspace:FindFirstChild("Map")

-- A rich coal deposit under the miner's origin cell, then a full conveyor
-- chain: mine -> belt -> smelt -> belt -> store. Logistics REQUIRE belts
-- (producers only hand items to adjacent conveyors), and belts are
-- directional (1 = +y, pointing down the chain).
map:SetOre(1, 1, "coal", 1000)
map:PlaceBuilding("miner", 1, 1) -- 2x2, origin (1,1) sits on the coal
map:PlaceBuilding("belt", 1, 3, 1) -- points +y into the smelter
map:PlaceBuilding("smelter", 1, 4) -- 2x2
map:PlaceBuilding("belt", 1, 6, 1) -- points +y into the storage
map:PlaceBuilding("storage", 1, 7) -- 1x1
