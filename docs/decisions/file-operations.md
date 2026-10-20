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
| Copy | Exclusive new files/folders; no overwrite/merge. Bounded streaming, length checks and flush. Sources remain. Does not preserve full ACLs, alternate streams or timestamps. |
| Move | Identity-verified handle rename with pinned ancestors and replacement disabled. No cross-volume copy/delete fallback. |
| Recycle | Windows IFileOperation recycling with an abort when recycling is not advertised. No application permanent-delete fallback. |
| Rename / New folder | Same-folder exact plan; validated Windows names; no collisions, case-only rename, merge or replay. New folder is empty. |
| Source Save | Frozen original/UTF-8 bytes; BOM/LF/CRLF preservation; invalid JSON, readonly/mixed encoding and stale content fail. Protected staging, byte verification and ReplaceFileW replace only the approved original. |
| ZIP effects | CRC-checked bounded members, exact member/physical plans and staged publication/rewrite. Source removal follows destination verification. No ZIP-to-ZIP or virtual recycling. |

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
Never empty the Recycle Bin or touch unrelated items.
