# TomasCommander

A keyboard-first Windows workspace, built in Rust with the selected **Ledger**
design. The product is a standalone executable with egui/eframe and the Glow
renderer: no browser, WebView, JavaScript runtime, or local web server is needed.

## Run

Build prerequisites: Windows, Rust stable 1.95 or newer, and Visual Studio 2022
Build Tools with the C++ workload and Windows SDK. These prerequisites have been
installed on the development machine.

```powershell
cargo build --release --locked
.\target\release\tomas-commander.exe --root "D:\TomasCommander"
```

The release executable is also directly launchable from Explorer. Omit `--root`
to start in the working directory. Appearance and favorites are stored in
`%LOCALAPPDATA%\TomasCommander\preferences.txt`; `--settings` selects another file.
An invalid preferences file produces a visible error and is not overwritten.

## Current native slice

Real two-pane directory listings, virtualized rows, folder navigation, file
opening through Windows associations, name filtering, sorting, selection,
persisted favorites, and a contextual **Places / Activity** rail. Both light and
dark themes have four configurable accent families.

The deterministic command palette shares the same commands as the buttons.
Copy, move, and recycling use actual filesystem services and require approval
of exact source/destination paths. Existing destinations are refused, never
overwritten or merged. Copy is recursive; move uses the non-overwriting Windows
API. Delete means Windows Recycle Bin, not permanent deletion. VS Code launches
the actual standard-installed `Code.exe`, with the selected folder as an argument.

**Not delivered yet:** Foundry authentication/models, AI search/planning,
recursive filename search, WorkIQ/WebIQ/MCP answers, voice input, Copilot App
session launching, and remappable shortcuts. There are no simulated substitutes.
The feature contracts remain in [PRD.md](PRD.md) and [docs/goals](docs/goals/README.md).

## Keyboard controls

| Key | Action |
|---|---|
| Ctrl+Shift+P | Command palette; type to filter, arrows to choose, Enter to execute |
| Tab | Switch file pane outside text editors/dialogs |
| Up / Down / Home / End | Move the focused file row |
| Shift+Up / Down | Range selection |
| Space / Ctrl+A | Toggle focused selection / select visible rows |
| Enter / Backspace | Open folder or associated file / parent folder |
| Ctrl+F / Ctrl+R | Filter active folder / refresh |
| Ctrl+Shift+S | Cycle name, size, and modification-time sorting |
| Ctrl+Shift+C / Ctrl+Shift+M | Plan copy / move to the opposite pane |
| Delete | Plan recycling |
| Ctrl+Shift+Left / Right | Resize file panes |
| Escape | Clear selection or dismiss palette/approval |
| Ctrl+Enter in approval | Explicitly approve the displayed operation |

The palette also exposes favorites, pinning, activity, pane balancing, VS Code,
theme, and accent commands. In favorites mode opened from the palette, use
arrows and Enter; Escape or Tab returns to the panes. Text editors retain their
normal editing keys. No F-keys, Z/Y shortcuts, or punctuation chords are required.
Physical CZ/US keyboard verification is still pending.

## Safety and validation

Read the [operation policy](docs/decisions/file-operations.md) before using
mutations. Cancellation is cooperative; a partial copy is **not automatically
deleted**. Activity exposes exact created/completed paths. The first slice copies
file content, not complete Windows ACL/alternate-stream/timestamp fidelity.
Cross-volume directory moves are not supported.

```powershell
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
```

Tests create unique synthetic fixtures and remove only inventoried paths, without
recursive cleanup. No existing user files are destructive test inputs. To test
actual recycling and restore only that uniquely named owned fixture:

```powershell
cargo test --locked --test filesystem recycle_and_restore -- --ignored --nocapture
```

The actual-window accessibility smoke test exercises native appearance controls,
navigation, filtering, an approved real copy, collision refusal, and preferences:

```powershell
powershell.exe -NoProfile -NonInteractive -STA -File .\tests\native-smoke.ps1 `
  -EvidenceDirectory "C:\path\to\existing\evidence-directory"
```

For restricted manual testing, create a synthetic directory first and run:

```powershell
.\target\release\tomas-commander.exe --fixture-root "D:\owned-test-fixtures" `
  --settings "D:\owned-test-fixtures\preferences.txt"
```

The boundary restricts browsing and mutation sources/destinations. It does not
sandbox launched external applications. Do not place valuable files in fixtures.

## Architecture and evidence

`src\files.rs` owns listing, typed plans, validation, copying, and outcomes;
`src\platform.rs` owns Windows integration; `src\preferences.rs` owns persistence.
`src\app.rs` implements Ledger and dispatches background worker jobs through
bounded channels. Blocking directory enumeration and operations do not run while
drawing. Stale scans are cancelled; visible rows are virtualized.

The [desktop decision](docs/decisions/desktop-shell.md) records the framework,
dependency choices, observed baseline, and unverified requirements. Rust alone
does not establish performance acceptance.

`prototypes\ui` is historical, explicitly simulated browser design exploration;
it is not the application or its service layer.