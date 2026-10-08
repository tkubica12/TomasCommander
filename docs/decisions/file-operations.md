# Native file-operation policy

Date: 2026-10-08. Native Ledger with bounded durable recovery; full acceptance
still requires the physical-input and agreed performance gates.

## Bound implementation

Windows Rust executable `target\release\tomas-commander.exe`.
Listing, plans, validation, and execution: `src\files.rs`.
Durable operation claims/outcomes and interrupted-retry refusal: `src\journal.rs`.
Native move/recycle/launch adapters: `src\platform.rs`.
Presentation and bounded background workers: `src\app.rs`.
Synthetic fixture inventory/cleanup: `tests\filesystem.rs`.
Real-window checks: `tests\native-smoke.ps1`.

## Operation contract

All mutations, including copy, require a modal approval showing operation,
actual source paths and destination. Cancellation is the default focus;
Ctrl+Enter is explicit approval. The background UI is disabled while a modal is
open. Plan fields are immutable outside the filesystem service. Each plan has a
unique execution key retained by its clones, independent of the UI counter.
Before mutation, the worker exclusively claims and flushes that plan's exact
paths and fingerprints to `preferences.operations` beside the chosen settings.
The journal directory is initialized before planning, so first-use storage
creation cannot invalidate an approved destination folder's metadata snapshot.
This does not bypass destination stale checks: subsequent unrelated changes
still require a new plan. Initialization/access failures are explicit.
Replay of the same approved plan is refused across restarts and app instances,
even if its output was subsequently removed. No redraw or restart resumes it.

Completed, created, incomplete, cancelled and error outcomes are flushed before
a completion marker is written. A record without that marker is an unknown
outcome: startup explicitly reports it; any new plan overlapping its recorded
sources/targets fails closed, including copying the source to a different
destination. Read back actual state before deciding on a fresh approval.
After manual reconciliation, archive the exact unresolved record outside the
journal; never erase evidence or treat absence of a result as permission to
retry. Known failures/cancellations retain their exact durable outcomes and
may receive a separately approved new plan; the old plan is still nonreplayable.
The journal uses lossless hexadecimal Windows path encoding, not shell strings.
Malformed/reparse/oversized records and journal access/write failures stop
mutation explicitly. Record scanning is capped at 10,000 records, 16 MiB per
record and 64 MiB total; archive reviewed records before exceeding those limits.
There is no automatic rollback, journal replay or permanent-delete recovery.

Sources are resolved and inspected. Filesystem roots, the fixture root, links,
junctions/reparse points, special files, duplicated selections, parent/child
selections, and copying/moving a directory into itself are refused.
The complete recursive source tree is recorded. Metadata and tree membership
are revalidated after approval; changed source/destination state requires a new
plan. Fingerprints include type, length, modification/creation times and Windows
volume/file identity; destination directory identity is retained too. Copy holds
ancestor/source identity handles without delete sharing throughout execution,
including newly created target folders, and its open source content handle
denies concurrent writes/deletion. Move pins ancestors and opens the approved
source with DELETE access, verifies source/destination IDs and rechecks its
type, length and modification/creation times against the approved snapshot,
then renames that handle without replacement. It does not rename a newly
substituted or observably changed source.
These are not cryptographic content fingerprints or a full hostile-filesystem
guarantee. Directory membership can still change concurrently; recycling uses
the Windows Shell rather than an application handle-relative deletion primitive.

| Operation | Actual effects and limitations |
|---|---|
| Copy | Recursively creates new folders/files; file creation is exclusive, never overwrite/merge. Streams content in 64 KiB chunks, checks length and calls `sync_all`. The source remains. Does not preserve full ACLs, alternate data streams, or timestamps. |
| Move | Uses `SetFileInformationByHandle(FileRenameInfo)` on the identity-verified source with replacement disabled and pinned ancestors. Existing targets fail. Cross-volume moves are unsupported; no copy/delete fallback is attempted. |
| Recycle | Calls Windows `IFileOperation` with recycling, early failure, no connected-item grouping, and a callback that aborts when Windows does not advertise recycling. There is no application permanent-delete API or fallback. Standard local recycling/restoration is verified; unsupported volumes and unusual Windows policies remain unverified. |

Cancellation is checked during planning/enumeration/validation and between copy chunks or
top-level move/recycle items. A single blocking OS request is not interruptible.
Execution stops on the first error. Completed work is not silently rolled back.
Created, completed and incomplete paths, cancellation, and errors are explicit; created
files can be partial after failure/cancellation. Only the latest operation's exact
path list is retained in the current UI; durable records retain all operations.

Guarding paths is not a complete handle-relative defense against hostile
concurrent filesystem replacement. Files changed without detectable metadata
changes can invalidate the assumed fingerprint. The owner explicitly accepted normal desktop-use safety for the initial
FILE-01 release on 2026-10-08, excluding hostile processes continuously replacing
files/directories. Stale plan/identity checks, containment/reparse refusal,
exact approval, durable retries and interruption recovery remain mandatory.
This scope does not claim adversarial-race guarantees; stronger protection is
required before making full hostile-filesystem claims. Actual concurrent source
content writes and source/destination ancestor renames are now refused during
copying; same-name source/destination substitutions are refused by the move
adapter's verified handles. A same-identity content change between validation
and opening the move handle is also refused and covered by exact hash read-back.

## Scope and test safety

`--fixture-root` canonicalizes and enforces a dedicated boundary. Testing
mutations is confined to unique synthetic folders. The tests inventory owned
paths and remove those exact files/directories without recursive deletion;
unowned additions prevent directory cleanup rather than being deleted.

The opt-in real Recycle Bin test recovers its uniquely named fixture using its
exact original location. It never empties the bin or touches unrelated items.
It is opt-in because recovery depends on the current Windows Shell.

`--settings` is outside the filesystem-operation boundary and must be chosen
explicitly for fixture runs. External file associations and VS Code launch are
not sandboxed by the fixture boundary.

## Checks and remaining gaps

Cargo tests cover real recursive copy/content preservation, move/non-overwrite,
denied approval, cancellation before execution, changed source/tree/destination,
root/parent-child guards, an actual junction to an outside synthetic sentinel,
preferences, and 1,001-entry listing. Additional tests exercise multi-source copy,
an exclusively locked real source that produces an explicit error, and actual
mid-copy cancellation with preserved source and retained partial destination.
The real-window smoke test verifies an approved copy through the actual UI,
conflicts, and preference persistence even when closing immediately after a change.
Same-folder refresh retains valid selection/focus rather than clearing them.

Additional bounded tests now cover actual process termination during copying,
ACL-denied creation, same-bytes/time source replacement, exact partial outcomes,
and recursive real-filename search with stale activation and junction exclusion.
The SHA256 inventory matrix covers recursive multi-item copy/nonoverwriting
move, approval refusal, collisions, ACL denial, retained partial multi-item
failure, cancellation/interruption and exact Recycle Bin restoration.
Interrupted-process tests now also reopen the real durable journal and refuse
an overlapping fresh plan even when its alternate destination is empty.
Warm release listing/filter/search and operation baselines were collected, not
accepted against invented limits.

Still required for complete FILE-01 acceptance: full keyboard/mouse matrix on
physical CZ/US layouts, final expanded-UI acceptance and complete performance
bindings. Hostile-process race guarantees are explicitly outside the
owner-approved initial-release scope, not implemented or passed.
Owner-approved warm service ceilings for the frozen 10,020-file local corpus:
listing <=3500 ms, filtering <=3 ms and recursive search <=6000 ms.
These are service-time pilot limits, not cold-start/resource/input-latency
acceptance. Do not report the entire card as PASS.

## Navigation and discovery

The pinned `..` row is navigation only, excluded from filtering, sorting,
selection, counts and every mutation target. The fixture boundary and filesystem
roots show unavailable parent navigation rather than escaping their scope.
Paths hide only supported Windows extended drive/UNC prefixes for presentation;
canonical internal paths and identities remain unchanged.

Recursive search uses the active folder and descendants without following
links/junctions. A bounded worker examines at most 100,000 entries and returns
at most 10,000 results and 100 errors, with explicit truncation/cancellation.
Result activation is revalidated off the UI thread. Filtering remains local.

Folder metadata is computed on generation-tagged pane workers, never during
paint. Each immediate folder traversal is capped at 100,000 entries and 100
warnings; links are skipped. Size sums descendant files and the displayed date
is the newest descendant-file modification time (UTC). Pending, partial, changed
and failed metadata remain explicit rather than silently displaying zero.
Re-sorting preserves focused/selected paths as aggregates arrive.

## Platform evidence

- [MoveFileW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefilew):
  earlier path-based adapter evidence; superseded by the identity-bound move.
- [SetFileInformationByHandle](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle)
  and [FILE_RENAME_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info):
  source-handle rename and explicit nonreplacement; official pages retrieved
  2026-10-08. Installed windows 0.62.2 binding layout/signature was inspected.
  A relative RootDirectory/name attempt returned invalid parameter on this
  machine; the verified adapter uses an absolute name with pinned ancestors.
- [IFileOperation flags](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags):
  recycling, early-failure and connected-item policy; Microsoft Learn fetch,
  successfully retrieved 2026-10-07.
- [PreDeleteItem](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperationprogresssink-predeleteitem):
  callback failure cancels deletion; Microsoft Learn fetch, successfully retrieved
  2026-10-07.
- [Transfer flags](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/ne-shobjidl_core-_transfer_source_flags):
  recycling indication is "if possible", not a proof for every volume/policy;
  Microsoft Learn fetch, successfully retrieved 2026-10-07.

Microsoft Learn code-sample lookup for UI Automation was attempted and blocked by
a transport initialization failure. The smoke test uses the installed Windows
.NET UI Automation assemblies; no testing package was added.
