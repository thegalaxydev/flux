# Sprite animation

Flux separates *drawing* from *animation data*, the way Godot/Unity/GameMaker do.

## Nodes

### `Sprite` — static render node
Draws a texture region and nothing else: `Texture`, `SourceRect` (pixels; zero
size = whole image), `Size`, `Pivot`, `Tint`, `FlipX`, `FlipY`, `Material`.
It has no animation state.

### `AnimatedSprite` — sprite-frame animation
A self-contained node that **owns playback and rendering**. It resolves the
current frame of the selected `Animation` from its `Frames` library and draws it
directly — it never creates or mutates another node.

| Property | Kind | Notes |
|----------|------|-------|
| `Frames` | authored | a `.spriteframes` library; the single source of truth for the texture |
| `Animation` | authored | which clip to play |
| `AutoPlay` | authored | play `Animation` when the session starts |
| `SpeedScale` | authored | playback multiplier |
| `Size`, `Pivot`, `Tint`, `FlipX/Y`, `Material` | authored | visual config |
| `Playing`, `CurrentFrame`, `TimePosition` | **transient** | runtime state; never serialized |

Transient properties are held on the instance so scripts and the inspector can
read them, but `to_json` skips them — scene files never contain a
`CurrentFrame = 5`.

> `AnimationPlayer` is intentionally **not** this node. That name is reserved for
> a future general-purpose property animator (keyframing arbitrary node
> properties like `Position`, `Rotation`, `Tint`). Sprite-frame animation does
> not depend on it.

## The `.spriteframes` asset

A JSON library of named clips (the `.frames.json` extension still loads):

```json
{
  "texture": "hero.png",
  "clips": {
    "Run": {
      "loop": true,
      "frames": [
        { "rect": [0, 0, 32, 32],  "duration": 0.10 },
        { "rect": [32, 0, 34, 32], "duration": 0.10 },
        { "rect": [66, 0, 30, 32], "duration": 0.16 }
      ]
    }
  }
}
```

Every frame is an explicit rectangle with its own duration, so **irregular
atlases** and **variable timing** are first-class. A clip (or a single frame) may
override the library `texture`. Libraries are parsed once and shared across all
`AnimatedSprite`s that reference the same file (`Rc<SpriteFrames>` via
`AnimationCache`).

## Luau API

```lua
local s = player.AnimatedSprite

s:Play("Run")        -- play from the start; no-op if "Run" is already playing
s:Play("Run", true)  -- force restart
s:Pause()
s:Resume()
s:Stop()             -- stop and reset to frame 0

s.Animation = "Idle" -- select without playing
s.SpeedScale = 2
s.FlipX = true

print(s.IsPlaying, s.Animation, s.CurrentFrame)
```

## Authoring workflow

1. Import `hero.png`.
2. Create a `.spriteframes` library and open it in the animation editor
   (double-click, or the "New animation library" button in Assets).
3. Slice the sheet (grid import) and build clips (`Idle`, `Run`, `Jump`).
4. Save.
5. Add an `AnimatedSprite`, set its `Frames` to the library, pick an
   `Animation`, and enable `AutoPlay` or call `:Play()` from a script.

No separate player node, no copied texture paths, no implicit parent hierarchy.

## Migration

Older scenes shaped as `Sprite → AnimationPlayer` load through a compatibility
path: `World::from_json` converts each such pair into a single `AnimatedSprite`,
transferring the frames library, autoplay/speed/animation, and the sprite's
transform and visual configuration. The obsolete player node is removed.

## Deferred (follow-up stages)

- Typed asset-reference property fields with drag-and-drop, picker, clear/open,
  and broken-reference state (currently `Frames`/`Texture`/`Material` are path
  fields).
- Assets-panel node creation on drag (texture → `Sprite`, `.spriteframes` →
  `AnimatedSprite`) and a `Create Sprite Frames` action on textures.
- Animation-editor extras: `Animation` inspector dropdown, reverse/ping-pong,
  multi-select + batch FPS, manual/auto atlas slicing.
