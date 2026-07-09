# Script editor intelligence

The Flux script editor has built-in Luau IDE features: autocomplete, hover
docs, signature help, and diagnostics. They work offline with no external LSP
(the luau-lsp definitions in [`luau_types.md`](luau_types.md) are separate and
only used by external editors like VS Code).

## Features

- **Autocomplete** — suggestions while typing letters, after `.`/`:`, and after
  `Enum.` / `game:GetService(`. Includes keywords, engine globals, known
  services, API members, and locals/functions declared in the current file.
  Up/Down navigate, Enter/Tab accept, Escape dismisses, and `.`/`:` commit the
  highlighted item and reopen member suggestions.
- **Hover** — a tooltip describing the symbol under the pointer (globals,
  members, and file-local declarations).
- **Signature help** — parameter hints while the cursor is inside a call, with
  the active argument highlighted.
- **Diagnostics** — a real front-end (lexer → parser → AST → semantic passes),
  not just token checks. Severities render as: error = red squiggle, warning =
  yellow squiggle, information = blue squiggle, hint = gray dotted underline.
  Hovering a squiggle shows every overlapping message; the gutter and header
  show counts. Coverage includes:
  - **Syntax** (parser): unexpected token, missing `end`/`)`/`]`/`}`, unclosed
    string, invalid number literal, invalid function syntax. Parsing recovers so
    one broken line doesn't blank out the rest.
  - **Scope**: undefined variable (with "did you mean?"), duplicate
    declaration, duplicate parameter, shadowing.
  - **Types** (lightweight inference): calling a non-function, arithmetic on a
    string/boolean, indexing a boolean/number.
  - **Builtins**: unknown method/member on a *known* type with a Levenshtein
    suggestion (e.g. `game:GetServi()` → "did you mean `GetService`?"); wrong
    argument counts; `.` vs `:` method-call mistakes.
  - **Flow / lints**: unreachable code, `break`/`continue` outside a loop,
    unused locals/parameters, empty loops, empty/constant/duplicate `if`
    conditions, duplicate table keys.

  Checks are conservative — they only fire when the analyzer is confident — so
  dynamic types (e.g. `Instance` children) and large standard libraries are not
  over-validated.
- **Go to error** — clicking a runtime error in the Output console opens the
  file at the reported line/column.
- Current-line highlight, matching-bracket highlight, and a Ln/Col readout.

## Architecture

Everything lives behind `crates/flux_editor/src/language/`, and the editor UI
(`script_editor.rs`) only talks to `ScriptLanguageService`. Two cooperating
halves:

**Interactive** (completion, hover, signature help) — fast token-based
resolution:

| Module          | Responsibility                                             |
| --------------- | ---------------------------------------------------------- |
| `lex.rs`        | Byte-level Luau tokenizer used by completion/hover.        |
| `symbols.rs`    | `SymbolIndex`: locals/functions/params + inferred types.   |
| `api.rs`        | Loads the API metadata and resolves member chains.         |
| `context.rs`    | Cursor-context detection (member vs. identifier vs. call). |
| `completion.rs` | `CompletionProvider`.                                      |
| `hover.rs`      | `HoverProvider`.                                           |
| `signature.rs`  | `SignatureHelpProvider`.                                   |

**Diagnostics** — a full front-end pipeline (`token → parser → ast → semantic`),
every stage writing into one `Diagnostics` collector:

| Module           | Responsibility                                                |
| ---------------- | ------------------------------------------------------------ |
| `diagnostics.rs` | `Diagnostic`/`DiagnosticSeverity`/`Span`, sink + `LineIndex`. |
| `token.rs`       | Full lexer (multi-char ops, number/string validation).       |
| `ast.rs`         | AST node types with spans.                                    |
| `parser.rs`      | Error-recovering recursive-descent parser → partial AST.     |
| `scope.rs`       | Lexical scope + symbol table (declare / resolve / unused).   |
| `types.rs`       | Lightweight type lattice (`Ty`) + inference.                 |
| `builtin.rs`     | Builtin symbol database (wraps `api.rs`).                     |
| `flow.rs`        | Control-flow reachability.                                    |
| `suggestions.rs` | Levenshtein "did you mean?".                                  |
| `semantic.rs`    | Analyzer orchestrating scope/type/builtin/flow passes.       |
| `mod.rs`         | `ScriptLanguageService` facade + per-buffer caching.         |

Analysis is cached per buffer (hashed), so nothing is recomputed while the text
is unchanged. The AST + scope + symbol tables are the foundation for scaling
into a full language server (go-to-definition, rename, find-references, code
actions). The parser is hand-written and error-recovering — no external
`luau-lsp` required — and can be swapped for a richer Luau parser without
touching the UI or the interactive providers.

## API metadata

Descriptions, parameters, return types, and members come from
[`assets/api/flux_luau_api.json`](../assets/api/flux_luau_api.json), embedded in
the editor with `include_str!`. When you add a global, instance method,
property, or `Enum` member to the runtime (`crates/flux_script`), update that
JSON (and `types/flux.d.luau`) to match.
