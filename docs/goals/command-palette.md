# Goal Card: Deterministic and natural-language commands

## OBJECTIVE

Card: CMD-01 | Version: 2 | Readiness: Draft - AI scenario and performance bindings blocked
Mode: bounded repair loop | Owner: project owner

Offer Ctrl+Shift+P command discovery and deterministic actions, plus bounded
natural-language planning through the cheap model. Support a fixture scenario:
find the Penta hackathon presentation and create a copy in the Kosik folder.
Destructive actions require explicit approval; ambiguous targets require
clarification, not guessing.

## OUTPUT

Implement shared command registry, palette, typed operation plans, approval UI,
and executor at bound source paths. Create
`docs\decisions\command-execution.md` with supported intents and approval policy.
Add frozen fixture scenarios and evidence under the shared contract.

## DONE WHEN

Mandatory C04-PERF from `README.md` applies: registry, planning validation, and
execution are Rust-first. Measure palette opening/matching and local dispatch
without counting remote inference as local processing.

| ID | Pass | Verify | Evidence | Failure response | Recheck |
|---|---|---|---|---|---|
| C01 | Palette opens under CZ/US layouts, lists/searches deterministic commands, preserves input editing, and restores focus. Deterministic commands work without AI access. | Run actual keyboard/mouse inventory, including provider disabled and no results. | Layout/task results, focus and offline observations. | Repair registry, matching, or focus. | C01 and C02 if dispatch changes. |
| C02 | Frozen natural-language scenarios resolve the expected source/destination or ask the required clarification. Approved execution produces the exact fixture state; copy preserves source/content. | Compare typed plans and filesystem read-back with owner-reviewed scenarios, including Czech/English wording, ambiguous files, and conflicting destination. | Plans, clarification/approval events, inventories and hashes. | Repair interpretation or planning; never loosen expected effects. | C02 and C03. |
| C03 | Move/delete/overwrite are blocked before target-specific approval. Changed plans invalidate approval. Malformed tools, prompt injection, denied/cancelled plans, and duplicate retries cannot cause unauthorized effects. | Run guard tests and real synthetic-file cases; inspect operation IDs/read-back and approval boundary. | Guard outcomes, approvals, unchanged state on denial, retry evidence. | Stop unsafe execution; repair application guards. | C03 and C02. |

## QUALITY

Show exact paths, operation type, conflicts, and per-item outcomes before relevant
approval. Model confidence is not permission. Natural language and deterministic
commands use the same executor; neither bypasses FILE-01 safety.

## CONTEXT

Required: shared registry, FILE-01, AI-01.
Bound deterministic implementation: typed registry and UI in `src\app.rs`,
shared real filesystem execution in `src\files.rs`; policy in
`docs\decisions\command-execution.md`. Every mutation, including copy, currently
requires target-specific approval; natural-language plans are not implemented.
Validation: `cargo test --locked` and `tests\native-smoke.ps1` against owned
synthetic fixtures. Actual native palette invocation passes the smoke; the
AccessKit editor bridge has a targeted unit test.
Partial evidence: `CMD-01-native-ledger-status.json` and `native-ledger-smoke.json`
in the shared session artifact directory.
C01 remains NOT_RUN as a complete check pending physical layouts and the full
focus/offline/no-results matrix. C02 and the AI portion of C03 are BLOCKED on
AI-01 and frozen approved scenarios; deterministic guards do not pass AI checks.
C04-PERF is BLOCKED on numeric limits/workload. No model output was fabricated.
Unresolved: allowed intent inventory, ambiguous-match policy, copy approval
policy, multi-step plan limits, and owner-reviewed scenarios.

## CONSTRAINTS

Inherit shared contract. No arbitrary shell, messaging, purchases, or workplace
mutation tools. Only approved fixture operations during implementation testing.

## STAGES

1. Bind supported intents, fixture scenarios, and approval policy.
2. Implement deterministic dispatch and typed AI planning.
3. Verify exact effects, ambiguity handling, and denied-action safety.

## STOP-CAPS

Inherit shared caps and approved plan/tool/model-call counts.
Example repair: a changed destination retains approval; invalidate approval on
plan change and rerun C03 and C02.
First-run learning: record clarification frequency and unsupported intents.
