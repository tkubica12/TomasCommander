# Goal Card: Safe two-pane file management

## OBJECTIVE

Card: FILE-01 | Version: 2 | Readiness: Draft - complete task and performance bindings pending
Mode: bounded repair loop | Owner: project owner

Deliver real two-pane local file management: navigation, single/range/multiple
selection, filtering, deterministic search, favorites, sorting, copy, move,
and delete. Keep every operation keyboard-first and mouse-accessible.
Exclude AI discovery and network/cloud filesystem providers.

## OUTPUT

Implement filesystem services behind the shared commands at paths bound after
APP-01. Add synthetic fixture creation, inventory, tests, and targeted cleanup.
Document operation/conflict/approval policies in
`docs\decisions\file-operations.md`. Record evidence under the shared contract.

## DONE WHEN

Mandatory C04-PERF from `README.md` applies: real filesystem operations, discovery,
and safety guards are Rust-first; benchmark large-folder navigation, filtering,
and operations while keeping input responsive.

| ID | Pass | Verify | Evidence | Failure response | Recheck |
|---|---|---|---|---|---|
| C01 | Both panes correctly browse, select, filter, search, sort, and persist/reload favorites for the agreed fixture inventory; all tasks work with keyboard and mouse. | Run bound service tests and the real Windows task matrix, including empty folders, Czech names, spaces, and inaccessible paths. | Task matrix, favorite read-back, observed errors and test results. | Repair state/service integration. | C01 and affected C02. |
| C02 | Copy preserves source and content; move preserves content at destination and removes only the approved source; delete follows the approved recovery policy. Conflicts, cancellation, partial failure, and stale plans are explicit. | Compare fixture inventories and content hashes before/after each operation; exercise collision, permission, interrupted operation, and cancellation cases. | Plans, approvals, per-item results, before/after inventory and hashes. | Repair scoped operation logic; unknown outcome stops for read-back. | C02 and C03. |
| C03 | Move/delete/overwrite cannot occur without approval of exact targets. Tests never mutate pre-existing files or escape the test root. Cleanup removes only owned fixtures and reports failure. | Test refusal, changed paths, traversal/reparse escape, unowned sentinels, retries, and cleanup; inspect native service boundaries. | Guard results, preserved sentinel state, ownership/cleanup inventory. | Stop unsafe work; repair guards on synthetic fixtures only. | C03 and C02. |

## QUALITY

No silent partial success. Report per-item outcomes and actionable failures.
Approval must not be inferred from selection. Preserve text-field shortcuts.
Selection, focus, and source/destination must be unambiguous (C01-C03).

## CONTEXT

Required: PRD, APP-01 bridge, chosen UI, fixture inventory.
Bound implementation: `src\files.rs`, `src\platform.rs`, `src\preferences.rs`,
`src\app.rs`, `tests\filesystem.rs`, and `tests\native-smoke.ps1`.
Current operation policy is in `docs\decisions\file-operations.md`: recursive
content copy, approval for every mutation, no overwrite/merge, actual Windows
move/recycling, cooperative cancellation, and explicit retained partial outputs.
Service commands: `cargo test --locked` and
`cargo test --locked --test filesystem recycle_and_restore -- --ignored --nocapture`.
Fixtures are uniquely named, inventoried and exactly cleaned; `TC_TEST_ROOT`
can bind an existing fixture parent. UI fixtures are owned by the smoke script.
Partial evidence: `FILE-01-native-ledger-status.json` and `native-ledger-smoke.json`
in the shared session artifact directory.
C01 remains NOT_RUN as a complete check: recursive search and the full physical
keyboard/mouse/favorite matrix remain unverified. C02 remains NOT_RUN as a complete
check: actual copy/move/recycle, collisions, stale plans, locked sources and
mid-copy cancellation pass bounded tests, but OS interruption, ACL permissions
and the complete before/after inventory/hash matrix are missing. C03 remains
NOT_RUN as a complete check: scoped guard tests pass, but durable retries and
stronger identity/race coverage do not. C04-PERF is BLOCKED on numeric limits.
Unresolved: deterministic search roots, complete recovery/interrupt behavior,
full safety/task matrix, and performance workload/thresholds.

## CONSTRAINTS

Inherit shared contract. Testing mutates only owned synthetic files.
Product access to user locations requires explicit scope; this draft grants none.
No broad cleanup or implicit overwrite.

## STAGES

1. Bind policies and fixtures; assess existing commands.
2. Implement service behavior and safety guards.
3. Verify real filesystem state and final user-visible behavior.

## STOP-CAPS

Inherit shared caps and outcomes. BLOCKED on unresolved operation policies.
Example repair: a multi-file copy skips a failure silently; add explicit
per-item errors, then rerun C02, C03, and the affected C01 display.
First-run learning: expand fixtures from observed conflict and Windows path cases.
