local f = script.Parent

-- Parent is the Gui screen root, so scale resolves against the viewport size.
f.Position = UDim2.fromScale(0.5, 0.5)
f.Size = UDim2.fromOffset(40, 20)

game.Heartbeat:Connect(function()
    local ap = f.AbsolutePosition
    local sz = f.AbsoluteSize
    if math.abs(ap.X - 100) < 0.5
        and math.abs(ap.Y - 50) < 0.5
        and math.abs(sz.X - 40) < 0.5
        and math.abs(sz.Y - 20) < 0.5 then
        f.Name = "AbsOk"
    end
end)
