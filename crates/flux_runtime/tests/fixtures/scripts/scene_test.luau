--!strict
-- Reports the current scene name and requests a switch at load time.
print("scene is " .. Scene.Name)
Scene:Load("hub.scene.json")
