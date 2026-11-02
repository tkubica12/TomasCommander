# Release notes

## Unreleased

## v0.9.0 - 2026-11-02

Cursor-following internal previews and larger image support for Windows x64.

- Internal previews now follow the source cursor across supported files and ZIP
  members, without replacing an open source editor. Ctrl+Enter help explicitly
  describes the toggle and cursor-following behavior.
- Large JPEG/PNG/BMP previews are proportionally downscaled instead of rejected
  above a 4096-pixel edge. Local image input now allows 64 MiB with a bounded
  decode budget; original files stay unchanged.

Unsigned, unsupported and provided at your own risk.

## v0.8.0 - 2026-11-01

Same-folder file cloning and cleaner context menus for Windows x64.

- Clone a focused file in its current folder with Ctrl+Shift+K,
  the bottom action bar beside Copy, the file's context menu or `/clone`.
  A single name dialog confirms with Enter; version numbers, month/year
  tokens and numeric suffixes advance
  automatically, falling back to `-v2`. Existing names are skipped,
  overwrites are refused, and the verified copy supports Undo.
- Right-click menus show only action names, without keyboard-shortcut hints,
  for both regular files and ZIP entries.

Unsigned, unsupported and provided at your own risk.

## v0.7.1 - 2026-11-01

Shortcut fix for Windows x64.

- The ZIP command (create or extract by context) moves from Ctrl+Shift+P to
  Ctrl+Shift+A (archive). Ctrl+Shift+P now opens Ask like Ctrl+Space, matching
  the command-palette habit from other tools. Ctrl+A select all now requires
  exactly Ctrl, so Ctrl+Shift+A never selects all, also in ZIP views.

Unsigned, unsupported and provided at your own risk.

## v0.7.0 - 2026-11-01

Safer file operations, pane tabs, reviewed Undo, AI change proposals, PDF/XLSX
content for AI search and a local usage log for Windows x64.

- Copy/Move conflicts: the review offers Skip (default), Replace older, Replace
  all or Keep both, and same-name folders merge. Replaced files go to the
  Recycle Bin after a staged, verified copy; nothing is overwritten in place.
  Copies now keep each file's modification time. AI commands and ZIP flows still
  refuse existing destinations.
- Move across drives: copy, CRC-32 re-read verification and a source recheck,
  then originals go to the Recycle Bin. Removable and network drives have no
  Recycle Bin, so their originals are kept; there is no permanent-delete fallback.
- Reviewed Undo (Ctrl+Shift+U, `/undo` or Operations) shows the inverse of a
  recent operation as an exact plan: items move back, unchanged results are
  recycled and originals are restored from the Recycle Bin through staging,
  never overwriting. Undo is journaled, never automatic and cannot run twice.
  ZIP insertion, moving out of a ZIP and source Save are not undoable.
- Pane tabs (Ctrl+T, Ctrl+W, Ctrl+Tab / Ctrl+Shift+Tab, `/new-tab` ...) with
  per-tab folder history (Alt+Left/Right, mouse side buttons, header `‹` `›`,
  `/back`, `/forward`). The tab strip appears with two or more tabs and accepts
  drops. Tabs persist in preferences v6; missing or offline folders fall back to
  the nearest existing parent in the background, without freezing the window.
- AI change proposals: for prompts such as "Utřiď Downloads podle typu", the
  read-only file harness can propose up to 500 new folder, move, copy, rename
  and recycle steps that reference only files it inspected inside approved
  roots. The review has per-row checkboxes (deletes start unchecked); selected
  steps are checked first, then run as normal journaled operations that stop on
  any conflict. Operations offers Undo per step or for the whole batch.
- The AI file harness reads PDF text layers (Czech ToUnicode fonts included),
  local Windows OCR for up to 10 scanned pages per read, and XLSX/XLSM cached
  cell values. Parsers are bounded against hostile files and the worker has a
  1 GiB memory cap.
- Local usage log, on by default and never uploaded: commands and how they were
  invoked, unhandled keys, navigation routes, filters, Ask texts, dialogs,
  operations, errors and slow work, written by a background thread to daily
  JSONL files with 90-day retention. `/usage` shows insights and exports a
  Markdown report; `--usage-report --out report.md` writes it without a window.
  Settings > Usage log turns it off or deletes the data (preferences v7).
- `src\app.rs` is split into focused modules. Closing right after a settings
  change no longer leaves the window open.

Unsigned, unsupported and provided at your own risk.

## v0.6.0 - 2026-10-31

ZIP workflows and developer-file previews for Windows x64.

- ZIP Enter/double-click opens CRC-verified temporary copies in the default
  Windows application; Ctrl+Enter retains the internal preview. External dragging
  now accepts ZIP multi-selections and folder trees as Copy-only real Shell files.
  Failed/cancelled exports are cleaned; successful handoffs have a seven-day
  temporary-file lease and never update the source ZIP.
- Ctrl+Shift+P, `/zip` and a context-sensitive bottom action create ZIPs from
  selected files/folders or extract the focused ZIP/browsed members to the other
  pane, retaining destination review and no-overwrite checks.
- Native code/config previews add theme-aware syntax colors, selectable source,
  line numbers, wrapping and copying. JSON autoformat is view-only, with original
  copying and readable parse errors. Markdown/HTML can switch to original source.

- ZIP browsing uses compact headers without persistent tutorial/verification
  text and the same row typography, cursor outline and selection tint as file
  panes. Single-click focuses; double-click/Enter opens. Checkbox/range selection,
  paging and minimal scrolling keep keyboard and mouse navigation consistent.
- ZIP Extract and Extract all actions prepare extraction to the other pane,
  retaining exact-plan confirmation and no-overwrite checks. Extract all ignores
  the current ZIP subfolder/filter; the archive remains unchanged.

Unsigned, unsupported and provided at your own risk.

## v0.5.0 - 2026-10-20

Compact confirmations, improved file dragging and accent-insensitive search
for Windows x64.

- New folder creates immediately after name submission, without a second
  confirmation. Other file confirmations are compact and structured, with
  one-word actions and technical information under Details. Enter activates the
  initially focused confirmation; Escape cancels. Repeated Enter never approves.
- Unsaved-edit choices are Save, Discard and Cancel; Save is the default.
  Dialog, connection and consent actions use shorter labels without changing
  network permissions or request limits.
- File actions consistently use Ctrl+Shift: Ctrl+Shift+R refreshes the folder,
  Ctrl+Shift+N creates a new folder, and rename moves to Ctrl+Shift+E.
- Dragging from a selected file's checkbox now carries the entire selected group,
  like dragging its filename. Alt before or during a drag hands physical files to
  Windows as Copy and temporarily hides Commander to reveal the target application.
  Drop, Escape and failures restore the window without stealing target focus.
  Alt also works after Windows takes over the drag. The drag bubble shows only
  the filename or item count, without instructions.
- File-pane filters, recursive filename search and ZIP filters now ignore Czech
  diacritics as well as letter case (`cez` matches `ČEZ`), including decomposed
  combining accents. Original names and sorting remain unchanged.

Unsigned, unsupported and provided at your own risk.

## v0.4.0 - 2026-10-18

Unified Recent activity, configurable tracking and keyboard/file-workflow refinements
for Windows x64.

- Copy path now copies all selected paths, one per line, when invoked on a
  selected row; unselected rows still copy only their own path.
- Ctrl+Shift+S and `/sort` now cycle exactly three sorting modes: name A-Z,
  newest first, largest first, preserving focus, selection and filtering.
- PageUp/PageDown jump ten rows in file panes, like Ctrl+Up/Ctrl+Down.
- Clicking a noncurrent filename only focuses it. A further slow click on the
  current filename starts inline rename; rapid double-click still opens it.
  New input or dragging cancels pending rename.
- Unified Recent activity merges document changes in watched folders, Commander
  opens/previews/saves/operations and best-effort Windows Recent. Background native
  notifications, bounded startup discovery and durable newest-first history.
- Recent rows support Copy path, Windows Open with, preview, Open containing folder and
  internal/external dragging, with keyboard actions and stable row identity.
- Settings now separates Appearance, Recent activity and AI connections.
  Watched folders are configurable, defaulting to D: and detected personal/work
  OneDrives; changes apply in the background and persist separately from AI roots.
- Ctrl+Shift+H now toggles the workspace rail open/closed, opening on Recent.
  Places no longer has its own shortcut. Recent uses the file-pane font and row
  height, a minimal shortcut hint, and an explicit Open containing folder action.
  Tracking diagnostics stay in Settings/Operations instead of the Recent rail.
- A single click on a Recent file opens its containing folder and focuses the
  file; a double-click opens the file itself. Enter and dragging are unchanged.
- Recent also tracks ZIP archives, including external changes and successful
  internal archive opens, within the same 100-file limit.

Recent tracks metadata while Commander is running, not ZIP member contents.
Startup discovery is bounded; Windows Recent does not report every application's
internal previews. No background service or AI access expansion is added.

Unsigned, unsupported and provided at your own risk.

## v0.3.0 - 2026-10-14

Ask, read-only AI file discovery and external file dragging for Windows x64.

- Native Windows external file dragging (for example to Teams), offering Copy
  only without moving or deleting sources; internal reviewed pane dragging stays.
- Added Copy path to file/folder context menus to copy the full path and filename.
- Renamed the shared prompt/command surface to Ask, opened with Ctrl+Space.
- AI tasks run without blocking file panes; a notification bell shows progress
  and unread results. Hide preserves work; cancellation remains explicit.
- Read-only Foundry file harness in Ask with separately approved
  roots, listing/glob/grep/paged reads and DOCX/PPTX text extraction.
- Ten-call/two-minute segments with cancellation, retained partial results and
  explicit continuation; keyboard result activation and previews.
- Plain prompt submission no longer accidentally executes a matching command.
- Separate durable agent usage history; old pilot budgets remain untouched.
- Explicit medium reasoning and batched, path-filtered candidate globs; unrelated
  file metadata is no longer opened before filename/type filtering.
- Local-tool timeouts are distinguished from uncertain model submissions, and
  worker errors are displayed once.

AI requires an existing configured Foundry connection and separately approved
roots/content access; no roots or credentials are shipped. PDF/OCR, image,
spreadsheet and legacy Office content remain unsupported. Whole-drive search
quality and latency are not guaranteed. External dragging passes real files to
Windows targets; an actual Teams upload has not been verified.

Unsigned, unsupported and provided at your own risk.

## v0.2.1 - 2026-10-13

Directory metadata display fix for Windows x64.

- Keep last known directory size and modification time visible during background
  refresh, replacing both together when the new calculation arrives instead of
  briefly reverting to `...`.
- Preserve live change monitoring and explicit calculation errors. A tooltip
  identifies retained values while they refresh.

Unsigned, unsupported and provided at your own risk.

## v0.2.0 - 2026-10-13

Keyboard navigation and command-palette improvements for Windows x64.

- Typing in a file pane now goes directly into its filter; Down from the filter
  focuses the first matching file. Escape clears the filter from the field or list.
- Simplified AI search to a prompt and Search action, with progress/results and
  errors shown when needed. The existing local filename-search scope is unchanged.
- Added slash commands and multiline input to the command palette, retaining
  command-name filtering and integrating the existing AI filename search.
  Slash names use accented monospace labels, distinct from their descriptions.
- Workspace rail starts collapsed, with expand/collapse controls and keyboard
  access. Increased spacing below action buttons for shortcut hints.
- Removed transient folder-change notifications from the status bar; watched
  cache refreshes still run and errors remain visible.
- Added a Windows drive chooser when navigating above a drive root with `..`,
  Backspace or Ctrl+Left. Supports arrow keys, Enter, mouse selection and Escape;
  only the initiating pane changes.

AI remains the existing bounded prompt-to-filename interpretation, not a
tool-calling harness or whole-computer search. No new provider/data permissions.

Unsigned, unsupported and provided at your own risk.

## v0.1.0 - 2026-10-11

First experimental Windows x64 release of a personal, keyboard-first Rust
file manager.

- Native two-pane file browsing, selection, sorting, filtering and search.
- Reviewed copy/move/recycle/rename/new-folder operations and durable receipts.
- Named saved places, recent documents, compact breadcrumbs and inline rename.
- Native Markdown/HTML/PDF/image/text/JSON/CSV previews and guarded source editing.
- ZIP navigation, creation, insertion and extraction.
- Command palette, VS Code and GitHub Copilot launchers.
- Light/dark themes, configurable accent, global zoom and internal drag-and-drop.
- Limited opt-in Foundry pilots; connected answers remain experimental and
  unverified live. Voice and WorkIQ are not implemented.

Unsigned, unsupported and provided at your own risk. Developed with GitHub
Copilot; 0% of the code has been reviewed by a human. Back up important data.
