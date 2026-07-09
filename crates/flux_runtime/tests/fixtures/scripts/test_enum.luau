local sprite = script.Parent

-- EnumItem fields mirror Roblox.
assert(Enum.KeyCode.A.Name == "A", "KeyCode.A.Name")
assert(Enum.KeyCode.A.Value == 97, "KeyCode.A.Value")
assert(Enum.KeyCode.A.EnumType == "KeyCode", "KeyCode.A.EnumType")
assert(tostring(Enum.KeyCode.Space) == "Enum.KeyCode.Space", "tostring")
assert(Enum.KeyCode.A == Enum.KeyCode.A, "equality")
assert(Enum.KeyCode.A ~= Enum.KeyCode.B, "inequality")
assert(#Enum.KeyCode:GetEnumItems() > 0, "GetEnumItems")

game.Heartbeat:Connect(function()
    -- Enum.KeyCode.Left must resolve to the engine's "ArrowLeft" token.
    if Input.IsKeyDown(Enum.KeyCode.Left) then
        sprite.Name = "MovedLeft"
    end
end)
