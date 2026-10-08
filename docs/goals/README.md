# Feature Goal Cards

These cards cover the product brief, not just its first UI milestone.
The owner of material decisions and final acceptance is the project owner.
Creating a card does not start its implementation or grant tool permissions.

| Card | Feature | Status | Prerequisites |
|---|---|---|---|
| UI-01 | [Five UI alternatives](ui-exploration.md) | Ledger direction accepted; no more design batches; physical-layout check still pending | None |
| APP-01 | [Windows shell and app integrations](desktop-integration.md) | Real Rust native slice built; physical-layout/launch/measurement acceptance incomplete | VS Code workspace read-back; supported Copilot mechanism; performance limits |
| FILE-01 | [Two-pane file management](file-management.md) | Bound operation/safety checks and warm service ceilings PASS; complete acceptance BLOCKED | Physical/native input verification; startup/resource/input limits |
| AI-01 | [Foundry connection and bounded runtime](ai-runtime.md) | Scoped bounded text/runtime pilot verified; C01-C04 PASS | Broader app/hardware acceptance remains separate |
| SEARCH-01 | [AI-assisted file search](ai-file-search.md) | Draft | FILE-01 discovery; AI-01 |
| CMD-01 | [Deterministic and AI command palette](command-palette.md) | Real deterministic palette; AI portion BLOCKED | Physical-layout/focus matrix; AI-01; performance limits |
| ANSWER-01 | [Web and workplace answers](connected-answers.md) | Draft | AI-01; authorized MCP connections |
| VOICE-01 | [Voice input](voice-input.md) | Draft | CMD-01; speech/privacy decision |

Prerequisites describe capability dependencies, not a requirement to implement
each card serially. Deterministic portions can be developed before provider
integration, but all product cards require real working software at completion.
Only UI-01 is a mock design exercise.

Native implementation bindings and partial evidence are recorded in APP-01,
FILE-01 and CMD-01 below, without weakening their mandatory checks. The latest
source validation passed 46 default tests, strict Clippy, formatting and an
isolated release build. Final-source opt-in real recycle/restore and the SHA256
operation/recovery matrix pass. Expanded native observations cover parent,
search, sorting, pending/partial metadata, selection and persisted Places/Recent
flows; the repaired-build native copy and actual search cancellation/restart
also pass. The owner accepted normal desktop-use safety and scoped warm service
ceilings, without hostile-race guarantees or full performance acceptance.
Precise unverified dispatch/physical checks remain. The current
reconciliation is `FILE-01-final-acceptance-status.json` in FILE-01's own
evidence directory; older native records keep their earlier build identity.
These results do not complete any product card: integrations, physical keyboards, full matrices and
numeric performance acceptance remain outstanding.

## Real-product acceptance rule

APP-01, FILE-01, AI-01, SEARCH-01, CMD-01, ANSWER-01, and VOICE-01 require actual
application behavior and actual integrations. Mocks, canned model output,
hardcoded search answers, simulated file operations, disabled buttons, and
success-shaped fallbacks cannot satisfy any real-capability acceptance check.

Synthetic fixtures are inputs for real filesystem operations. Test doubles may
exercise failures in unit tests, but do not replace successful end-to-end checks.
AI, MCP, speech, and application launching must use their actual authorized
providers/adapters through the product, not these chat tools or prerecorded
responses. Record real read-back and provider evidence without exposing secrets.

If access is missing or a capability is unsupported, its check is BLOCKED.
Do not count an unavailable integration as delivered, even when clearly labeled.
An owner-approved scope revision must change the card explicitly before a
different deliverable can be accepted.

## Shared execution contract for the draft cards

The seven draft cards below incorporate this section. UI-01 keeps its existing
self-contained contract.

### Rust-first and lightweight performance contract

All product cards require Rust-first implementation. Filesystem work, discovery,
indexing, command execution, safety policy, provider/MCP orchestration, and speech
lifecycle belong in Rust wherever practical. A non-Rust exception needs a
specific capability reason and owner approval. Any web UI must remain a thin
presentation layer, not a second implementation of business logic.

Final delivery is a standalone Windows desktop application, not a browser page.
Prefer a native Rust renderer. Any proposed web-view exception needs measured
overhead, a concrete reason the native option does not meet agreed needs, and
explicit owner approval. Do not presume the browser prototype is the product
architecture. Appearance uses one-click theme switching and a simple accent
control for the four canonical families; this applies to all feature surfaces.
Prefer minimal dependency count, small maintained libraries, and standard
facilities; justify the capability and runtime/packaging cost of each production
dependency. Do not introduce a large agent harness without demonstrated need.

Every product card includes mandatory C04-PERF below in addition to its existing
checks. Stable ID: C04-PERF within each card.

| ID | Pass | Verify | Evidence | Failure response | Recheck |
|---|---|---|---|---|---|
| C04-PERF | Rust owns the feature's non-presentation functionality, exceptions are owner-approved, dependencies are justified, and the real saved build meets agreed numeric resource/latency thresholds under its frozen workload. | Inspect source/service boundaries and dependency inventory; measure the real Windows build using the bound benchmark procedure. Separate remote-provider waits from local CPU/UI work; exercise a pending scan/request to check UI responsiveness. | Build/version, machine profile, dependency rationale, workload, trial results, startup/idle-memory/CPU and relevant action latencies, plus owner-approved thresholds and exception decisions. | Repair Rust/service boundaries or measured bottlenecks within scope; stop for missing thresholds or unavoidable tradeoffs. Never substitute a mock benchmark or weaken limits. | C04-PERF and all affected functional/safety checks. |

Bind representative file counts/types, input sizes, concurrency, cold/warm
conditions, statistic, trial/repeat procedure, measurement tools, and acceptable
startup/memory/input-response values before execution. Thresholds are unresolved,
not permission to call any implementation "extremely fast". Provider deployment
latency is a separate metric, not evidence that local code is slow or fast.

First-run learning should establish measured baselines and identify actual
bottlenecks. A fast language alone cannot pass C04-PERF.

### Evidence and final reconciliation

Before implementation, bind the actual source/output paths, runtime, validation
commands, fixture inventory, provider configuration, and owner-approved limits
in the card. Do not invent validation commands for an unselected framework.

Use the existing session artifact directory:
`C:\Users\tokubica\.copilot\session-state\947dfde0-a1eb-40dc-b348-628e8df8e692\files`.
Each run writes `<card-id>-<run-id>-status.json` there. Bind the run ID before
execution. This naming pattern is a reusable binding rule, not a resolved run.

Record card/version, source and input versions, check ID, NOT_RUN/PASS/FAIL/BLOCKED,
timestamp, checker or command, observed result, evidence pointer, cumulative
limits, repair outcomes, next gap, and external operation identifiers.
Exclude secrets and unnecessary personal content.

All checks are mandatory. At design time none has passed. Review the saved final
artifacts and invalidate checks affected by repairs. Missing evidence, skipped
tests, unavailable sources, or ambiguous outcomes cannot pass. Preserve
unaffected evidence only with a recorded reason. Never weaken acceptance tests,
change the rubric, or drop requirements to manufacture success.

### Permissions and safety

Read only approved project sources and explicitly authorized data.
Write only bound feature outputs, related documentation, and evidence.
Preserve unrelated changes. No commits, pushes, deployments, new subscriptions,
credential disclosure, or unapproved paid calls.

All implementation testing that mutates files uses a dedicated synthetic test
root and an ownership inventory. Resolve paths and enforce root containment,
including reparse points and path changes at execution time. Never modify
existing user files for tests. Cleanup removes only inventoried test-owned
artifacts; preserve unrelated additions and report cleanup failures.

Model output and retrieved content are untrusted input. Only application code
may authorize typed tool calls. No arbitrary shell execution or hidden access.
Human approval must identify the exact operation and targets; a materially
changed plan requires renewed approval.

Before retrying a state-changing action, perform read-back or use a persisted
deduplication key. An unknown outcome is a stop condition, not permission to
repeat. Rollback is allowed only when explicitly safe and scoped.

Follow approved package feeds for any necessary dependency work. Preserve
lockfiles. Document deviations from established project patterns.

### Limits and terminal outcomes

Pilot proposal per implementation run: three assess-act-check-adjust cycles and
120 minutes, reserving the final 20 minutes for verification and saving state.
Bind owner-approved provider/model/MCP operation limits before any live calls;
no unapproved paid calls. These proposals do not authorize spending.

Count the first pass as cycle one; include retries and delegated work.
Resume does not reset counters. Waiting consumes elapsed time.
Stop after three consecutive cycles without closing a failed check or reducing
the recorded defect count, without a new required regression. More searches or
cosmetic rewrites do not count as improvement.

Check limits before acting and preserve time for final reconciliation.

- DONE: every required check passes on final outputs; approvals are evidenced;
  no unresolved constraint violation.
- BLOCKED: required input, permission, provider, check, or owner decision is
  missing. Record the precise unblock action.
- CAPPED: a resource/no-progress cap is reached before DONE.
- CANCELLED: the owner stops the run. Preserve safe state and stop.

Preserve partial outputs and remaining gaps for every non-DONE exit.
End-of-run is not success. Required owner acceptance is a human gate, not a
reason to keep repairing without feedback.

## Loop strategy and first-run learning

Use bounded repair loops for reproducible behavioral failures. Stop for missing
authority, ambiguous user intent, provider unavailability, or human acceptance.
Do not run an umbrella loop that tries to implement every feature at once.

Keep the first synthetic corpora and task inventories small and owner-reviewable.
Use observed failures to refine fixture coverage, data contracts, and later
performance thresholds. Do not report invented latency or accuracy targets as
user requirements.
