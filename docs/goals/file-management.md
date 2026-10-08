# Goal Card: Safe two-pane file management

## OBJECTIVE

Card: FILE-01 | Version: 3 | Readiness: Partially bound - physical/native input and complete performance acceptance pending
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
| C03 | Within the owner-approved normal desktop-use scope, move/delete/overwrite cannot occur without approval of exact targets. Tests never mutate pre-existing files or escape the test root. Cleanup removes only owned fixtures and reports failure. | Test refusal, changed paths, traversal/reparse escape, unowned sentinels, retries, and cleanup; inspect native service boundaries. | Guard results, preserved sentinel state, ownership/cleanup inventory. | Stop unsafe work; repair guards on synthetic fixtures only. | C03 and C02. |

Owner scope revision, 2026-10-08: initial-release safety covers normal desktop
use, stale-file checks and interrupted-operation recovery. Hostile processes
continuously replacing files/directories are explicitly excluded, not protected
against or passed. Exact approval, immutable plans, identity/stale checks,
fixture containment/reparse refusal and durable retry/interruption safeguards
remain mandatory.

## QUALITY

No silent partial success. Report per-item outcomes and actionable failures.
Approval must not be inferred from selection. Preserve text-field shortcuts.
Selection, focus, and source/destination must be unambiguous (C01-C03).

## CONTEXT

Required: PRD, APP-01 bridge, chosen UI, fixture inventory.
Bound implementation: `src\files.rs`, `src\search.rs`, `src\journal.rs`, `src\platform.rs`, `src\preferences.rs`,
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
The latest bounded acceptance pass is reconciled below. None of the complete
checks is promoted to PASS while its mandatory evidence remains unresolved.
The owner has now bound the normal desktop-use safety scope and warm service
ceilings; this does not waive physical input or other performance checks.

Search scope is bound to the active folder and descendants, without following
links/junctions. Places, Recent documents and Activity are explicit rail modes.
Recents are bounded to 20 document-like files opened through TomasCommander;
there is no OS-wide monitoring or access-timestamp scanning.

Own evidence directory for this milestone:
`C:\Users\tokubica\.copilot\session-state\7d4a0b6a-6e1c-4816-8428-3e2d2670bca5\files`.
Earlier native smoke has 27 checks and visual evidence has 30 screenshots, both
predating final expanded polish.

### Historical UI-polish evidence

These observations belong to the earlier builds and fixture inventories,
not the latest acceptance corpus or final executable. The corrected button layout was rebuilt and
directly observed at default and maximized native window sizes: top controls
remain on the right, action buttons share a baseline, and hints are centered
under their own actions. Source layout regressions cover 880, 1000, 1240 and
1600 logical-point widths, plus 400-point wrapped action rows. At that stage,
37 default Rust tests and strict Clippy passed. Anchored native Ctrl+Shift+C reaches copy preflight
and refuses an existing destination; Ctrl+Shift+B enters pinned-location focus.
Search query Enter produces three actual duplicate-name results and an unmatched
query reports no results. Native Size-header clicks toggle ascending/descending
order and arrows while preserving the parent and selected file. Ctrl+Shift+D
opens an exact-source Recycle Bin approval; cancelling leaves files unchanged.
The CLI action opens a separate Windows Terminal titled GitHub Copilot; terminal
inspection is blocked by the tool safety policy, so cwd/session acceptance is
not claimed. Native narrow-window verification subsequently succeeded using
Windows tiling shortcuts: the final release was observed at the 880-point
minimum with both panes and aligned controls visible. Tiling below the initial
native minimum exposed overflow; native viewport-size recovery now preserves
the shared 880 x 560 minimum, with regression checks for both dimensions and
no resizing of already-valid windows.

The native acceptance continuation on 2026-10-08 used the same release hash
and directly observed right-click selection/deselection across panes, Name
descending and Last modified ascending/descending order with parent/selection
stability, search cancellation/restart, arrow selection and Enter navigation
to an actual nested-name result. An unmatched query submitted with Enter
reported no results. Actual document-open requests added Czech and duplicate-name
documents to Recent; Ctrl+Shift+H, arrows and Enter reopened a document and
moved it to the front without duplication. Exact preference read-back and a
native close/relaunch confirmed both recents and pinned locations persisted.
Ctrl+Shift+B, Down and Enter then navigated to the persisted Beta favorite.
The rendered aggregate metadata showed Alpha 175 B, Beta 41 B, Empty 0 B
without a descendant date, and Large 810.0 KB. A new real directory read-denial
regression proves accessible bytes are retained with an explicit partial
warning; aggregate junction coverage proves links are not double-counted.
A further native check used a separately inventoried synthetic descendant with
real directory read denial: its aggregate rendered `9 B+` and a `*`-marked
modified date, rather than a fake complete value. Restoring the exact ACL,
reading back the two files and nonrecursive cleanup succeeded. The next native
Refresh image showed the 30,000-file Large aggregate as `...` in both metadata
columns while the listing and controls remained available. Quantified input
latency and the full physical responsiveness matrix remain outstanding.
The native Empty folder displayed only the parent row and zero real items;
Ctrl+Shift+D refused that navigation row without preparing a deletion.
Enter navigated back to the fixture root. A scoped Copilot App session-launch
approval was requested, but the owner was unavailable; session acceptance stays
BLOCKED and no approval was inferred.
This is bounded evidence, not the full physical input or responsiveness matrix.
Two interrupted native actions yielded to user input; no foreground workaround,
extra CLI launch or unapproved filesystem operation was attempted.

Do not promote earlier screenshots to evidence for the final build.
Earlier bounded observations and source hashes are in
`FILE-01-native-resume-status.json`, with the 37-test output in
`FILE-01-native-resume-tests.txt`, in the own evidence directory above.
`FILE-01-responsive-status.json` remains the earlier responsive-repair record;
its outstanding cancellation/recents/right-click gaps are superseded only by
the specific observations in the continuation record. The executable SHA256
for that earlier record is
`90B94165837ACF9B627120E28651D705716283A95C3B5FBB8A8988BF8CAFB9C3`.

### Latest bounded acceptance: 2026-10-08

Run: `final-acceptance-20261008-1056`. State: **BLOCKED, not DONE**.
Copilot integrations are explicitly excluded from this FILE-01 acceptance pass.
The earlier 120-minute/three-cycle contract was a proposal, not an owner-bound
cumulative cap; this pass reserved verification time and targeted a stop by
12:56. Prior work and failures remain recorded rather than resetting the pilot.

| Check | Complete status | Delivered evidence and precise remaining gate |
|---|---|---|
| C01 | BLOCKED | Parent/filter/root/selection invariants, six header directions, relative search/no-results and arrow/Enter activation, pending/partial metadata, Places/Recent keyboard flows and restart persistence observed. Current-build cancellation and restart pass on a separate owned 80,000-file corpus. Physical CZ/US/DPI, native Shift range, repeat same-point right-click and double-click outcomes remain unverified. |
| C02 | PASS, bound corpus/policy | Complete SHA256 service matrix covers recursive multi-item copy/move, nonoverwrite, refusal, ACL denial, partial failure, cooperative cancellation, forced interruption/read-back and exact real recycle/restore. Freshly approved native copy on the repaired build succeeds with source and destination hashes identical; the exact Delete plan was cancelled without authorization to execute it. |
| C03 | PASS, owner-approved desktop scope | Immutable plans, flushed durable claims/outcomes, clone/restart replay refusal, unknown-outcome overlap refusal, malformed-record fail-closed, source/ancestor guards, verified move identity/content and containment/sentinel/cleanup checks pass. Hostile replacement guarantees are explicitly outside scope; the documented caveats remain. |
| C04-PERF | BLOCKED; service subcheck PASS | All three final warm trials meet owner-approved listing/filter/search ceilings. Cold-start, resource and input-response limits remain unbound; instrumented input latency and physical responsiveness acceptance remain outstanding. |

Final source passes **46 default Rust tests**, strict Clippy and formatting.
The earlier final-source opt-in real Recycle Bin test passes with identical
restored SHA256; its filesystem/platform implementation hashes are unchanged
by the later input/journal repairs. The final isolated release is
`D:\TomasCommander\target\file01-acceptance\release\tomas-commander.exe`,
SHA256 `0C1B507738EA1A2BAC693AE1C29BFCD6A9B3C4F4094000E57C2A86B2DA49A65F`.
The isolated path avoids replacing the user's running earlier executable.
Restart using this exact executable for the durable recovery/move/input guards.
No commit, push, branch change, dependency addition or Copilot launch occurred
in this acceptance pass.

The owned native corpus binds 10,020 Large files, empty/nested/duplicate/Czech/
spaced inputs, a real read-denied folder, two junctions and an outside owned
sentinel. Exact denial removal succeeded; the earlier complete read-back
verified **10,029 unchanged input-file SHA256 hashes**. Current full read-back
also verifies those originals, all **80,000 cancellation inputs**, and the
approved source/target bytes. The checker initially misdecoded one Czech path
using the Windows default codepage; an explicit UTF-8 inventory read and exact
bound-path reread corrected that checker error. The failed checker record is
retained; no input was modified or silently substituted. The later cancellation
corpus contains 80 folders with 1,000 inventoried real files each; it is not a
replacement performance workload. Inputs, settings, journals and the approved
new copy are intentionally retained, without cleanup permission. All owned
acceptance helpers were closed and process shutdown verified; no other app
was closed by this run.

Current warm release trials over the frozen 10,020-file local corpus:
listing **641.87 / 559.22 / 556.77 ms**, local filter
**0.97 / 0.49 / 0.52 ms**, recursive search
**1053.31 / 1101.63 / 1026.20 ms**. Owner-approved ceilings are respectively
**3500 / 3 / 6000 ms**. Compare the maximum of all three consecutive warm
trials against each ceiling. The corpus uses eight-byte files named
`match-00000.txt` through `match-10019.txt`; local query `019` finds 121 files.
Recursive query `match` reaches the real 10,000-result cap with explicit
truncation: this is not unlimited traversal or a throughput benchmark.
Twenty exclusive eight-byte copies took 7.63 ms planning and 89.61 ms validated
execution/sync/read-back. An earlier shared-machine
trial set ranged up to 2719.45 ms listing and 4765.33 ms search; retain this
variation rather than selecting only the faster run. Earlier A253 trials
overlapped native-corpus SHA256 read-back; current repaired-build service trials
predate the new cancellation corpus/read-back. Neither is an isolated-machine
or cold-cache benchmark. Current native initialization reported 126 ms; idle
private memory was 76.5 MiB, working set 120.79 MiB and CPU 62.5 ms over
5155.96 ms. Resource/startup numbers remain baselines without approved ceilings;
tool round-trip duration is not input latency.

Two follow-up defects were repaired rather than waived. Shift navigation now
uses the pressed key event's modifiers, including when Shift release arrives
in the same frame; regressions cover all four navigation keys and unrelated
chords. Journal storage is initialized before capturing a plan, so first-use
creation cannot invalidate the destination snapshot. The real regression
proves copying succeeds while subsequent unrelated destination changes still
refuse a stale plan.

The first specifically approved native copy on A253 was safely refused:
first-use journal creation changed the destination stamp. That permission was
consumed and never reused. A fresh specific approval on 0C1 authorized exactly
one `Alpha\Nested\duplicate.txt` to root `duplicate.txt` copy. Native Activity
reported one completed node, one created path and zero incomplete items;
both files are 22 bytes with identical SHA256. The target remains for review.
Current-build native checks also show all six header directions with parent
and selected Beta retained; query Enter returns four actual duplicate paths
including the new target. Result arrows/Enter navigate to Alpha, and an
unmatched query reports no results. A genuinely running 80,000-file search
was cancelled through its current control, displayed `Search cancelled.`,
then restarted to return 80 real relative-path matches. A saved rendered
Refresh image shows pending `...` metadata beside completed 25.0 KB folders
while controls/listing remain available.

Native Shift injection, repeated same-point right-click and indexed double-click
reported dispatch without their required observable outcomes. These remain
unverified, not successful hardware tests or permission to waive the repaired
Shift regression. No shell input/foreground workaround was used.

After the owner's explicit continuation request, a temporary fixture-only input
trace distinguished delivery from application behavior. Both supported Shift
chord spellings reached the app as ArrowDown with no Shift modifier. Indexed
double-click delivered one primary press/release pair; repeated same-point
right-click delivered no recorded pointer/accessibility event, although the
first right-click selected the file. These attempts cannot establish the required
native behaviors or an additional product defect. Physical input checks remain
mandatory. The diagnostic app was closed, the logger removed, and all 15 bound
source identities verified unchanged. Formatting, all 46 default tests, strict
Clippy and release compilation passed again. Rebuilt executable bytes differed
and were preserved separately, not promoted to native evidence; the exact
previously observed `0C1B5077...` executable was restored and hash-verified.
This continuation retains the prior run's accounting and consumed approvals.

At 13:21 the owner answered the pending physical check: "I cannot test them
now." This run is stopped with C01/C04 blocked; that answer supplies no physical
acceptance or performance approval. A later resource-baseline attempt collected
zero trials because unexpected external interaction/target-state ambiguity
prevented an idle measurement. The two production windows (722826 and 2233004)
are intentionally left untouched. No further UI actions, launches, resource
trials or questions are scheduled. Exact pending gates and the corrected manual
procedure are saved in `FILE-01-human-acceptance-gate.json`; the zero-trial
attempt is recorded in `FILE-01-resource-baseline-attempt.json`.

Evidence in the own directory above:
`FILE-01-final-acceptance-status.json`,
`FILE-01-final-verified-identities.json`,
`FILE-01-final-verified-tests.txt`,
`FILE-01-final-verified-clippy.txt`,
`FILE-01-final-verified-release.txt`,
`FILE-01-final-verified-recycle.txt`,
`FILE-01-final-verified-baselines.txt`,
`FILE-01-final-verified-native-performance.json`,
`FILE-01-final-native-observations.json`,
`FILE-01-final-native-tool-record.json`,
`FILE-01-final-corpus-inventory.json` and
`FILE-01-final-native-hash-readback.json`.
Current continuation records:
`FILE-01-current-verified-identities.json`,
`FILE-01-input-journal-followup-identities.json`,
`FILE-01-input-journal-followup-tests.txt`,
`FILE-01-input-journal-followup-clippy.txt`,
`FILE-01-input-journal-followup-release.txt`,
`FILE-01-input-journal-followup-performance.json`,
`FILE-01-input-journal-followup-baselines.txt`,
`FILE-01-approved-native-copy-refused.json`,
`FILE-01-approved-repaired-native-copy-after.json`,
`FILE-01-current-native-performance.json`,
`FILE-01-cancel-corpus-inventory.json` and
`FILE-01-current-full-readback.json`.
The later input-delivery trace, exact scoped tool outcomes and restoration
are in `FILE-01-input-diagnostic-evidence.json` and
`FILE-01-input-diagnostic-restoration-checks.txt`.
Native tool records preserve actual accessibility results and persisted image
asset references, not invented screenshot filenames. Earlier expanded UI observations retain their original binary identity.
Places/Recent persistence, aggregate services and responsive layout were not
changed by the later event-bound file-navigation/journal-initialization repairs;
current-build search/sorting/approval/pending metadata were separately observed.

Manual unblock: using the owned fixture on each physical CZ/US layout, exercise
Copy, pinned-folder arrows/Enter, query Enter and Delete/Cancel; check text-field
clipboard shortcuts, Shift range, repeat right-click, double-click and parent
exclusion. For a measurable range, sort Alpha by Name ascending, click
`duplicate.txt` and press Shift+Down: it and `with spaces.txt` must be selected,
with a count of two. Starting on the last row (`Česká.txt`) cannot demonstrate
an extended range. Clear selection before the two same-point right-clicks
(zero to one to zero); double-click Nested must navigate to Alpha's Nested
folder. Repeat at the owner's required DPI/window sizes. Owner must separately
bind startup, resource and input-response limits; approved warm service limits
and the normal desktop-use scope are already recorded. Native cancellation
and fresh copy confirmation are no longer pending.

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
