# Luau type definitions

Flux ships type definitions for its scripting API so editors using the
[Luau Language Server](https://github.com/JohnnyMorganz/luau-lsp) get
autocomplete, hover docs, and type checking in `.luau` scripts.

## Files

- [`types/flux.d.luau`](../types/flux.d.luau) — the definitions: `game`,
  `workspace`, `script`, `Vec2`, `Color`, `UDim`, `UDim2`, `Enum`, `Input`,
  `task`, `Instance`, `Signal`/`Connection`, and `DataStore(Service)`.
- [`.luaurc`](../.luaurc) — sets `languageMode` to `nonstrict` (instances expose
  children and engine properties dynamically by name) and registers the engine
  globals for the linter.
- [`.vscode/settings.json`](../.vscode/settings.json) — points luau-lsp at the
  definitions and selects the **standard** (non-Roblox) platform.

## VS Code

Install the **Luau Language Server** extension. The workspace settings load
`types/flux.d.luau` automatically — no extra steps.

## Other editors / CLI

Pass the definitions to luau-lsp directly:

```sh
luau-lsp analyze --definitions=types/flux.d.luau --settings='{"luau-lsp.platform.type":"standard"}' projects/**/scripts/*.luau
```

## Keeping types in sync

The definitions mirror what the runtime injects in `crates/flux_script`
(`setup_globals`, `instance.rs`, `types.rs`, `enums.rs`, `datastore.rs`). When
you add a global, instance method, property, or `Enum` member there, update
`types/flux.d.luau` to match.
