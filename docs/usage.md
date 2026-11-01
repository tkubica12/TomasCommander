# Usage and limits

## File work

The `..` row is navigation only, never an operation target.
At a filesystem root it opens a chooser of Windows drive letters, including
mapped/removable drives reported by Windows. Up/Down and Enter select; Escape
cancels. Backspace, Ctrl+Left and the header's `..` use the same route.
Fixture-root boundaries remain closed and never offer an escape to other drives.
Filter is local; recursive search examines actual filenames without following links/junctions.
File-pane filters, recursive filename search and ZIP filters ignore letter case
and Czech diacritics: `cez` matches `ČEZ`. File names and sorting are unchanged.
Typing in a file pane appends to its filter and focuses the filter field.
Down from the filter focuses the first matching file; subsequent arrows navigate
the list. Escape clears a nonempty filter from either the field or the file list,
preserving selected files. With no filter, Escape still clears the selection.
Ctrl+F still focuses the filter explicitly. Text editors, dialogs,
the places rail and previews retain their own keyboard handling.
Selected rows operate on their selected group; an unselected row targets itself.
Rename always targets the focused item.
New folder needs only a name and Enter (or Create); there is no second
confirmation. Other file changes show a compact review: Enter confirms, Escape
cancels. The confirming button has initial focus; Tab can select Cancel instead.
Technical details remain available under Details. A held Enter does not confirm
a subsequent dialog.
Copy and Move (buttons, Ctrl+Shift+C/M, `/copy`, `/move` or dragging between
panes) check the destination for items that already exist. Without conflicts
the review is unchanged. Otherwise it adds a compact conflict section: new items,
folders merged, file conflicts split into source newer / source older or same,
and up to five example names; Details lists every conflict. Choose **Skip**
(default), **Replace older**, **Replace all** or **Keep both** with the mouse,
or Tab to the choice and use Left/Right. Each change prepares a new plan;
confirmation stays disabled until it is ready, and the confirm button shows
replacements, for example **Copy + replace 3**. Folders with the same name are
merged. Replaced files go to the Recycle Bin; nothing is overwritten in place.
Replace older replaces only when the source is strictly newer. Keep both adds
` (2)`, ` (3)`, ... before the extension. A file never replaces a folder or vice
versa; links/junctions in the destination stop planning. Moving into a merged
folder moves its contents and removes emptied source folders; folders that
still hold skipped items stay. Copies keep the source's modification time.
Operations lists replaced (original in Recycle Bin), merged and skipped paths.
AI commands still refuse any existing destination.
Move between different drives (for example C: to a USB disk or D:) cannot rename,
so it copies, verifies and then recycles: each file is copied with its
modification time, re-read and compared by CRC-32, and only after a whole
selected item is verified and rechecked unchanged does its original go to the
Recycle Bin (one entry per selected item; if conflicts skipped some files, only
the moved files go and emptied folders are removed, skipped files stay). The
review says **Across drives: copy, verify, then send originals to the Recycle
Bin**; Details shows the strategy. Removable and network drives have no Recycle
Bin, so the review says the originals are kept, and the Move only copies and
verifies. A verification mismatch, a source changed during the copy, a failed
Recycle Bin hand-off or cancellation stops before recycling the affected
original; Operations reports what was copied, recycled and kept. Originals are
never deleted permanently. AI commands move only within one drive.
Prompt editors keep their existing submit/newline shortcuts; they are not
confirmation dialogs.
Ctrl+Shift+N creates a new folder, Ctrl+Shift+E renames the focused item,
and Ctrl+Shift+R refreshes the active folder. Ctrl+N and Ctrl+R are no longer
file-pane shortcuts.
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
A drag to another drive prepares the same reviewed copy-verify-recycle Move.
Dragging any selected file (including its selection checkbox) carries the entire
selected group, including selected files hidden by the filter. An unselected
file is dragged alone.
Press **Alt before or during a drag** to send physical files straight to another
application: Commander temporarily hides so the target underneath is accessible,
then reappears without taking focus after drop, Escape or failure.
Alt also works after the pointer has left Commander and Windows owns the drag.
Release Alt once the window hides; keep holding the mouse button until the drop.
This is an external **Copy-only** drag; plain dragging between panes is unchanged.
The drag bubble shows only the filename or item count, without shortcut hints.
Escape cancels. Drag real files/folders outside the window to hand them to Windows
applications such as Teams or Explorer. External dragging offers Copy only:
the source is never moved/deleted, even with Shift. The target decides whether
it accepts the files; accepting a drop does not prove an upload completed.
Once Windows takes over, Escape cancels; release and start a new drag to return
to internal pane operations. ZIP members and multi-selections use the same
external Copy-only drag after background extraction to verified temporary files.
Alt can reveal the target under Commander for these ZIP drags too.
ZIP-to-ZIP transfers are unsupported.

Saved places are available in the left rail and Alt+Up popup. Edit changes the
display name; Delete removes only its bookmark.

### Tabs and folder history

Each pane holds one or more tabs; exactly one is active. Ctrl+T (`/new-tab`)
opens the current folder in a new tab right of the active one and switches to
it. Ctrl+W (`/close-tab`) closes the active tab and activates its right
neighbour (or the new last tab); the last tab cannot be closed. Ctrl+Tab and
Ctrl+Shift+Tab (`/next-tab`, `/previous-tab`) cycle with wrap-around. These
chords also work while typing in the pane filter; plain Tab still switches
panes. They apply to the active file pane, not to a reading pane.

Every tab keeps its own folder, sort column and direction, filter text, focused
item, selection, Ctrl+Left/Right parent returns and Back/Forward history.
Inactive tabs keep no listing, scan or folder watcher: switching to a tab loads
its folder like Refresh (folder sizes come from the shared cache when still
valid). Folders are checked in the background, so a slow or offline network
folder never freezes the window. If the folder has disappeared or cannot be
reached, the tab shows its nearest existing parent instead and the status bar
says `Folder X is not available; showing Y.`; without any existing parent (for
example an unplugged drive) the pane shows the usual folder-read error.
Selected items that no longer exist are dropped.

With two or more tabs, a single strip at the top of the pane header shows each
folder name (the drive, such as `D:\`, for roots); hover shows the full path.
The active tab uses the accent colour. Click a tab to activate it, middle-click
it or use its `×` to close it, and `+` for a new tab. Tabs shrink in narrow
panes and the strip scrolls sideways when they still do not fit. With one tab
the pane looks as before. Dropping files on a tab prepares the usual reviewed
Copy/Move plan into that tab's folder.

A tab browsing a ZIP shows the ZIP name. New, close and switch commands first
close that ZIP view; returning to the tab shows the folder containing the ZIP
with the ZIP focused. Back/Forward are not available inside a ZIP view.

Alt+Left / Alt+Right (`/back`, `/forward`), the mouse side buttons over a pane,
or the header's `‹` `›` buttons move through the active tab's folder history.
Every navigation (Enter, Backspace, Ctrl+Left/Right, breadcrumbs, typed paths,
saved places, drive choice, Recent and search results) records the folder it
leaves and clears Forward; Back and Forward move between the two lists without
clearing them. Refresh and operation updates record nothing, and opening the
folder already shown is not recorded twice. Each direction keeps 50 folders.
Back and Forward open the remembered folder without checking it first; one
that no longer exists or cannot be reached falls back in the background to its
nearest existing parent with the same status message, and that parent takes
its place in the history. Typed paths, Enter and other ordinary navigation
still report an unavailable folder as an error.
Ctrl+Left/Ctrl+Right keep their parent/return behaviour; going to the parent is
also an ordinary history step.

Open tab folders and the active tab of each pane are saved when Commander
closes (and with any other settings change) and restored at the next launch.
No saved folder is checked before the window opens: each pane's active tab
loads in the background and falls back to its nearest existing parent like a
switched-to tab, and inactive tabs are checked only when activated, so an
unreachable network folder never delays startup. `--root` opens one tab per
pane at that folder for that run and neither restores nor overwrites the saved
tabs. In fixture mode (`--fixture-root`) every saved tab is checked at launch
and only folders inside the fixture are restored (a pane without any opens the
fixture root). Fixture runs restore and save tabs only with an explicit
`--settings` file, so they never replace the tabs in your default preferences.

### Undo

Ctrl+Shift+U or `/undo` prepares the inverse of the most recent operation that
can still be undone and opens the standard review, for example **Undo Move: 3
items back to D:\x**. Details lists every path, including each Recycle Bin
restore. Enter confirms, Escape cancels; nothing is undone automatically.
**Operations** lists recent operations with **Undo** or a short reason
(*Undone*, *Not undoable: ZIP insert*, *Changed since: D:\x\a.txt*), so older
operations can be undone too. If nothing can be undone, a short message names
the latest operation and why.

| Operation | Undo |
|---|---|
| Rename | Renames the same item back if the old name is free. |
| New folder | Recycles the folder if it is still empty and the same folder. |
| Copy | Recycles the items the copy created, if unchanged (in merged folders only the new items, never existing content), then restores replaced files from the Recycle Bin. |
| Move, same drive | Moves each item back without replacing, recreates source folders the Move emptied and removed, then restores replaced files. |
| Move across drives | Restores the originals from the Recycle Bin, then recycles the unchanged copies. A copy whose original was kept (for example on a USB drive) is recycled only while that original is still present and unchanged; otherwise undo is refused and the copy stays. |
| Delete | Restores the items from the Recycle Bin. |
| Create ZIP / extract ZIP | Recycles the created ZIP or the extracted top-level items, if unchanged. |

ZIP insertion, moving members out of a ZIP and source Save are not undoable.
Undo itself cannot be undone (there is no redo), and operations recorded by
older versions are not undoable. Operations checks rows quickly (results still
exist and are the same items); preparing also checks folder contents. Preparing
refuses with the exact path: a changed result, an occupied original path, an
item no longer in the Recycle Bin (emptied or restored), an ambiguous match or
a kept original that is gone (*Original no longer present: E:\Photos; the copy
is kept.*). Results on a drive without a Recycle Bin are never deleted, so their
undo is refused.
For a partly completed or cancelled operation, undo reverses only what
completed; the review says so and Details lists what stays.
A restore never overwrites: the item moves into a new staging folder beside its
original location and is then renamed into place. If the original path became
occupied, the item stays in the Recycle Bin or in staging and Operations shows
where. Execution stops at the first change it finds; an undo that changed
anything is not repeated, even after a restart.

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

## Usage log

Tomas Commander records a local usage log so you can review periodically, for
example with an AI assistant, how you actually use it: which commands you run
and whether by shortcut, mouse or Ask; keys that did nothing (such as F5 or
Delete out of habit); folders you navigate to and how; filter and search
queries; Ask texts that found nothing or were cancelled; dialogs you confirm or
cancel; operations and their results; errors; and slow folders, actions and
frames. Typing is only counted, never logged per key.

**Nothing is sent anywhere.** The log stays in `usage\` beside the
preferences file (`%LOCALAPPDATA%\TomasCommander\usage\`, or beside the file
given with `--settings`) as one `YYYY-MM-DD.jsonl` file per day. It contains
full folder/file paths and Ask texts, but never file contents, edited text,
clipboard contents, AI answers or credentials. Files older than 90 days are
deleted at startup; a day file stops at 20 MB and the folder at 200 MB.
Recording never waits for the disk: a background thread writes every few
seconds and the final write at exit waits at most 300 ms.

**Settings > Usage log** turns recording on or off (on by default), shows the
folder and its size, opens the folder and offers **Delete usage data**, which
after confirmation permanently deletes only the usage log files and exported
reports in that folder. Fixture runs (`--fixture-root`) record nothing unless
they also pass `--settings`.

`/usage` (Ask: *Usage insights*) shows a report for the last 7, 30 or 90 days,
computed in the background: commands with their keyboard/mouse/Ask share and
shortcut opportunities, unhandled keys, unmet Ask texts, most cancelled
dialogs, errors, slow work, most visited folders and Places candidates (folders
repeatedly reached through 3+ navigation steps), tabs/history, search, AI tasks
and session time. **Export Markdown report** writes
`usage\report-YYYY-MM-DD.md` and copies its path. Without opening a window:

```powershell
.\tomas-commander.exe --usage-report --days 30 --out "$env:TEMP\usage-report.md"
```

`--settings <preferences file>` reads another usage folder; without `--out`
the report goes to standard output (redirect it, because release builds have no
console). See [usage log](decisions/usage-log.md) for every recorded field.

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

Markdown/HTML previews switch between **Rendered** and **Source** and offer
**Copy source**. Rendered Markdown retains headings, tables, task lists and
copyable fenced code. Python, Rust, JSON, `.env` (including `.env.local`),
configuration and script previews use local lexical syntax colors in both
themes, selectable read-only text, optional line numbers and wrapping.
**Copy** copies displayed text; select text and Ctrl+C to copy a part.
JSON has a view-only **Autoformat** toggle and **Copy original**.
Invalid JSON stays readable with its line/column error instead of a blank
preview. No Python/config formatter runs: rewriting whitespace can change
semantics. Formatting/copying a preview never saves changes or sends data to AI.
These are quick readers, not a replacement for a full language-aware editor.

Ctrl+E opens the original UTF-8 source, not a JSON-prettified/rendered copy.
Ctrl+S prepares Save for explicit approval. BOM and LF/CRLF are preserved;
mixed/legacy newlines, readonly files, external changes and invalid JSON fail.
Dirty close asks before discarding. Images/PDF and virtual ZIP members are not
editable. Failed writes can retain protected staging files: check Operations.

## ZIP

Enter opens a ZIP as a virtual directory without extraction. Backspace moves
up or exits at its root; a nonempty filter searches relative member names.
One click focuses a row; double-click or Enter opens a directory, or extracts a
CRC-verified temporary file and opens it with its Windows default application.
Ctrl+Enter / Ctrl-double-click previews a member internally without extraction.
ZIP rows use the same font, height, cursor
outline and selection tint as file panes. Checkboxes, Space and Ctrl+click select
items; Shift+arrows selects a range and Ctrl+A selects visible entries.
PageUp/PageDown jump ten rows, like Ctrl+Up/Down. Home/End jumps to either end.
Typing starts a filter; Down leaves the filter for the first match. Escape clears
a nonempty filter before closing the ZIP.

**Extract** (also in a row's context menu) prepares extraction of selected
members, or the current file/folder, to the folder in the other pane.
**Extract all** prepares extraction of the entire ZIP, even when browsing a
subfolder or filtering. Both show the destination in the existing confirmation
before writing; neither removes or overwrites source files.

**Ctrl+Shift+A**, the bottom **Create ZIP / Extract ZIP** box and `/zip` share
one context-aware command. On regular selected files/folders it asks for a ZIP
filename and writes to the other pane's folder. On a single ZIP it prepares
extraction of the entire archive without first opening it. Inside ZIP it
extracts selected/current members, or the entire archive on the parent row.
The existing Extract all button always ignores the ZIP subfolder and filter.
Copy/move can also extract selected members or insert files from the other pane.
Changes review physical/member paths and use staged
publication; move removes sources only after destination verification.
No overwrite/merge, ZIP-to-ZIP transfer or virtual Recycle Bin operation; the
Copy/Move conflict choices do not apply inside ZIP.
Only Store/Deflate; ZIP64, encrypted, split and unsafe/ambiguous archives fail.
Temporary opening/dragging is Copy-only and never changes the source archive;
edits saved by another app affect only its temporary copy. Successful handoffs
stay under `%TEMP%\tomas-commander-export-*` for asynchronous consumers; the
next Commander launch cleans managed copies older than seven days.
Cancelled/failed/abandoned exports are cleaned immediately. Files inherit the
archive's protected access permissions. Duplicate filenames selected from
different ZIP folders are disambiguated with numbered suffixes; selected folder
trees and empty directories are preserved. Temporary exports retain existing
ZIP limits (32 MiB input/selected expanded bytes, 16 MiB per file).

## Settings and integrations

The workspace rail starts collapsed on every launch. Use its `>` / `<` controls
to expand/collapse it, or Ctrl+Shift+H to toggle it. Opening selects Recent.
The Places tab has no dedicated shortcut; saved places remain available through
the separate Alt+Up chooser while the rail is collapsed.
Settings has separate Appearance, Recent activity, AI connections and Usage log
tabs; appearance controls stay immediate, while watched-folder changes use Apply.

Ctrl+Space (or Ctrl+Shift+P, as in other command palettes) opens Ask, with multiline input for AI and commands. Every existing action has a
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

### AI change proposals

Ask with a request to change files, for example *Utřiď Downloads podle typu do
podsložek*, *přejmenuj fotky v D:\Foto\2026 podle data ve formátu
YYYY-MM-DD_nazev*, *přesuň všechny faktury z Downloads do
Documents\Faktury\2026* or *smaž staré instalačky z Downloads*. The read-only
harness looks only at what it needs and proposes changes; it never changes
anything itself. When it is done the bell shows **Proposal ready: N changes**;
click it to open the review (it never opens by itself while you work).

The review shows the assistant's summary and the changes grouped into New
folders, Moves, Copies, Renames and Deletes (Recycle Bin), each as
`before → after`, relative to the common folder when that is shorter. Hover a
row for its reason; Details lists every exact path and reason. Everything
except deletes starts checked; **deletes start unchecked**, are marked DELETE
and must be checked one by one. Unchecking a new folder also unchecks the
changes that need it; they stay disabled with *needs unchecked new folder ...*
until the folder is checked again, which restores them.

Run has the initial focus. Arrows and Tab move between rows and buttons, Space
toggles the focused row, **Enter runs only while Run (Run selected (N)) has
focus**, a held Enter never runs, and Escape cancels. Cancel changes nothing
and the bell reopens the proposal. Run first checks every selected change
against the current files; if anything changed (for example a target now
exists) the problems are listed in the review and nothing starts. Otherwise the
changes run in order, each as a normal operation planned again just before it
runs: no overwrite (an existing target stops the batch), moves across drives
copy, verify and recycle as usual, and deletes go to the Recycle Bin. The batch
stops at the first failing step, **Cancel work** stops it between steps, and
completed steps stay done. Operations lists each step as done, failed (with the
error), cancelled or not run, groups the batch's records, and offers Undo for a
single step or **Undo batch**, a reviewed undo of every step newest first.
If the assistant's proposal itself is invalid (for example an unknown file or a
name that already exists), nothing changes and **Continue** lets it fix the
listed problems. See [AI change proposals](decisions/ai-changes.md).

Preferences: `%LOCALAPPDATA%\TomasCommander\preferences.txt`. v1-v6 remain
readable; v5 added watched Recent folders, v6 each pane's tab folders and
active tab (`tabs=` line) and v7 the usage-log switch (`usage_log=`, on when
missing). Appearance, zoom, favorites/names, tabs, the usage-log switch and
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
Model output never approves or executes filesystem effects: change proposals
run only the steps you select and approve.
See [AI runtime](decisions/ai-runtime.md),
[AI search](decisions/ai-search.md),
[AI change proposals](decisions/ai-changes.md),
[commands](decisions/command-execution.md) and
[connected answers](decisions/connected-answers.md).
