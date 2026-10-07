# Goal Card: Five mock UI alternatives

## OBJECTIVE

Card: UI-01 | Version: 2 | Readiness: Ready for bounded browser design exploration
Mode: repair loop with human review | Decision owner: project owner

Produce five interactive UI alternatives for TomasCommander: two
Commander-dense, two modern terminal-inspired, and one minimal. Enable the
owner to choose a direction. Exclude real filesystem operations, live AI,
external app launching, MCP connections, and voice capture.

Apply the owner's Rust-first, extremely lightweight/fast direction during this
round. Keep visual design separate from renderer selection. Browser-only mock
logic is disposable design scaffolding, not authorized production JavaScript.
Do not claim this UI exercise demonstrates Rust/native product performance.
Owner decision: browser previews are approved for this design round only; the
selected real product must be a standalone Rust-first Windows application.

Review outcome, 2026-10-07: owner accepts Ledger / alternative 02 as sufficient
direction to proceed to the real product. The left rail becomes a contextual
panel instead of permanently only pinned locations. No further visual batch is
requested. C05 is satisfied for direction selection; C02 physical-layout evidence
remains missing and must not be represented as passed. Repeat keyboard validation
on the real native application under APP-01.

Readiness describes the contract, not task success. No check has passed.

## OUTPUT

- Editable prototype sources and one comparison launcher in `prototypes\ui\`.
  Create new files; preserve unrelated work. Subsequent rounds update these
  sources only after owner feedback.
- Shared synthetic dataset, application state, and command registry in that
  directory. Five views consume the same behavior.
- Run status and evidence in
  `C:\Users\tokubica\.copilot\session-state\947dfde0-a1eb-40dc-b348-628e8df8e692\files\ui-01-status.json`.
  Create or update this run record without resetting cumulative caps.
  Store screenshots alongside it with run/version-specific filenames.
- A runnable local comparison surface. Record its launch instructions in
  `prototypes\ui\README.md` and verify those instructions.

Mark unfinished work as partial in the run record. A reviewable batch is not
an accepted design.

## DONE WHEN

All checks are required for DONE. Record NOT_RUN, PASS, FAIL, or BLOCKED with
artifact version, timestamp, checker/procedure, observed outcome, and evidence
pointer in the run record.

| ID | Pass condition | Verify | Evidence | If it fails | Recheck |
|---|---|---|---|---|---|
| C01 | Five runnable alternatives share the same synthetic files and commands. All support browsing, selection, filtering, favorites, operation previews, sorting, resizing, and palette use. Their structure and density differ, not just color. Launch documentation works. | Executor launches the saved sources using the documented instructions, inspects all five, and runs the same task inventory in each. | Launch outcome, behavior matrix, and screenshots of all five. | Repair missing behavior, differentiation, or instructions inside prototype sources. | C01 and checks affected by the repair. |
| C02 | Each view supports all C01 tasks without a mouse, with visible focus and predictable focus restoration. Ctrl+Shift+P works; no required F-key, Z/Y, or punctuation binding. Inputs keep normal text-editing behavior. | Executor performs the task inventory on Windows under actual CZ and US layouts, including text-field and overlay contexts. Synthetic event tests supplement but do not replace layout checks. | Layout names, input method, per-task results, and focus observations. | Repair bindings or focus. If a layout or direct input check is unavailable, record BLOCKED, not PASS. | C02 and C03 when appearance changes. |
| C03 | Every view has one-click light/dark switching and a simple Accent control cycling the four canonical families. Only one family is active; theme changes preserve it. Paths are readable, focus/selection visible, controls unclipped at 1366x768 and 1920x1080, and all actions mouse-accessible. | Inspect each view at both sizes in all eight theme/accent combinations; exercise both appearance controls and mouse equivalents of the task inventory. | Screenshots and appearance/mouse results per view, size, theme, and accent. | Repair layout, contrast, or controls. | C03 and C02 if interaction changes. |
| C04 | All file changes affect only in-memory synthetic state. No real file access, live AI requests, microphone capture, or external app/MCP actions. Move/delete require explicit approval; cancellation leaves mock files unchanged. Simulated AI and launch results are labeled. | Executor inspects service wiring and runtime network activity; compares mock state before/after approved and cancelled operations, including conflicts, errors, and empty results. | Adapter inspection, observed requests, and state assertions. | Remove unsafe wiring or repair simulation/approval handling. Stop immediately if an unexpected real side effect occurs. | C04 and affected C01/C02 workflows. |
| C05 | Owner explicitly accepts a specific saved design version. Choosing a candidate with requested changes is feedback, not final acceptance. | Owner reviews the comparison surface; executor records the exact decision and version. | Decision reference tied to final artifacts. | Stop for feedback or approval. A new five-design round needs an explicit owner request. | C05 and checks affected by authorized revisions. |
| C06 | This round adds no framework/runtime packages, network assets, or production JavaScript service logic. Renderer limitations are explicit, and the owner agrees whether to keep browser scaffolding for design review or require a Rust-hosted/native exploration. | Inspect prototype imports, assets, launch instructions, and the recorded renderer decision. Measure prototype behavior only as prototype behavior; do not treat browser overhead as product overhead. | Dependency/asset inventory, renderer decision, and explicit performance limitations. | Remove unnecessary dependencies; stop for a renderer decision if the current approach is not accepted. | C06 and C01-C04 if the renderer changes. |

Final reconciliation: evidence must apply to the saved final artifacts.
Invalidate affected checks after repairs; retain unaffected evidence only with
a recorded reason. Empty, skipped, unavailable, or ambiguous checks cannot pass.

## QUALITY

Use real semantic controls with terminal-inspired typography and structure.
Do not substitute screenshots or decorative ASCII for interactive controls.
Use the same scenarios in each view for a fair comparison (C01).
Keep focus, selection, and approval targets explicit (C02-C04).
Do not fabricate measurements, tested layouts, AI responses represented as
live results, or owner approval. The owner resolves subjective visual choices
(C05); visual preference does not waive safety or functional checks.

## CONTEXT

Required sources:

- `D:\TomasCommander\PRD.md`: agreed requirements and proposed pilot defaults.
- `D:\TomasCommander`: implementation repository; initially README and license.
- User brief and decisions from this session: mock UI first, the five-design
  range, Ctrl+Shift+P, and no destructive testing of existing files.
- Canonical styling reference:
  `C:\Users\tokubica\.copilot\skills\html-docs\assets\tokens.css`.

At run start, record the run ID, card version, source revision/dirty state,
dataset version, and available Windows/browser verification tools.

The final product is a standalone Windows application with preferably native
Rust rendering, not a browser page. Any browser exploration requires explicit
agreement as a temporary design-only artifact under C06.

Pilot defaults: English labels, initially blue accent, synthetic Czech/English filenames,
and the PRD shortcut proposals. The current dependency-free browser prototype
does not commit to the eventual desktop shell. The owner explicitly approved
retaining it as disposable scaffolding for this round. Real product performance thresholds will be
bound in APP-01, not inferred from this browser exercise.

## CONSTRAINTS

Read project files and the local style reference. Write only prototype sources,
directly related project documentation, and named session evidence artifacts.
Preserve unrelated changes. No commit, push, deployment, paid calls, dependency
installation without an actual need, real file mutations, or credential access.

If dependencies are necessary, follow the managed-machine approved feed policy;
never bypass it. Prefer existing tooling and dependency-free prototypes.

Keep criteria fixed. Changes to the finish line require owner approval.
Maintain run/card version, cumulative cycles/time, evidence, repair outcomes,
next gap, and any unexpected external effects. Resume reads this record and
does not reset limits. Do not retry an unknown external effect.

For later filesystem work, the PRD fixture-only testing rule remains mandatory;
this card grants no permission to test against real files.

## STAGES

1. Inspect existing artifacts and tool availability. Bind the shared dataset and
   task inventory. Stop if valid evidence already proves an accepted result.
2. Produce or repair the five views through shared commands and mock adapters.
3. Verify final saved artifacts against C01-C04 and reconcile evidence.
4. Present the batch and stop for owner review. Apply only authorized feedback.

## STOP-CAPS

Pilot caps for the initial implementation run: one five-view batch, three
assess-act-check-adjust cycles including the first pass, and 120 minutes of
elapsed time including verification and waiting. Reserve the final 20 minutes
for checks and saving state. No paid operations or live integration calls.

Stop after three consecutive cycles without closing a failed check or reducing
the recorded behavioral defect count, with no new required regression.
Cosmetic changes do not reset the no-progress count.

Do not start work that exceeds a cap or leaves insufficient verification time.
Reaching owner review ends automatic repair work; waiting does not authorize a
new batch. A later owner-authorized run gets a new recorded run ID and cap;
resuming the same run does not reset counters.

- DONE: C01-C05 pass for the final artifacts with no constraint violation.
- BLOCKED: a required tool, actual keyboard-layout check, permission, or owner
  decision is unavailable. Record the precise unblock action.
- CAPPED: a resource or no-progress limit is reached before DONE.
- CANCELLED: the owner stops the run; preserve state and do not continue.

For every non-DONE exit, preserve partial sources, check results, remaining
gaps, and cumulative counters. A review-ready batch awaiting acceptance is
BLOCKED on C05, not a completed design.
