# DataStoreService

Flux exposes a small, Roblox-familiar `DataStoreService` to Luau scripts for
persistent key/value data. Scripts never see SQL or know which database is used —
everything routes through the `flux_data` persistence layer.

```
Luau script → DataStoreService → flux_runtime → flux_data → SQLite (Postgres later)
```

## API

```lua
local DataStoreService = game:GetService("DataStoreService")
local store = DataStoreService:GetDataStore("player_data")        -- name
local scoped = DataStoreService:GetDataStore("player_data", "u42") -- name + scope
```

| Method | Description |
| --- | --- |
| `store:GetAsync(key)` | Returns the stored value, or `nil` if unset. |
| `store:SetAsync(key, value)` | Writes `value` (overwrites). |
| `store:RemoveAsync(key)` | Deletes the key; returns the old value (or `nil`). |
| `store:IncrementAsync(key, amount)` | Adds `amount` (default `1`) to a number; returns the new number. Errors if the existing value is not a number. |
| `store:UpdateAsync(key, fn)` | Atomically transforms the value (see below). |
| `store:ListKeysAsync()` | Returns an array of keys in this store/scope. |

Every store is identified by `(scope, store_name)`; `scope` defaults to `"global"`.
Two stores, or two scopes, never collide.

### UpdateAsync

`UpdateAsync` reads the current value, calls your function with it, and writes
whatever the function returns — all as one atomic, version-checked transaction.
If the function returns `nil`, the key is removed.

```lua
-- Safe read-modify-write; never a naive get-then-set.
local newBest = store:UpdateAsync("best", function(old)
    return math.max(old or 0, score)
end)

-- Remove by returning nil.
store:UpdateAsync("temp", function(_) return nil end)
```

Do not call other DataStore methods from inside an `UpdateAsync` callback.

## Values and JSON

Values are serialized to JSON. Supported Luau types:

- `nil` (stored as JSON null; via `SetAsync` this stores null, `RemoveAsync` deletes)
- `boolean`
- `number`
- `string`
- `table` (arrays → JSON arrays, string/number-keyed tables → JSON objects)

Rejected with a clear Luau error:

- functions, userdata, threads → `cannot store a <type> value in a DataStore`
- cyclic tables → `cannot store a cyclic table in a DataStore`
- non-finite numbers (NaN/Infinity)

## Local SQLite behavior

SQLite (bundled, no system dependency) backs:

- editor playtesting
- local standalone player builds
- project-local saves

The database file lives at:

```
projects/<project_name>/.flux/data/playtest.sqlite
```

Schema (`flux_data`):

```
datastore_entries(
  id, scope, store_name, key, value_json, version, created_at, updated_at,
  UNIQUE(scope, store_name, key)
)
```

`version` increments on every write and is used for the `UpdateAsync`
compare-and-swap. `.flux/` is gitignored.

## Temporary vs persistent playtest data

In the editor, the **Playtest** menu controls this:

- **Persist playtest data = off (default):** the session uses a throwaway
  in-memory SQLite database. Nothing is written to disk; data is discarded when
  you press Stop. This keeps iterative playtesting clean.
- **Persist playtest data = on:** the session opens the project's
  `.flux/data/playtest.sqlite`, so values survive across Play/Stop and restarts —
  exactly what the standalone player uses.
- **Clear Playtest Data:** deletes the project's `playtest.sqlite` (only when not
  playing). Use it to reset saved state.

The standalone `flux_player` always uses the persistent project-local file.

## Failure handling

Persistence must never block playtesting. If the configured database cannot be
opened, the session falls back to a temporary in-memory database and prints a
clear error to the Output console, e.g.:

```
Persistence unavailable (database error: ...); using temporary in-memory data.
```

The game still runs.

## Limitations (current)

- Single local player only; no cross-client or cloud sync.
- No key size/quota limits, no request budgeting, no `GetSortedAsync`/ordered
  stores, no versioning history API (only an internal version counter).
- `UpdateAsync` callbacks must not call DataStore methods.
- Values are whole JSON documents; there is no partial/nested update besides
  `UpdateAsync` rewriting the value.

## Future: PostgreSQL

The `flux_data::PersistenceProvider` trait is the entire surface the runtime
depends on. Adding Postgres means:

1. Add a `PostgresProvider` implementing `PersistenceProvider` (same schema; use
   `INSERT ... ON CONFLICT` and a `SELECT ... FOR UPDATE` / version CAS for
   `update`).
2. Add a `DataBackend::Postgres { url }` variant and wire it in `flux_data::open`.

No script-facing code, and nothing in `flux_runtime`/`flux_editor`/`flux_player`
beyond choosing the backend, has to change. Editor playtesting stays on SQLite;
Postgres is for shared/hosted deployments.
