# TomasCommander

A keyboard-first, standalone Windows file manager built in Rust with a native
Ledger-style UI. No browser, WebView, or JavaScript runtime.

## Run

Requires Windows, Rust 1.95+, and Visual Studio 2022 Build Tools with C++ and the
Windows SDK.

```powershell
cargo build --release --locked
.\target\release\tomas-commander.exe --root "D:\TomasCommander"
```

Includes two-pane navigation, selection, filtering, sorting, favorites, a command
palette, VS Code launching, light/dark themes and four accent colors.
Preferences live in `%LOCALAPPDATA%\TomasCommander\preferences.txt`.

## Keys

| Key | Action |
|---|---|
| Ctrl+Shift+P | Command palette |
| Tab / arrows | Switch pane / navigate |
| Space / Shift+arrows / Ctrl+A | Toggle / range / select all |
| Enter / Backspace | Open / parent folder |
| Ctrl+F / Ctrl+R | Filter / refresh |
| Ctrl+Shift+C / Ctrl+Shift+M | Copy / move |
| Delete | Recycle |
| Escape / Ctrl+Enter | Cancel / approve in confirmation |

Other commands are available in the palette. No F-keys or Z/Y bindings required.

## Safety and status

Every mutation requires exact-target approval. Existing destinations are never
overwritten. Cancellation can leave partial copies; Activity shows actual paths.
Copy does not preserve full ACLs, alternate streams or timestamps. Cross-volume
directory moves are unsupported. See the [operation policy](docs/decisions/file-operations.md).

AI/Foundry, MCP answers, voice, Copilot sessions, recursive search and remappable
shortcuts are not implemented. Physical CZ/US, multi-DPI and full performance
acceptance remain pending.

## Development

```powershell
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
```

Real-window checks: `tests\native-smoke.ps1` and `tests\native-visual.ps1`.
They use owned synthetic fixtures; the visual run temporarily controls its test
window, so leave the desktop idle. See the scripts for parameters.

[PRD](PRD.md) · [Goal Cards](docs/goals/README.md) ·
[Architecture and validation](docs/decisions/desktop-shell.md).
`prototypes\ui` is historical browser design exploration, not the product.
