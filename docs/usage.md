# Usage and limits

## File work

The `..` row is navigation only, never an operation target.
At a filesystem root it opens a chooser of Windows drive letters, including
mapped/removable drives reported by Windows. Up/Down and Enter select; Escape
cancels. Backspace, Ctrl+Left and the header's `..` use the same route.
Fixture-root boundaries remain closed and never offer an escape to other drives.
Filter is local; recursive search examines actual filenames without following links/junctions.
Typing in a file pane appends to its filter and focuses the filter field.
Down from the filter focuses the first matching file; subsequent arrows navigate
the list. Escape clears a nonempty filter from either the field or the file list,
preserving selected files. With no filter, Escape still clears the selection.
Ctrl+F still focuses the filter explicitly. Text editors, dialogs,
the places rail and previews retain their own keyboard handling.
Selected rows operate on their selected group; an unselected row targets itself.
Rename always targets the focused item.
PageUp/PageDown jump ten rows, just like Ctrl+Up/Ctrl+Down.
Clicking a filename that is not current only moves the cursor. A further slow
click on the current filename starts inline rename; a rapid double-click opens
it instead. Starting a drag or another input cancels pending inline rename.
Ctrl+Shift+S cycles the active file pane through name A-Z, newest modified first,
and largest first, then repeats. From the default name sort, the first press
selects newest first. `/sort` uses the same cycle. The current column
header shows the mode/direction; clicking headers still allows either direction.
Filtering, selection and the focused file are preserved.
Right-click a file or folder and choose **Copy path** to copy its full Windows
path, including the filename, as plain text to the clipboard. On a selected row,
this copies every selected item's full path, one per line (Windows CRLF),
including selected items hidden by the filter. On an unselected row it copies
only that item's path. The `..` navigation row has no such action.

Drag a row or selected group to the opposite pane/folder. Move is the default;
hold Ctrl at release for Copy. Drops prepare a plan, not an immediate effect.
Escape cancels. Drag real files/folders outside the window to hand them to Windows
applications such as Teams or Explorer. External dragging offers Copy only:
the source is never moved/deleted, even with Shift. The target decides whether
it accepts the files; accepting a drop does not prove an upload completed.
Once Windows takes over, Escape cancels; release and start a new drag to return
to internal pane operations. Virtual ZIP members cannot be dragged externally,
and ZIP-to-ZIP transfers are unsupported.

Saved places are available in the left rail and Alt+Up popup. Edit changes the
display name; Delete removes only its bookmark.

### Recent activity

Ctrl+Shift+H toggles the workspace rail open/closed, regardless of keyboard focus.
Opening it selects Recent by default. This global list includes documents created/modified
in watched folders, successful Commander opens/previews/saves and operations,
plus local Windows Recent shortcuts when another application reports an open.
There is one entry per file, with the newest activity first, capped at 100 files.
A single click opens the containing folder in a file pane and focuses the file;
a double-click opens the file itself. Enter still opens the file directly.
Hover shows the full path, activity and age. Right-click offers Open, Preview,
**Open containing folder** (opens its parent in a file pane and focuses the file),
Windows **Open with...**, and Copy path. File names use the same font, size and
row height as the file panes. Drag a row to a pane for
a reviewed Copy/Move or outside the window for the existing Copy-only Shell drag.
Arrows select; Enter opens, Ctrl+Enter previews, Shift+Enter opens the system
application chooser, Alt+Enter locates, Ctrl+Shift+C copies the full path.

**Settings > Recent activity** configures watched folders, one absolute path per
line, including subfolders. Defaults are D: and detected personal/work OneDrive
folders. Apply validates and switches watchers in the background; unavailable
roots are reported, not silently accepted. Empty roots disable external tracking,
not explicitly requested Commander activity. The app keeps these paths separate
from AI access roots: Recent does not authorize AI reads or uploads.

Native notifications run only while Commander is open; there is no background
service. Startup scans modification times from the last 30 days, capped at
100,000 directory entries / five seconds, and imports up to 2,000 Windows Recent
shortcuts / two seconds. Details about bounded discovery, inaccessible trees and
notification overflows remain in Settings and Operations, not a persistent rail warning.
A bounded startup scan is not a complete
offline activity log. Windows Recent is best effort, restricted to watched roots;
internal previews in Total Commander or another app cannot be guaranteed.
Typical Office, PDF, HTML, Markdown, TXT, spreadsheet documents and ZIP archives
are included in the shared 100-file limit. Successful internal ZIP opens also
count as activity; archive members are not separate physical Recent files.
System/hidden files, temporary Office files, links/junctions and known
build/cache folders are excluded. Metadata monitoring does not read document
contents or deliberately hydrate OneDrive files; opening/previewing a file can.
Missing files are pruned. Fixture mode never discovers personal roots or reads
Windows Recent.

Folder sizes sum descendant files; dates show the newest descendant-file
modification time in UTC. They share a bounded watched in-memory cache.
Refresh forces revalidation. Remote/unmonitored/partial values have warnings.
See [cache policy](decisions/directory-cache.md).

## Reader

Ctrl+Enter uses the opposite pane as a reader. Repeating it on the same document
closes the preview; a different document replaces it. Escape closes with
unsaved-edit protection. Tab switches source/reader focus; arrows and Page
Up/Down scroll. Ctrl+Left/Right rotates image/PDF previews without changing files.
PDF pages use Ctrl+PageUp/PageDown or the Previous/Next controls.

Markdown is rendered natively, including headings, lists, tables and code.
Document images are omitted. HTML is parsed into semantic content, not rendered
as a web page: no CSS fidelity, scripts, remote assets or embedded controls.
Unsupported table/list structures fail explicitly. PDF uses Windows.Data.Pdf.
Images support JPEG/PNG/BMP, EXIF orientation, fit, zoom and quarter-turn rotation.
Reads/decoding run off the UI thread with bounded sizes.

Ctrl+E opens the original UTF-8 source, not a JSON-prettified/rendered copy.
Ctrl+S prepares Save for explicit approval. BOM and LF/CRLF are preserved;
mixed/legacy newlines, readonly files, external changes and invalid JSON fail.
Dirty close asks before discarding. Images/PDF and virtual ZIP members are not
editable. Failed writes can retain protected staging files: check Operations.

## ZIP

Enter opens a ZIP as a virtual directory without extraction. Backspace moves
up or exits at its root; a nonempty filter searches relative member names.
Enter on a member previews bounded CRC-verified contents.

Copy/move can extract selected members or insert files from the other pane.
Create ZIP is in Ask. Changes review physical/member paths and use staged
publication; move removes sources only after destination verification.
No overwrite/merge, ZIP-to-ZIP transfer or virtual Recycle Bin operation.
Only Store/Deflate; ZIP64, encrypted, split and unsafe/ambiguous archives fail.

## Settings and integrations

The workspace rail starts collapsed on every launch. Use its `>` / `<` controls
to expand/collapse it, or Ctrl+Shift+H to toggle it. Opening selects Recent.
The Places tab has no dedicated shortcut; saved places remain available through
the separate Alt+Up chooser while the rail is collapsed.
Settings has separate Appearance, Recent activity and AI connections tabs;
appearance controls stay immediate, while watched-folder changes use Apply.

Ctrl+Space opens Ask, with multiline input for AI and commands. Every existing action has a
slash command, for example `/rename`, `/preview`, `/settings` or `/copilot-cli`.
Typing words still filters commands by name; `/` filters slash commands.
Enter submits ordinary text to AI; slash input runs the highlighted command.
Click a command or select it with arrows then Enter to run a filtered command.
Shift+Enter inserts a newline, Escape closes.
Commands retain their existing dialogs and guarded effects; `/rename new.txt`
is not an inline rename operation and is rejected.

Use `/ai-search your prompt` or a free-text prompt to start the read-only file
harness. Selecting `/ai-search` without a prompt prepares the input. Arrow keys
navigate real results, Enter shows a result in the pane, Ctrl+Enter previews.
Escape cancels an active run without discarding known results; Escape again
closes it. A capped run offers explicit Continue.
Submission hides Ask immediately after validation. AI keeps running while you
work in the file panes. The header notification bell shows a running dot, then
an unread result badge; clicking it reopens the same task without starting another.
One AI task runs at a time; a second AI submission reopens the active task.
Slash commands remain available while AI runs. Hide or Ctrl+Space dismisses task
details without cancellation; Cancel/Escape cancels. Completion never opens a
modal automatically. The latest result is retained in memory until a new search
replaces it or the application closes.
`/search` opens deterministic local search;
`/search query` prefills it without sending anything to a provider.
There is no separate AI-search toolbar button.

Preferences: `%LOCALAPPDATA%\TomasCommander\preferences.txt`. v1-v4 remain
readable; v5 adds watched Recent folders. Appearance, zoom, favorites/names and
Recent folder settings persist. Activity is stored separately in
`recent-activity.json` beside preferences; legacy Recent entries migrate using
known modification times, not invented opening times. Operations use a durable
journal beside preferences.

In file panes, Shift+Enter opens VS Code/Copilot choices for the active context. These require
installed tools. Copilot App requests an idle folder session; CLI uses `--yolo`
and therefore grants the external agent its unattended execution mode.

AI remains a restricted experiment. Settings exposes Foundry connection under
its advanced panel. The app neither installs Azure CLI nor signs you in.
The file harness sends paths, metadata and supported contents only inside
separately approved roots, with optional automatic OneDrive hydration.
Its settings are in `%LOCALAPPDATA%\TomasCommander\ai-file-access.json`.
No configured roots means no access. Favorites alone never expand authority.
Each segment has ten model calls / two minutes; no lifetime spending cap is
configured. Usage is recorded without document contents; old pilot counters stay.
Text, DOCX and PPTX extraction are supported; other binary formats fail explicitly.
Model output never approves or executes filesystem effects.
See [AI runtime](decisions/ai-runtime.md),
[AI search](decisions/ai-search.md),
[commands](decisions/command-execution.md) and
[connected answers](decisions/connected-answers.md).
