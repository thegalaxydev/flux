local a = require(game.Scripts.Balance)
local b = require(game.Scripts.Balance)  -- second require returns the cache
print("speed " .. a.speed)
print("same " .. tostring(a == b))
