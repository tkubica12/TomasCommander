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
effects explicitly. There is no automatic rollback/recovery/replay.
Journal limits: 10,000 records, 16 MiB per record, 64 MiB total.
Archive only reviewed/reconciled records deliberately.

## Effects

| Operation | Contract |
|---|---|
| Copy | New files/folders are created exclusively. Existing destinations follow the reviewed conflict policy (below); nothing is overwritten in place. Bounded streaming, length checks and flush. Sources remain. Preserves each copied file's modification time; does not preserve creation time, full ACLs or alternate streams. |
| Move | Identity-verified handle rename with pinned ancestors and replacement disabled. Merged folders move their children individually; emptied source folders are then removed only if empty. No cross-volume copy/delete fallback. |
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
Recycle Bin and publishes the staged file with a no-replace rename. Move checks
the same volume and the destination fingerprint, recycles the old file and then
performs the verified handle rename. Any failure stops the operation and leaves
the staging file and the recycled original in place for inspection.

The plan's identity includes the fingerprint of every existing destination it
merges into, replaces or skips. Changed, removed or newly appearing conflicts
fail validation with "Destination changed; create a new plan". The journal
records the policy, each node's resolution (create/merge/replace/skip/rename),
conflict fingerprints and staging paths; receipts list replaced, merged and
skipped paths. AI commands and other non-interactive callers keep the legacy
fail-on-conflict policy; there is no overwrite through AI.

Copy pins ancestors/source identity and denies source writes/deletion during
execution. Move rechecks the opened identity/type/size/times. These are not
cryptographic fingerprints or a full hostile-filesystem guarantee; concurrent
directory membership and undetectable same-metadata changes remain limitations.
The intended scope is ordinary desktop use, not adversarial replacement races.

Cancellation is checked during traversal, validation and copy chunks/top-level
effects. A blocking OS call is not interruptible. Execution stops on error and
does not silently roll back completed work. Partial/staging paths and errors are
reported in Operations and retained in receipts. Inspect actual state before retry.

## Safe testing

`--fixture-root` canonicalizes an owned boundary. Mutating tests use uniquely
named synthetic paths and explicit ownership inventories; unknown additions must
prevent cleanup rather than being deleted. `--settings` is separately selected.
External associations/launchers are not sandboxed by a fixture boundary.

Real Recycle Bin recovery is opt-in and must identify the exact owned fixture.
Never empty the Recycle Bin or touch unrelated items. Automated conflict tests
cover planning, validation and non-replacing execution; the replacement test that
recycles and restores its own synthetic originals is ignored by default.
