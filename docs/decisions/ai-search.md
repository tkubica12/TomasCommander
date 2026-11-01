# Read-only AI file discovery

AI search uses the configured Foundry primary deployment in a small native Rust
Responses function-calling loop. `/ai-search prompt` or Enter on ordinary text
in Ask (Ctrl+Space) starts it. Slash actions remain deterministic; clicking
a command or explicitly selecting it with arrows runs that command.
The active folder is context, not a scope boundary.

Access is separately persisted in
`%LOCALAPPDATA%\TomasCommander\ai-file-access.json`, bound to the approved
Foundry connection. No roots are shipped, inferred from future favorites or
silently granted. The explicit initial setup command
`--file-agent-configure-favorites` accepts only the owner's three approved
locations (D: and the personal/organization OneDrive folders), refuses existing
authority, and makes no provider request. Configuration changes during a run
stop further access. File handles and ancestor checks reject junctions, symlinks,
device paths, traversal, streams and unknown reparse tags; known Cloud Files
placeholders may hydrate when download permission is enabled.

Tools: `list_directory`, case/diacritic-insensitive `glob`, literal `grep`,
and `read_file` with character offsets, plus the final-step `propose_changes`
for organize/rename/move/copy/delete requests, which only returns a validated
proposal for the user's review ([AI change proposals](ai-changes.md)). A null
traversal path searches all approved roots in one call. Globs support one brace group such as `*.{pptx,pdf,html,htm}`
and an optional literal full-path/name query. Filename/type/path filtering happens
before unrelated file metadata is opened; directory containment is still checked
before traversal. The model ranks these candidates before reading their content.
Supported content is UTF-8/BOM-marked UTF-16 text, DOCX main-body text, PPTX
slide text, XLSX/XLSM cell values and PDF text. External Office relationships,
macros, VBA projects and embedded objects are not followed. Spreadsheets list
each sheet as `[sheet name]` (hidden sheets marked) followed by tab-separated
rows aligned to their columns; empty rows are skipped, formulas contribute only
their cached value, numbers/dates stay raw serial values, and chart/macro sheets
are not read. PDFs are parsed with `lopdf` in a panic-contained, time-bounded
helper thread of the contained worker: `[page N]` text in content order, with
ToUnicode, font encodings, form XObjects and word/line spacing reconstructed
from positions; JavaScript, forms, annotations and attachments are ignored.
Owner-password-only PDFs are readable; PDFs needing a user password,
password-protected Office files and OLE files mislabeled as Open XML fail
explicitly. Pages without a usable text layer (images or undecodable fonts)
are rendered with Windows.Data.Pdf and recognized by Windows.Media.Ocr as
`[page N, OCR]`, for at most 10 pages, 2 seconds per page and 20 seconds per
document; `grep` never runs OCR and marks such pages instead. Missing OCR
languages are reported in the text. Limits: 500 pages, 20-second text-layer
budget, 16 MiB per decompressed stream, 32 MiB per page content and extracted
text. The text budget is enforced while decoding each string and spreadsheet
cell, not after a page or row: ToUnicode CMaps are parsed by a bounded in-house
parser that never expands code ranges (at most 16 UTF-16 units per code,
131,072 entries per CMap, 1,048,576 per document, each shared stream parsed
once; lopdf never parses ToUnicode), at most 4,096 fonts are decoded per
document, and spreadsheet shared strings are borrowed (at most 500,000) with
cells beyond column XFD skipped. Extracted PDF/Office/spreadsheet text is cached per file
identity/size/time for one harness run (64 MiB total). Images and legacy Office
content are explicitly unsupported; images can still be found by names.
Reads are limited to 32 MiB files and 6,000 characters per page. Full supported
documents may be read over successive pages bound to the returned file version.
Content and metadata share held file/ancestor handles; changed observations get
new IDs, and stale results fail activation instead of opening a replacement.
Modification dates before 1970 are explicitly unsupported.
Traversals report incomplete
results after 100,000 examined items, 100 returned items, 15 seconds, or bounded
output capacity. Permission/read errors and skipped protected paths are explicit.
Known credential paths and obvious credential markers are excluded, not a
guarantee that arbitrary documents contain no sensitive information.

Each explicitly submitted segment permits at most ten model calls and two
minutes including waiting. Capped runs and rejected change proposals keep
partial results and offer Continue;
Ask hides on submission. One task runs in the background; the header bell shows
progress and unread completion without interrupting file work. Hide keeps the
task running; explicit Cancel/Escape and app shutdown cancel it.
Continuation keeps transcript and cumulative usage in memory. A full context
requires a narrower new search. Escape cancels a busy run; a further Escape
closes the task window. Closing the application cancels too. Results support arrows, Enter to show in
the source pane and Ctrl+Enter to preview. Activation rechecks file identity.

The owner approved content transmission and automatic OneDrive hydration inside
the configured roots, with no monetary/lifetime request cap. A separate durable
attempt counter and content-free usage log retain actual provider usage without
resetting old pilot history. The shared spending lease serializes provider work;
uncertain submissions block automatic retries. Technical context/output bounds
remain. No shell, MCP connectors, model fallback or new provider, and the model
never changes files: a change request ends in a reviewed proposal whose
selected steps run only after explicit approval, as ordinary journaled
operations.
The standard Responses loop explicitly sends `reasoning.effort=medium`, carries
encrypted reasoning items and returns local results as `function_call_output`.
There is no additional planner model or agent self-orchestration.

Ctrl+Shift+F and `/search` remain deterministic local filename search. The old
one-shot intent pilot remains for compatibility and injected diagnostic tests;
it is not the default AI-search route. See [runtime boundaries](ai-runtime.md).
