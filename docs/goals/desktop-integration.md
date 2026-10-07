# Goal Card: Windows shell and app integrations

## OBJECTIVE

Card: APP-01 | Version: 4 | Readiness: Draft - full integration and performance acceptance blocked
Mode: bounded repair loop for behavior; one-pass capability investigation
Owner: project owner

Run the selected presentation as a real standalone Windows application, not a
browser page, with a preferably native Rust renderer and typed service
boundaries. Open a selected folder in VS Code and start a GitHub Copilot App
session through a supported integration. Verify feasibility first; if no supported
mechanism exists, this capability is BLOCKED pending an explicit scope decision.
Do not replace Windows, automate private app internals, or claim unsupported
capabilities work.

## OUTPUT

Update `docs\decisions\desktop-shell.md` with the resolved egui/eframe renderer,
measured native-build evidence and verified integration contracts; preserve its
source manifest and explicitly unresolved findings. Implement the Windows host and launch adapters at source
paths bound after that choice. Add build/run instructions and integration tests.
Use the shared run record for evidence and explicit partial-output status.

## DONE WHEN

Mandatory C04-PERF from `README.md` applies: compare Rust-native UI against any
thin web-view alternative, choose with owner approval, and measure the real
Windows host. Rust owns native services; no JavaScript-heavy shell by default.

| ID | Pass | Verify | Evidence | Failure response | Recheck |
|---|---|---|---|---|---|
| C01 | A standalone Windows application runs the accepted view, both themes, all four configurable canonical accent families, and shared keyboard/mouse commands without requiring a browser page or localhost service. Native Rust rendering is used unless an explicitly approved measured exception exists. Source paths and build instructions are documented. | Build and launch the real executable using bound commands; repeat the UI task inventory under CZ/US layouts and cycle appearance controls. | Build output, actual app launch, appearance/task results, and renderer decision. | Repair host wiring; stop for owner resolution of shell tradeoffs. | C01 and affected UI checks. |
| C02 | VS Code opens the exact synthetic folder, including spaces/Czech characters, without interpreting its path as shell syntax. Missing VS Code produces an actionable error. | Invoke the real adapter on fixture folders and inspect the opened workspace; test unavailable-app behavior. | Exact target and observed workspace/error. | Repair typed argument handling or detection. | C02. |
| C03 | The product starts an actual Copilot App session for the selected synthetic folder through a verified supported mechanism. | Inspect authoritative integration documentation; invoke the real product command and verify the new session and its target folder in Copilot App. | Inspected documentation, invocation result, and actual session/folder read-back. | Fix the supported integration; if unavailable, mark BLOCKED and request an explicit scope revision, not fallback acceptance as a pass. | C03 and C01 if commands change. |

## QUALITY

Keep presentation independent of native services. Explain shell tradeoffs using
inspected evidence, not popularity. An unavailable integration is not a working
launch button (C03). All final outputs are reconciled under the shared contract.

## CONTEXT

Required: PRD, accepted UI direction, Windows environment, app installation
details. Accepted direction: Ledger, with a contextual left rail rather than a
favorites-only panel. Implemented shell: egui/eframe with Glow; rationale and prerequisites
in `docs\decisions\desktop-shell.md`.
Bound native outputs: `Cargo.toml`, `Cargo.lock`, `src\main.rs`, `src\app.rs`,
`src\files.rs`, `src\preferences.rs`, `src\platform.rs` and
`target\release\tomas-commander.exe`. Rust/MSVC tooling is installed.
Validation: `cargo test --locked`, `cargo clippy --locked --all-targets -- -D warnings`,
`cargo fmt --all -- --check`, `cargo build --release --locked`, and
`tests\native-smoke.ps1` using only owned synthetic fixtures.
Partial evidence: `APP-01-native-ledger-status.json` and
`native-ledger-smoke.json` in the shared session artifact directory.
C01 remains NOT_RUN as a complete check despite the standalone/window/appearance
subchecks passing: physical CZ/US and the full UI inventory have not run.
C02 is NOT_RUN: the typed VS Code adapter exists, but actual workspace/error
read-back has not been completed. C03 is BLOCKED: no supported launch mechanism
has been verified or wired. C04-PERF is BLOCKED pending numeric limits and workload.
Unresolved: measured thresholds, packaging scope, supported Copilot mechanism.
Current environment tool availability does not prove target-user installations.

## CONSTRAINTS

Inherit `README.md` shared contract. Testing launches only synthetic locations.
No OS setting changes, extension installs, private API scraping, or credential
reuse without explicit authorization.

## STAGES

1. Resolve approved toolchain/source and native viability parameters; verify
   remaining integration options. Start with the recorded egui/eframe path.
2. Implement host and typed adapters.
3. Verify real launches and saved outputs; record unsupported capabilities as
   BLOCKED, never delivered.

## STOP-CAPS

Inherit shared pilot caps; bind any additional research/action budget before
execution. BLOCKED until shell and scope decisions are resolved.
Example repair: if a spaced path opens incorrectly, repair argument passing,
then rerun C02 and affected C01 commands.
First-run learning: record Windows focus behavior and installation assumptions.
