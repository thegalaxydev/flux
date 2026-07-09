# Flux

Flux is a small 2D game engine with a built-in editor. If you've used Roblox
Studio, the shape of it will feel familiar: your game is a tree of objects,
behaviour lives in Luau scripts attached to that tree, and you hit Play to try
it out. The whole thing is written in Rust, and games script in
[Luau](https://luau.org/).

It's early and opinionated. There's no asset store, no 3D, no physics engine —
just sprites, a GUI layer, input, a scheduler, and a persistence service, plus
an editor that's genuinely usable. I've been building actual little games with
it (an endless runner, a click-the-targets game) to keep it honest.

## Running the editor

You'll need a recent stable [Rust toolchain](https://rustup.rs/). Luau is
compiled from source (via `mlua`), so you also need a C compiler — on Windows
that's the MSVC build tools that ship with the default Rust install; on Linux
you'll want `cc` plus GTK dev headers for the file dialogs (`libgtk-3-dev` on
Debian/Ubuntu).

```sh
cargo run -p flux_editor
```

The editor opens on a launcher listing your recent projects. Pick **New Scene**
to start from scratch or **Open Project…** to load an existing
`main.scene.json`. Recent projects are remembered between runs.

Prefer to just run a game with no editor? There's a standalone player:

```sh
cargo run -p flux_player -- path/to/main.scene.json
```

## The editor at a glance

- **Explorer** (left) — the object tree. The four services (`Workspace`,
  `Storage`, `Scripts`, `Gui`) are always there; everything else hangs off them.
- **Properties** (right) — edit the selected object. Vec2, UDim2, colours, and
  asset paths all have proper widgets.
- **Viewport** (centre) — the scene. Scroll to zoom, drag to pan.
- **Script editor** — opens as a tab when you double-click a `.luau` file or a
  Script. It has autocomplete, hover docs, signature help, and live diagnostics
  (red/yellow squiggles) for the Flux API — no external language server needed.
- **Output** — `print`/`warn` and runtime errors. Clicking an error jumps to the
  offending line.

Handy keys: `F5` play/stop, `Q`/`W`/`E`/`R` for select/move/scale/rotate,
`Ctrl+Z`/`Ctrl+Y` undo/redo, `Ctrl+D` duplicate, `Del` delete, `Ctrl+S` save,
`Ctrl+F` find in a script.

When you drag sprites you can snap to the grid; when you drag GUI elements they
snap to line up with each other and their container (hold `Shift` to drag
freely).

## How a game is put together

Everything under the root is an **instance** with a class and some properties.
The top-level containers are:

- **Workspace** — the visible scene. Sprites live here, along with the
  `Camera2D`. World coordinates are centred on the camera, and **+Y points
  down**.
- **Storage** — off-screen objects. Good for templates you `Clone()` at runtime
  (bullets, enemies, UI you spawn on demand).
- **Scripts** — a tidy home for scripts that aren't tied to a specific object.
- **Gui** — the on-screen UI layer. GUI objects are laid out in screen space.

Two coordinate systems, because they solve different problems:

- **Sprites / Node2D** use plain `Vec2` for `Position` and `Size` — world pixels.
- **GuiObjects** (`Frame`, `Label`, `Button`) use `UDim2`, the Roblox-style
  "scale + offset" type, so UI can be part-relative to the screen and
  part-fixed-pixels. `UDim2.new(xScale, xOffset, yScale, yOffset)`.

## Scripting

Attach a `Script` anywhere in the tree (or drop it in `Scripts`) and point its
`SourcePath` at a `.luau` file in your project. Scripts run when the game starts;
`script.Parent` is whatever the script is attached to.

Here's a sprite that moves with the arrow keys — most of the core ideas in one
place:

```lua
local sprite = script.Parent
local speed = 240

game.Heartbeat:Connect(function(dt)
    local dir = 0
    if Input.IsKeyDown(Enum.KeyCode.Left) then dir -= 1 end
    if Input.IsKeyDown(Enum.KeyCode.Right) then dir += 1 end
    sprite.Position += Vec2.new(dir * speed * dt, 0)
end)
```

What you get to work with:

- **Globals**: `game`, `workspace`, `script`, `Input`, `Enum`, `Vec2`, `Color`,
  `UDim`, `UDim2`, `task`, `print`, `warn`, and the usual Lua standard library
  (`math`, `string`, `table`, …).
- **Events**: `game.Heartbeat:Connect(fn)` fires every frame with the delta
  time; a `Button` fires `Activated` when clicked. `Connect` returns a connection
  you can `:Disconnect()`.
- **Input**: `Input.IsKeyDown(Enum.KeyCode.Space)`, `Input.IsMouseDown(...)`,
  `Input.MousePosition()`, `Input.ViewportSize()`.
- **Instances**: `FindFirstChild`, `GetChildren`, `GetDescendants`, `IsA`,
  `Clone`, `Destroy`, and (for sprites) `GetTouchingSprites`. Read and write
  properties by name — `sprite.Tint = Color.new(1, 0, 0, 1)`.
- **Scheduler**: `task.wait(seconds)`, `task.spawn(fn)`, `task.defer(fn)`.
- **Saving data**: `game:GetService("DataStoreService"):GetDataStore("scores")`
  gives you `GetAsync` / `SetAsync` / `UpdateAsync` / `IncrementAsync`. In the
  editor it writes to a throwaway in-memory database by default; flip "Persist
  playtest data" if you want it to survive a stop.

Type definitions for editors that use the Luau Language Server live in
[`types/flux.d.luau`](types/flux.d.luau). There's more detail in
[`docs/script_editor.md`](docs/script_editor.md),
[`docs/luau_types.md`](docs/luau_types.md), and
[`docs/datastore_service.md`](docs/datastore_service.md).

## Project layout on disk

A project is a folder with a scene file and a `scripts/` directory:

```
my_game/
  main.scene.json      -- the object tree, saved from the editor
  scripts/
    main.luau
  assets/              -- images, etc. (optional)
```

`SourcePath` and texture paths are resolved relative to the project folder.

## Building a release

The optimised build produces two self-contained executables — the editor bakes
in its icons, the API metadata, and Luau, so you can copy the `.exe` on its own.

```sh
cargo build --release -p flux_editor -p flux_player
# -> target/release/flux_editor.exe, target/release/flux_player.exe
```

Pushing a `v*` tag builds and publishes a GitHub release automatically (see
[`.github/workflows/release.yml`](.github/workflows/release.yml)):

```sh
git tag v0.1.0
git push origin v0.1.0
```

## Repository layout

Flux is a Cargo workspace. The crates, roughly in dependency order:

| Crate | What it does |
| --- | --- |
| `flux_core` | The object/instance model, values, scene serialization, layout + transform math |
| `flux_data` | SQLite-backed storage for the DataStore service |
| `flux_render` | Asset classification and image loading |
| `flux_script` | Luau bindings (`mlua`) — the runtime API scripts call |
| `flux_runtime` | Ties the world + scripts into a running session you can step |
| `flux_view` | Draws a world to an egui painter |
| `flux_icons` | The editor's icon set (lucide) |
| `flux_editor` | The editor application |
| `flux_player` | The standalone game runner |

Tests live alongside the code (`cargo test --workspace`).

## Status

This is a work in progress and the API will still shift. Sprites are solid
colours unless you give them a texture; there's no audio engine yet (sound cues
just log); GUI text is basic. If something's rough, it probably just hasn't been
needed yet.

## License

MIT — see [LICENSE](LICENSE).
