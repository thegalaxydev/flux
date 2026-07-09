local sprite = script.Parent

game.Heartbeat:Connect(function()
    if Input.IsKeyDown(Enum.KeyCode.A) then
        sprite.Name = "KeyDown"
    end
end)
