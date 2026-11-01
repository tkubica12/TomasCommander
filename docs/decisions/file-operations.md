# File-operation policy

Filesystem mutations, including Copy, review an immutable exact-path plan.
The confirmation action has initial keyboard focus: Enter confirms and Escape
cancels. Tab can select Cancel; Enter then activates Cancel, not the default.
Held/repeated Enter never approves a newly displayed plan. Model output, redraws
and restarting the app never grant approval.

New folder is the exception to a separate review dialog: submitting its name
with Create or Enter is the approval. The worker still prepares, validates and
journals the exact target before creating the empty folder. Name collisions,
invalid names, cancellation and changed parents still prevent creation.
Rename continues to show a separate review.

Confirmations show the operation, exact destination/source names and the main
effect. Technical snapshot, staging and ZIP member information is under Details;
it is not discarded or omitted from execution validation.

## Snapshots and journal

Sources/targets are resolved and inspected. Roots, links/junctions/reparse
points, special files, duplicate/parent-child selections and directory-into-self
operations fail. Recursive membership and metadata/Windows file identities are
rechecked after approval. Changed sources/destinations require a new plan.

Before execution the worker exclusively claims and flushes the plan's unique
key, paths and fingerprints to `preferences.operations` beside settings.
The journal is initialized before planning. Clones/restarts/app instances cannot
replay a claimed plan, even if an output later disappears.

Results and completion markers are durable. Missing completion means unknown
outcome, not permission to retry. Overlapping new plans fail until actual state
is reconciled. Malformed/reparse/oversized records and journal failures stop
effects explicitly. Nothing is rolled back, recovered or replayed automatically;
undo is a separate, explicitly approved plan (see Undo below).
Journal limits: 10,000 records, 16 MiB per record, 64 MiB total.
Archive only reviewed/reconciled records deliberately.

## Effects

| Operation | Contract |
|---|---|
| Copy | New files/folders are created exclusively. Existing destinations follow the reviewed conflict policy (below); nothing is overwritten in place. Bounded streaming, length checks and flush. Sources remain. Preserves each copied file's modification time; does not preserve creation time, full ACLs or alternate streams. |
| Move | Same volume (equal volume serial and volume root of the real, resolved paths, so mapped/subst drives compare by their actual volume): identity-verified handle rename with pinned ancestors and replacement disabled; merged folders move their children individually and emptied source folders are then removed only if empty. Different volumes: copy, verify, recycle (below). No permanent-delete fallback. |
| Recycle | Windows IFileOperation recycling with an abort when recycling is not advertised. No application permanent-delete fallback. |
| Rename / New folder | Same-folder exact plan; validated Windows names; no collisions, case-only rename, merge or replay. New folder is empty. |
| Source Save | Frozen original/UTF-8 bytes; BOM/LF/CRLF preservation; invalid JSON, readonly/mixed encoding and stale content fail. Protected staging, byte verification and ReplaceFileW replace only the approved original. |
| ZIP effects | CRC-checked bounded members, exact member/physical plans and staged publication/rewrite. Source removal follows destination verification. No overwrite/merge, ZIP-to-ZIP or virtual recycling; conflict policies do not apply to ZIP flows. |

## Copy/Move conflicts

Planning detects existing destination items recursively and records each
resolution in the plan. The review dialog chooses one policy for the whole plan;
changing it prepares a new plan (new id) in the background and keeps
confirmation disabled until that plan arrives. An approved plan is never edited.

| Existing item | Skip | Replace older | Replace all | Keep both |
|---|---|---|---|---|
| Folder for a source folder | Merge | Merge | Merge | Merge |
| File for a source file | Skip | Replace only if the source is strictly newer, otherwise skip | Replace | Copy/move as `name (2).ext`, `name (3).ext`, ... |
| File/folder type differs | Skip | Planning fails | Planning fails | New unique name |
| Link, junction or special item | Planning fails | Planning fails | Planning fails | Planning fails |

Keep both names are unique case-insensitively, on disk and against every other
planned target. A source can never replace or merge into itself: the same item
is skipped (Skip), given a new name (Keep both) or refused (Replace). Replacement
never truncates or overwrites data in place: Copy streams into a uniquely named
`.tomas-commander-*.stage` file beside the target, flushes and verifies its
length, rechecks the planned destination fingerprint, sends the old file to the
Recycle Bin and publishes the staged file with a no-replace rename. A same-volume
Move checks the destination fingerprint, recycles the old file and then
performs the verified handle rename; a cross-volume Move replaces exactly like
Copy. Any failure stops the operation and leaves the staging file and the
recycled original in place for inspection.

The plan's identity includes the fingerprint of every existing destination it
merges into, replaces or skips. Changed, removed or newly appearing conflicts
fail validation with "Destination changed; create a new plan". The journal
records the policy, each node's resolution (create/merge/replace/skip/rename),
conflict fingerprints and staging paths; receipts list replaced, merged and
skipped paths. AI commands and other non-interactive callers keep the legacy
fail-on-conflict policy; there is no overwrite through AI. AI change proposals
plan every approved step with that policy too.

## Cross-volume Move

Planning records the Move strategy in the plan and journal snapshot
(`strategy=same-volume-rename`, `cross-volume-copy-verify-recycle` or
`cross-volume-copy-verify-keep`); validation fails if it changes. Conflict
policies apply exactly as for Copy. For each selected top-level item:

1. Copy every file through the Copy path (exclusive creation or staging,
   length check, flush, modification time) while computing its CRC-32; close it,
   re-read the destination and compare. A mismatch stops the Move, keeps the
   source and reports the bad destination.
2. After the whole item is copied and verified, recheck every source
   fingerprint and folder membership against the plan. A change stops before
   recycling; the original and its verified copy both remain and are reported.
3. Without skipped descendants, send the item to the Recycle Bin as one entry.
   Otherwise send only the moved units in one Windows IFileOperation batch, then
   remove emptied merged source folders with `remove_dir` (which fails on
   non-empty folders). Skipped items stay untouched.

Recycling reuses the recycle-only guard that aborts when Windows does not
advertise recycling (no bin, item too large, bin disabled). There is no
permanent-delete fallback. Planning probes the source drive: only fixed local
drives have a Recycle Bin, so removable, network, optical and RAM drives (and
an unanswered probe) plan `cross-volume-copy-verify-keep` and the review states
that originals are kept. If recycling fails at execution, the operation stops,
reads back each original and reports exactly which were recycled and which were
kept. Cancellation never recycles an original whose item was not fully copied,
verified and rechecked. Receipts add `recycled=` and `kept=` paths. AI command
moves refuse different volumes at planning.

Verification re-reads through the Windows file cache; it detects copy-path
corruption, not later media failure. Like Copy, a cross-volume Move keeps
modification times but not creation times, ACLs or alternate streams. Source
deletion is not atomic with the copy; the recheck (and a final check that every
copy is still present) narrows, but does not eliminate, the window for
concurrent changes.

Copy pins ancestors/source identity and denies source writes/deletion during
execution. Move rechecks the opened identity/type/size/times. These are not
cryptographic fingerprints or a full hostile-filesystem guarantee; concurrent
directory membership and undetectable same-metadata changes remain limitations.
The intended scope is ordinary desktop use, not adversarial replacement races.

Cancellation is checked during traversal, validation and copy chunks/top-level
effects. A blocking OS call is not interruptible. Execution stops on error and
does not silently roll back completed work. Partial/staging paths and errors are
reported in Operations and retained in receipts. Inspect actual state before retry.

## Undo

Undo never runs automatically. Ctrl+Shift+U, `/undo` or an Operations row
prepares an inverse plan from a finished journal record, validates it against
the current state and shows it in the standard review (Enter confirms, Escape
cancels, the held-Enter and focus protections apply). The approved plan is
claimed and journaled like any plan, with `undo_of=<original key>`.

**Recorded data.** Claiming a plan appends `started=<ns since 1970>` to its
snapshot. The outcome adds `undo_format=2`, `kind=` (copy, move, move-across,
move-across-kept, delete, rename, new-folder, zip-create, zip-extract,
zip-extract-move, zip-insert, zip-insert-move, save, undo) and `ended=`, then
what the outcome proves:

| Line | Meaning | Inverse |
|---|---|---|
| `undo_created=<path> <mark>` | Complete item the operation created (top-level item, a direct child of a merged folder, Keep both name, created ZIP or top-level extraction output) | Recycle if the mark still matches |
| `undo_kept=<original> <copy> <mark>` | Cross-volume Move original that was not recycled (no Recycle Bin, stopped or cancelled Move), with the copy made from it | Recycle the copy only while the original still matches its mark |
| `undo_moved=<original> <result> <mark>` | Same-volume Move unit or Rename | Move back without replacement if the result is the same item |
| `undo_recycled=<path>` | Deleted item or cross-volume original in the Recycle Bin | Restore |
| `undo_replaced=<path>` | Destination file replaced into the Recycle Bin | Restore after the replacing item is gone |
| `undo_removed=<path>` | Emptied merged source folder removed by a Move | Recreate empty folder |
| `undo_partial=<path>` | Created but incomplete output | Never undone; listed in the review |

A file mark is its Windows file identity, size and modification time. A folder
mark is its identity plus a hash of every descendant's name, type, identity,
size and modification time (folder times are excluded, so an added and removed
temporary file is not a change). Moved items are checked by identity only:
moving the same item back is lossless. Records without `undo_format` come from
an older version and are not undoable. Format 1 (no `undo_kept`) stays readable,
but a cross-volume Move record with kept originals and no `undo_kept` lines, or
a stopped format-1 cross-volume Move, is not undoable: its copies may be the
only remaining data. If undo data would push a record past the
16 MiB limit, `undo_unavailable=record size` is written instead.

**Order.** Recreate emptied folders (outermost first), restore recycled
originals, move items back, recycle created items, restore replaced files. A
cross-volume Move therefore returns its originals before recycling the verified
copies; a stopped undo never leaves an item without either copy.

**Validation.** Preparing refuses the whole undo with the exact path when a
result changed or vanished (`Changed since: <path>`), an original path is
occupied, a parent folder is missing, a Recycle Bin item is missing or ambiguous,
a kept cross-volume original is gone or changed (`Original no longer present:
<path>; the copy is kept.`), or a result lies on a drive without a Recycle Bin
(undo never deletes permanently). Execution rechecks each step immediately
before its effect, including the kept original just before its copy is
recycled, and stops at the first problem; cancellation is checked between
steps. Partly completed, cancelled or failed operations undo only what their
outcome proves.

**Operations rows.** Rows are read on the file worker with cheap checks only:
the record, its undo state, and the existence and top-level identity of results
and kept originals. Folder contents and Recycle Bin items are verified only when
preparing, so a row may offer Undo that preparing then refuses with the exact
path. Reads honor the shared cancellation and coalesce: a new read is queued
only when none is pending.

**Recycle Bin restore.** The Shell enumerates `FOLDERID_RecycleBinFolder`
(`IEnumShellItems`, `IShellItem2`). An item's original path is
`System.Recycle.DeletedFrom` joined with `System.FileName` (which keeps the
extension even when Explorer hides it); its deletion time is
`System.Recycle.DateDeleted`. A recorded path matches only an exact
case-insensitive ordinal path comparison with a deletion time between the
claim time minus 2 s and the completion time plus 2 s. No match refuses as
"Not in Recycle Bin (emptied or restored)"; several matches refuse as ambiguous.
The review binds the exact stored item, which is re-identified and rechecked
before the restore. IFileOperation moves it into a new, uniquely named
`.tomas-commander-restore-<key>-<n>` folder created beside the original, so no
existing item can collide and `FOF_NOCONFIRMATION` can never answer an
overwrite prompt. `FOF_NOERRORUI`, `FOF_SILENT` and `FOF_NOCONFIRMMKDIR`
suppress Windows UI, and a progress sink aborts every action except moving
that item into that folder. The staged item is verified (type, file size) and
published with the no-replace handle rename to the exact original path; the
empty staging folder is then removed. An original path occupied at execution
keeps the item in the Recycle Bin, or in staging if it became occupied after
the move; Operations reports staging leftovers.

**Not undoable.** ZIP insertion and rewrite, moving members out of a ZIP, source
Save, interrupted records (reconcile them first) and undo itself: there is no
redo. Under the journal lock a second undo of the same operation is refused
once an earlier undo changed anything or was interrupted; an undo refused
before any effect does not block a new reviewed plan. Operations shows
*Undone*, *Partly undone* or the reason per record, also after a restart.

**Batches.** Steps of an approved [AI change proposal](ai-changes.md) are
ordinary records that also carry `batch=<key>` in their snapshot. Operations
groups them and offers **Undo batch**: a reviewed plan listing every undoable
step newest first (already undone steps and steps without effects are noted
and skipped; a step that cannot be undone refuses the batch). Before approval
only the cheap row checks run; after approval each step's inverse is prepared
with full validation just before it runs, so a created folder is checked once
the items moved into it are back. Each inverse is journaled with `undo_of`;
the batch undo stops at the first refusal, failure or cancellation and reports
the steps not undone.

Limitations: changes that keep the same identity, size and modification time
are not detected; recreated folders get new identities and times; a moved
folder is moved back with any later changes inside it; restoring rechecks the
Recycle Bin per item, so very large bins make restores slower.

## Safe testing

`--fixture-root` canonicalizes an owned boundary. Mutating tests use uniquely
named synthetic paths and explicit ownership inventories; unknown additions must
prevent cleanup rather than being deleted. `--settings` is separately selected.
External associations/launchers are not sandboxed by a fixture boundary.

Real Recycle Bin recovery is opt-in and must identify the exact owned fixture.
Never empty the Recycle Bin or touch unrelated items. Automated conflict tests
cover planning, validation and non-replacing execution; the replacement test that
recycles and restores its own synthetic originals is ignored by default.
Cross-volume tests force the strategy on one volume and inject a recycler that
moves items into a fixture folder, plus a post-copy hook for corruption,
source-change and cancellation cases (`#[doc(hidden)]` seams). The real
Recycle Bin and real second-volume (`TC_SECOND_VOLUME_ROOT`) tests are ignored by
default. Undo tests inject a fake Recycle Bin (`undo::RecycleBin`) that records
deletion times and restores from its fixture folder; the real restore test
deletes, restores and recovers only its own uniquely named items and is ignored
by default.
