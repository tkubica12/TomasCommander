# Usage and limits

## File work

The `..` row is navigation only, never an operation target. Filter is local;
recursive search examines actual filenames without following links/junctions.
Selected rows operate on their selected group; an unselected row targets itself.
Rename always targets the focused item.

Drag a row or selected group to the opposite pane/folder. Move is the default;
hold Ctrl at release for Copy. Drops prepare a plan, not an immediate effect.
Escape cancels. External shell dragging and ZIP-to-ZIP transfers are unsupported.

Saved places are available in the left rail and Alt+Up popup. Edit changes the
display name; Delete removes only its bookmark. Recent documents records files
opened through this app, not system-wide activity.

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
Create ZIP is in the palette. Changes review physical/member paths and use staged
publication; move removes sources only after destination verification.
No overwrite/merge, ZIP-to-ZIP transfer or virtual Recycle Bin operation.
Only Store/Deflate; ZIP64, encrypted, split and unsafe/ambiguous archives fail.

## Settings and integrations

Preferences: `%LOCALAPPDATA%\TomasCommander\preferences.txt`. v1-v3 remain
readable; custom saved-place names use v4. Appearance, zoom, favorites/names and
recent documents persist. Operations use a durable journal beside preferences.

Shift+Enter opens VS Code/Copilot choices for the active context. These require
installed tools. Copilot App requests an idle folder session; CLI uses `--yolo`
and therefore grants the external agent its unattended execution mode.

AI remains a restricted experiment. Settings exposes Foundry connection under
its advanced panel. The app neither installs Azure CLI nor signs you in.
Only explicitly confirmed prompts are sent, not enumerated local files.
Model output never approves or executes filesystem effects.
See [AI runtime](decisions/ai-runtime.md),
[AI search](decisions/ai-search.md),
[commands](decisions/command-execution.md) and
[connected answers](decisions/connected-answers.md).
