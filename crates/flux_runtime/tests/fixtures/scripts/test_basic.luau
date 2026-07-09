local sprite = script.Parent

print("hello", sprite.Name)
sprite.Position = Vec2.new(1, 2)

game.Heartbeat:Connect(function(dt)
    sprite.Position = sprite.Position + Vec2.new(10 * dt, 0)
end)

task.spawn(function()
    task.wait(0.05)
    sprite.Name = "Waited"
end)

local clone = game.Storage.Template:Clone()
clone.Name = "ClonedThing"
clone.Parent = workspace
