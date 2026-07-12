--!nonstrict
-- Type definitions for the Flux engine's Lua/Luau scripting API.
--
-- These describe the globals the runtime injects into every Script (see
-- crates/flux_script). Point luau-lsp at this file (see .vscode/settings.json or
-- run luau-lsp with `--definitions=types/flux.d.luau`) to get autocomplete and
-- type checking for `game`, `Vec2`, `UDim2`, `Enum`, `Input`, `task`, etc.

-- ---------------------------------------------------------------------------
-- Math / value types
-- ---------------------------------------------------------------------------

declare class Vec2
	X: number
	Y: number
	Magnitude: number
	Unit: Vec2
	function __add(self, other: Vec2): Vec2
	function __sub(self, other: Vec2): Vec2
	function __mul(self, other: Vec2 | number): Vec2
	function __div(self, scalar: number): Vec2
	function __unm(self): Vec2
end

declare Vec2: {
	new: (x: number?, y: number?) -> Vec2,
	zero: Vec2,
	one: Vec2,
}

declare class Color
	R: number
	G: number
	B: number
	A: number
end

declare Color: {
	new: (r: number?, g: number?, b: number?, a: number?) -> Color,
}

-- One axis of a UDim2: `scale` (fraction of parent) + `offset` (pixels).
declare class UDim
	Scale: number
	Offset: number
	function __add(self, other: UDim): UDim
	function __sub(self, other: UDim): UDim
end

declare UDim: {
	new: (scale: number?, offset: number?) -> UDim,
}

-- Roblox-style UDim2 used for GuiObject Position/Size.
declare class UDim2
	X: UDim
	Y: UDim
	Width: UDim
	Height: UDim
	function __add(self, other: UDim2): UDim2
	function __sub(self, other: UDim2): UDim2
end

declare UDim2: {
	new: (xScale: number?, xOffset: number?, yScale: number?, yOffset: number?) -> UDim2,
	fromScale: (xScale: number?, yScale: number?) -> UDim2,
	fromOffset: (xOffset: number?, yOffset: number?) -> UDim2,
}

-- ---------------------------------------------------------------------------
-- Enums
-- ---------------------------------------------------------------------------

declare class EnumItem
	Name: string
	Value: number
	EnumType: string
end

type EnumKeyCode = {
	A: EnumItem, B: EnumItem, C: EnumItem, D: EnumItem, E: EnumItem, F: EnumItem,
	G: EnumItem, H: EnumItem, I: EnumItem, J: EnumItem, K: EnumItem, L: EnumItem,
	M: EnumItem, N: EnumItem, O: EnumItem, P: EnumItem, Q: EnumItem, R: EnumItem,
	S: EnumItem, T: EnumItem, U: EnumItem, V: EnumItem, W: EnumItem, X: EnumItem,
	Y: EnumItem, Z: EnumItem,
	Zero: EnumItem, One: EnumItem, Two: EnumItem, Three: EnumItem, Four: EnumItem,
	Five: EnumItem, Six: EnumItem, Seven: EnumItem, Eight: EnumItem, Nine: EnumItem,
	F1: EnumItem, F2: EnumItem, F3: EnumItem, F4: EnumItem, F5: EnumItem, F6: EnumItem,
	F7: EnumItem, F8: EnumItem, F9: EnumItem, F10: EnumItem, F11: EnumItem, F12: EnumItem,
	Space: EnumItem, Return: EnumItem, Tab: EnumItem, Backspace: EnumItem, Escape: EnumItem,
	Delete: EnumItem, Insert: EnumItem, Home: EnumItem, End: EnumItem,
	PageUp: EnumItem, PageDown: EnumItem,
	Up: EnumItem, Down: EnumItem, Left: EnumItem, Right: EnumItem,
	Minus: EnumItem, Equals: EnumItem, Plus: EnumItem, Comma: EnumItem, Period: EnumItem,
	Slash: EnumItem, BackSlash: EnumItem, Semicolon: EnumItem, Quote: EnumItem,
	Backquote: EnumItem, LeftBracket: EnumItem, RightBracket: EnumItem,
	GetEnumItems: (any) -> {EnumItem},
}

type EnumUserInputType = {
	MouseButton1: EnumItem, MouseButton2: EnumItem, MouseButton3: EnumItem,
	MouseMovement: EnumItem, Keyboard: EnumItem, Focus: EnumItem, Touch: EnumItem,
	GetEnumItems: (any) -> {EnumItem},
}

declare Enum: {
	KeyCode: EnumKeyCode,
	UserInputType: EnumUserInputType,
}

-- ---------------------------------------------------------------------------
-- Events
-- ---------------------------------------------------------------------------

declare class Connection
	function Disconnect(self): ()
end

declare class Signal
	function Connect(self, callback: (...any) -> ()): Connection
end

-- ---------------------------------------------------------------------------
-- Persistence (game:GetService("DataStoreService"))
-- ---------------------------------------------------------------------------

declare class DataStore
	function GetAsync(self, key: string): any
	function SetAsync(self, key: string, value: any): ()
	function RemoveAsync(self, key: string): any
	function IncrementAsync(self, key: string, delta: number?): number
	function UpdateAsync(self, key: string, transform: (any) -> any): any
	function ListKeysAsync(self): {string}
end

declare class DataStoreService
	function GetDataStore(self, name: string, scope: string?): DataStore
end

-- ---------------------------------------------------------------------------
-- Instances
-- ---------------------------------------------------------------------------

declare class Instance
	Name: string
	ClassName: string
	Parent: Instance?
	-- Fires every frame on `game`, with the delta time in seconds.
	Heartbeat: Signal
	-- Fires on a Button when it is clicked.
	Activated: Signal
	-- GuiObject only: computed absolute pixel rect against the viewport.
	AbsolutePosition: Vec2
	AbsoluteSize: Vec2
	function GetService(self, service: "DataStoreService"): DataStoreService
	function GetService(self, service: string): Instance
	function FindFirstChild(self, name: string): Instance?
	function GetChildren(self): {Instance}
	function GetDescendants(self): {Instance}
	function IsA(self, className: string): boolean
	function Clone(self): Instance
	function Destroy(self): ()
	-- Sprite only: overlapping sprites in the workspace.
	function GetTouchingSprites(self): {Instance}
	-- Child instances and engine properties are accessed dynamically by name.
	[string]: any
end

-- ---------------------------------------------------------------------------
-- Input
-- ---------------------------------------------------------------------------

declare Input: {
	IsKeyDown: (key: EnumItem | string) -> boolean,
	IsMouseDown: (button: EnumItem | string) -> boolean,
	MousePosition: () -> Vec2,
	ViewportSize: () -> Vec2,
}

-- ---------------------------------------------------------------------------
-- Scheduler
-- ---------------------------------------------------------------------------

declare task: {
	-- Yields the current thread for `seconds` and returns the elapsed time.
	wait: (seconds: number?) -> number,
	-- Runs `fn` immediately on a new thread.
	spawn: (fn: (...any) -> (...any), ...any) -> thread,
	-- Runs `fn` on a new thread at the end of the current frame.
	defer: (fn: (...any) -> (...any), ...any) -> thread,
}

-- ---------------------------------------------------------------------------
-- Globals
-- ---------------------------------------------------------------------------

declare game: Instance
declare workspace: Instance
-- The Script instance the current file runs as.
declare script: Instance

declare function warn(...: any): ()
-- Loads a Module instance and returns the value it returns (cached per module).
declare function require(module: Instance): any
