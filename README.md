# Tomas Commander

My personal, made-to-measure file manager for the modern era: a standalone
Windows application built in Rust, with a native UI and a goal of being very
fast, lightweight and keyboard-first.

Inspired by Total Commander and Norton Commander, it brings two-pane file work,
in-app reading and editing, and developer/agent launchers into one clean,
terminal-inspired workspace. No browser engine, WebView or JavaScript runtime.
AI commands and more AI-assisted workflows are part of the direction; the
existing AI integrations are limited experiments, not general-purpose agents.

**Use entirely at your own risk. This is a personal project with zero support
and no guarantees. Developed with GitHub Copilot; 0% of the code has been
reviewed by a human. Back up your files before using it.**

![Tomas Commander in dark mode with a blue accent](assets/screenshot-dark-blue.jpg)

Actual v0.1.0 application screenshot; the displayed folder listing was captured
before the release cleanup.

## Download and run

Download `tomas-commander.exe` from [GitHub Releases](https://github.com/tkubica12/TomasCommander/releases).
The latest version is **v0.9.0**, an experimental Windows x64 release.
No installer or Rust toolchain is needed to run the downloaded executable.
The binary is unsigned; this project does not provide code-signing or support.

```powershell
.\tomas-commander.exe
# Optional initial folder (one tab per pane; saved tabs stay for later runs):
.\tomas-commander.exe --root "C:\Projects"
```

## Features

- Two file panes, keyboard navigation, multi-selection, filtering, recursive
  filename search and sortable size/date columns.
- Tabs in each pane (Ctrl+T, Ctrl+W, Ctrl+Tab) with their own folder, sort,
  filter, focus and Back/Forward history (Alt+Left/Right or mouse side
  buttons); tab folders are restored at the next launch.
- Copy, move, Recycle Bin deletion, rename and new folder, with
  exact-path confirmation and durable operation receipts. Copy/Move conflicts
  offer Skip, Replace older, Replace all (originals to the Recycle Bin) or Keep
  both, and same-name folders are merged. Moves across drives copy, verify
  (CRC-32) and then send originals to the Recycle Bin.
- Reviewed Undo (Ctrl+Shift+U, `/undo` or Operations): the inverse of a recent
  operation is shown as an exact plan before anything changes. It moves items
  back, recycles unchanged results and restores originals from the Recycle Bin
  without overwriting; nothing is undone automatically.
- Saved locations with editable names and removal, unified Recent activity, compact
  clickable breadcrumbs and inline filename editing.
- Recent document and ZIP-file changes in configurable folders (D: and both detected OneDrive
  roots by default), Commander opens/views/saves and best-effort Windows Recent.
  Ctrl+Shift+H toggles the rail, opening on Recent; Places has no dedicated shortcut.
  Recent rows match file-pane typography and support preview, Copy path, system
  Open with, Open containing folder and dragging.
  Settings separates Appearance, Recent activity, AI connections and Usage log.
- Native previews for Markdown, semantic HTML, PDF, JPEG/PNG/BMP, UTF-8
  text/config/source files, formatted JSON and comma-delimited CSV.
  Code/config previews have syntax colors, selectable text, line numbers,
  wrapping and one-click copying; Markdown switches between rendered and source.
- Source editing with guarded Save; ZIP browsing, creation, insertion and
  extraction with reviewed operation plans.
- Ask for AI prompts and slash commands, and Open with choices for VS Code, GitHub Copilot CLI and
  GitHub Copilot App, when installed. The read-only `/ai-search` file harness
  reads text, DOCX/PPTX, XLSX/XLSM cell values and PDF text (scanned pages via
  Windows OCR) inside approved roots.
- AI change proposals: ask to sort, rename, move, copy or delete files and the
  harness proposes up to 500 changes without touching anything. The bell shows
  *Proposal ready: N changes*; the review groups new folders, moves, copies,
  renames and deletes (deletes start unchecked). You select and approve; every
  selected step is checked first, then runs as a normal journaled operation
  with no overwrite, deletes go to the Recycle Bin, and Operations offers Undo
  per step or for the whole batch.
- Light/dark mode, four accent choices, proportional UI zoom and internal
  drag-and-drop: Move by default, Ctrl for Copy.
- Native Windows dragging of real files to external applications, offering Copy
  only. ZIP selections are automatically extracted to temporary copies for
  external opening and dragging, including multiple files and folders.
- Local usage log for reviewing your own habits, for example with an AI
  assistant: commands and whether they came from a shortcut, the mouse or Ask,
  keys that did nothing, navigation routes, unmet Ask texts, cancelled dialogs,
  errors and slow work. `/usage` shows Usage insights and exports a Markdown
  report; `tomas-commander.exe --usage-report --out report.md` writes it without
  a window. Recording never waits for the disk.

## Keyboard essentials

| Shortcut | Action |
|---|---|
| Tab / arrows | Switch pane / navigate |
| Enter / Backspace | Open / parent folder |
| Ctrl+Left / Ctrl+Right | Parent / return after parent navigation |
| Alt+Left / Alt+Right or mouse back / forward | Back / forward in the active tab's folder history |
| Ctrl+T / Ctrl+W | New tab at the current folder / close tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab in the active pane |
| PageUp / PageDown or Ctrl+Up / Ctrl+Down | Jump ten rows |
| Space / Shift+arrows / Ctrl+A | Toggle / range / select all |
| Ctrl+Enter | Toggle internal preview; it follows the source file cursor |
| Shift+Enter | Open with menu |
| Ctrl+Space / Ctrl+Shift+P | Ask / AI prompts and slash commands |
| Ctrl+Shift+H | Toggle workspace rail, opening on Recent |
| Alt+Up | Saved places, including Add current folder |
| Ctrl+F / Ctrl+Shift+F | Filter / recursive filename search |
| Ctrl+Shift+S | Cycle sorting: name A-Z / newest first / largest first |
| Ctrl+Shift+C / Ctrl+Shift+M / Ctrl+Shift+D | Copy / move / Recycle Bin |
| Ctrl+Shift+K | Clone focused file in the same folder |
| Ctrl+Shift+U | Undo: review the inverse of the latest undoable operation |
| Ctrl+Shift+N / Ctrl+Shift+E / Ctrl+Shift+R | New folder / rename / refresh |
| Ctrl+Shift+A | Create ZIP / extract ZIP by context (archive) |
| Ctrl+plus / Ctrl+minus / Ctrl+wheel | Global UI zoom |
| Ctrl+0 | Reset zoom |
| ? / status-bar Help | Shortcut help |

Typing in a file pane goes directly into its filter. Down from the filter
focuses the first matching file; further arrows navigate the list. Escape clears
the filter from the field or file list.
Ask has multiline input and slash actions such as `/rename`
and `/preview`, alongside command-name filtering. Enter runs; Shift+Enter
inserts a newline. `/ai-search your prompt` starts the read-only Foundry file
harness inside explicitly configured roots. Ordinary text Enter goes to AI;
click or explicitly select a filtered command with arrows to run it.
AI submission closes Ask and runs in the background, leaving both file panes
usable. The header bell opens progress, partial results or the latest answer;
an unread badge appears on completion without interrupting work. One AI task
runs at a time. Hide keeps it running; Cancel/Escape stops it. Continue is explicit.
A change request ends with *Proposal ready: N changes*; the bell opens its
review, where Space toggles the focused row and Enter runs only from the
focused Run button.
The workspace rail starts collapsed; expand it with `>` or toggle it with
Ctrl+Shift+H. See [release notes](CHANGELOG.md) for version history.

No function keys or Z/Y bindings are required. Mouse access remains available.
At a drive root, opening `..`, Backspace, Ctrl+Left or the header's `..` button
opens the Windows drive chooser. Use Up/Down and Enter, or click a drive;
Escape cancels. The chosen drive replaces only the active pane. Unavailable
or disconnected drives report an ordinary folder-read error.
Click a filename to focus it. A further slow click on the current filename
starts in-place editing; Enter reviews the rename and Escape cancels.
Rapid double-click opens normally; Ctrl-double-click previews and
Shift-double-click opens the chooser.
Clone is also available beside Copy in the bottom action bar, by right-clicking
a regular file or using `/clone`.
Right-click menus show action names only; shortcut hints stay in Help,
Ask and the action bar.
Its simple name dialog creates the copy with Enter, without another review.
It advances version tokens (`v1`, `ver1`, `verze1`, `version1`, `rev1`, with
optional separators), month/year tokens (`Jun26` to `Jul26`, `Dec26` to
`Jan27`), or trailing `-1` / `_1`; otherwise it appends `-v2` before the
extension. Existing names are skipped, and an edited name never overwrites
an existing item. The original is kept; the copy uses the normal verified
Copy workflow and can be undone.
Inside a ZIP, Enter/double-click extracts a temporary file and opens its Windows
default application; Ctrl+Enter/Ctrl-double-click stays in the internal preview.
The bottom ZIP action creates an archive from selected files/folders, or extracts
the selected ZIP/its browsed members to the other pane, with reviewed destinations.

Preferences, saved-location names and appearance persist in
`%LOCALAPPDATA%\TomasCommander\preferences.txt`.

## Important limits

Confirmations are safeguards, not a guarantee against data loss. Ordinary file
operations never overwrite in place: Copy/Move replacement sends the existing
file to the Recycle Bin and is chosen explicitly in the review; AI commands, AI
change proposals and ZIP flows do not overwrite or merge existing destinations. Cancellation can
leave partial outputs or staging files; inspect **Operations** before retrying.
Copy and moves across drives preserve modification times but not creation
times, all Windows metadata, ACLs or alternate streams. Moves between drives are
a verified copy followed by sending the originals to the Recycle Bin; from
removable or network drives, which have no Recycle Bin, originals are kept and
never deleted permanently. Undo covers Copy, Move, Delete, Rename, New folder
and ZIP create/extract, but not ZIP insertion, moving out of a ZIP or source
Save; it refuses when results changed, originals are no longer in the Recycle
Bin or kept originals are gone, and it cannot itself be undone.

The usage log is local only and never uploaded, but it contains full folder and
file paths and Ask texts (never file contents, clipboard, AI answers or
credentials). It is on by default; turn it off or delete it in Settings > Usage
log. Files live in `%LOCALAPPDATA%\TomasCommander\usage\` and are deleted after
90 days. See [usage log](docs/decisions/usage-log.md).

HTML previews are semantic, not full CSS/browser rendering; scripts and external
assets are not loaded. ZIP support is limited to Store/Deflate and excludes
encrypted, split and ZIP64 archives. Voice, remappable shortcuts and incoming
Explorer drag-and-drop are not implemented. Broad hardware, performance,
accessibility and physical CZ/US keyboard acceptance are not complete.

Copilot CLI is launched with `--yolo`, allowing that external agent to act
without its usual permission prompts. Only use it in a folder you trust.

The read-only Foundry file harness requires separately approved roots and
supports names, metadata, text, DOCX/PPTX text, XLSX/XLSM cached cell values
(never macros or formula evaluation) and PDF text layers. Up to 10 scanned PDF
pages per read get local Windows OCR (an installed OCR language is needed);
grep never OCRs. Encrypted PDFs/Office files, legacy .doc/.xls/.ppt and images
are not read as content. It has ten calls /
two minutes per segment, explicit continuation and no lifetime spending cap.
It uses the standard Responses tool loop with explicit medium reasoning, not
subagents. Batched candidate globs can search approved roots and file types in
one call, filter names/folder paths, and avoid opening unrelated file metadata.
AI change proposals never change files by themselves: only the steps you check
and approve run, each re-checked and stopped on any conflict; a batch is not
atomic, so a failure leaves earlier steps done (and undoable). A proposal can
still be wrong; review it. See [AI change proposals](docs/decisions/ai-changes.md).
Prompt-only interpretation/file-command pilots still require your own
explicit configuration, consent and existing Azure CLI sign-in. They have
durable attempt limits and no automatic retries. Connected WebIQ answers have
an experimental app-side implementation but no completed live acceptance;
WorkIQ is not a working integration. AI data/budget boundaries are described
in [AI runtime](docs/decisions/ai-runtime.md) and
[connected answers](docs/decisions/connected-answers.md).

## Build from source

Windows, Rust 1.95+ and Visual Studio 2022 Build Tools with C++/Windows SDK
are required.

```powershell
.\scripts\build.ps1
.\release\tomas-commander.exe --root "C:\Projects"
```

`release\tomas-commander.exe` is the single current local executable. `release`
and Cargo's `target` cache are gitignored; published versions live in GitHub
Releases. Close the local app before rebuilding. A failed build/publication
preserves the previous executable.

```powershell
cargo test --locked
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
.\tests\build-workflow.ps1
```

Diagnostic features are nonshipping. Build them only with
`.\scripts\build.ps1 -Mode Diagnostic -Features native-desktop-tests`, then
close the diagnostic process and run
`.\scripts\build.ps1 -Mode CleanDiagnostic`.

The existing local WinHTTP timeout regression is intermittent under a broad
parallel test run; an isolated rerun can pass. This is a known validation limit,
not evidence that the full suite is consistently green.

[Usage and format limits](docs/usage.md) ·
[Architecture](docs/decisions/desktop-shell.md) ·
[File-operation policy](docs/decisions/file-operations.md) ·
[AI change proposals](docs/decisions/ai-changes.md) ·
[Usage log](docs/decisions/usage-log.md) ·
[Roadmap](docs/roadmap.md) · [Release notes](CHANGELOG.md)

Licensed under [MIT](LICENSE).
