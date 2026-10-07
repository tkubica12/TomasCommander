# Native file-operation policy

Date: 2026-10-07. Initial real Ledger slice; not full FILE-01 acceptance.

## Bound implementation

Windows Rust executable `target\release\tomas-commander.exe`.
Listing, plans, validation, and execution: `src\files.rs`.
Native move/recycle/launch adapters: `src\platform.rs`.
Presentation and bounded background workers: `src\app.rs`.
Synthetic fixture inventory/cleanup: `tests\filesystem.rs`.
Real-window checks: `tests\native-smoke.ps1`.

## Operation contract

All mutations, including copy, require a modal approval showing operation,
actual source paths and destination. Cancellation is the default focus;
Ctrl+Enter is explicit approval. The background UI is disabled while a modal is
open. A plan executes once per in-process operation ID, not on redraw.
Deduplication is not durable across app restarts: inspect actual state after an
unknown outcome rather than blindly retrying.

Sources are resolved and inspected. Filesystem roots, the fixture root, links,
junctions/reparse points, special files, duplicated selections, parent/child
selections, and copying/moving a directory into itself are refused.
The complete recursive source tree is recorded. Metadata and tree membership
are revalidated after approval; changed source/destination state requires a new
plan. Fingerprints are type, length, and modification time, not cryptographic
content hashes or stable Windows file IDs.

| Operation | Actual effects and limitations |
|---|---|
| Copy | Recursively creates new folders/files; file creation is exclusive, never overwrite/merge. Streams content in 64 KiB chunks and checks length. The source remains. Does not preserve full ACLs, alternate data streams, or timestamps. |
| Move | Calls `MoveFileW` without replacement. Existing targets fail. Cross-volume directory moves are unsupported; no copy/delete fallback is attempted. |
| Recycle | Calls Windows `IFileOperation` with recycling, early failure, no connected-item grouping, and a callback that aborts when Windows does not advertise recycling. There is no application permanent-delete API or fallback. Standard local recycling/restoration is verified; unsupported volumes and unusual Windows policies remain unverified. |

Cancellation is checked during planning/enumeration and between copy chunks or
top-level move/recycle items. A single blocking OS request is not interruptible.
Execution stops on the first error. Completed work is not silently rolled back.
Created and completed paths, cancellation, and errors are explicit; created
files can be partial after failure/cancellation. Only the latest operation's exact
path list is retained in the current UI, not a durable operation journal.

Guarding paths is not a complete handle-relative defense against hostile
concurrent filesystem replacement. Files changed without detectable metadata
changes can invalidate the assumed fingerprint. This first slice is not accepted
for adversarial-race guarantees; stronger identity/handle-based protection is
required before making such claims.

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

Still required for complete FILE-01 acceptance: OS/process interruption and ACL
permission-failure coverage, full keyboard/mouse matrix on physical CZ/US layouts,
recursive deterministic search, stronger stale identity/race tests, large-corpus
responsiveness measurements, and agreed performance thresholds. Do not report
the entire card as PASS.

## Platform evidence

- [MoveFileW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefilew):
  non-overwriting move and cross-volume restrictions; Microsoft Learn, inspected
  during native implementation.
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
