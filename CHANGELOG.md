# Release notes

## Unreleased

No unreleased changes.

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
