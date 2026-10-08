# Tomas Commander

A keyboard-first, standalone Windows file manager built in Rust with a native
Ledger-style UI. No browser, WebView, or JavaScript runtime.

## Run

Requires Windows, Rust 1.95+, and Visual Studio 2022 Build Tools with C++ and the
Windows SDK.

```powershell
cargo build --release --locked
.\target\release\tomas-commander.exe --root "D:\TomasCommander"
```

Includes two-pane navigation, selection, filtering, recursive filename search,
bidirectional column sorting, asynchronous folder sizes/dates, persisted
favorites/recent documents, a command palette, VS Code/Copilot launchers,
light/dark themes and four accent colors.
Preferences live in `%LOCALAPPDATA%\TomasCommander\preferences.txt`.

## Keys

| Key | Action |
|---|---|
| Ctrl+Shift+P | Command palette |
| Tab / arrows | Switch pane / navigate |
| Space / Shift+arrows / Ctrl+A | Toggle / range / select all |
| Enter / Backspace | Open / parent folder |
| Ctrl+F / Ctrl+R | Filter / refresh |
| Ctrl+Shift+F | Search active folder and descendants |
| Ctrl+Shift+B | Focus pinned folders; arrows / Enter choose, Escape / Tab return |
| Ctrl+Shift+H | Focus Recent documents |
| Ctrl+Shift+C / Ctrl+Shift+M | Copy / move |
| Ctrl+Shift+D | Delete to the Recycle Bin, with approval |
| Ctrl+Shift+S | Cycle Name / Size / Last modified, ascending / descending |
| Escape / Ctrl+Enter | Cancel / approve in confirmation |

Other commands are available in the palette. No F-keys or Z/Y bindings required.
The pinned `..` row always remains above the files, including empty/filtered
folders. It is navigation only, never a selection or operation target.
Search uses real relative filenames, skips links/junctions, and supports
Enter to search, arrows/Enter to show a result, and cancellation.
Click a column header to sort; click it again to reverse direction.
Right-click toggles file selection. Recent documents records only document-like
files opened through this app, not Windows-wide activity.

Run GitHub Copilot CLI requests `copilot --yolo` in the active folder using the
Windows console/default-terminal host. It permits the launched CLI to act without
permission prompts. Run GitHub Copilot App requests `copilot app` in that folder;
the native CLI and registered `ghapp` handler are required. Actual external
terminal/session creation still needs end-to-end verification.

## Safety and status

Every mutation requires exact-target approval. Existing destinations are never
overwritten. Cancellation can leave partial copies; Activity shows actual paths.
Durable operation records prevent replay after restart; interrupted overlapping
operations require actual-state read-back, never automatic retry.
Copy does not preserve full ACLs, alternate streams or timestamps. Cross-volume
moves are unsupported. See the [operation policy](docs/decisions/file-operations.md).

Foundry supports an explicit prompt-only native pilot: open **Foundry**, configure
the nonsecret Azure resource/identity settings, then check the existing Azure CLI
sign-in and send once. No file data, tools, automatic retries or model fallback.
Verified identity/token reuse is memory-only and capped at 60 seconds; Check
forces fresh verification, and Disconnect clears the session.
Settings are in `%LOCALAPPDATA%\TomasCommander\ai-connection.json`; the five-attempt
pilot budget is durable. Cancel/deadline outcomes may still be billed.
See [AI runtime policy](docs/decisions/ai-runtime.md).

MCP answers, voice, AI file search/planning and remappable shortcuts are not
implemented. Physical CZ/US, multi-DPI and full performance acceptance remain pending.

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
