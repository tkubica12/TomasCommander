# Goal Card: Foundry connection and bounded AI runtime

## OBJECTIVE

Card: AI-01 | Version: 1 | Readiness: Draft - not ready to run
Mode: bounded repair loop | Owner: project owner

Connect the app to an authorized Foundry deployment using the intended
tomas@tomasonline.net identity. Provide a small typed tool runner for cheap
intent/search tasks and a separately configured stronger-model route.
Choose a larger harness only if verified requirements justify it.

## OUTPUT

Create `docs\decisions\ai-runtime.md` describing direct runner versus harness
tradeoffs, selected integration, authentication, data rules, and limits.
Implement provider/tool interfaces, non-secret configuration, and tests at
bound source paths. Record sanitized live-call evidence under the shared contract.

## DONE WHEN

Mandatory C04-PERF from `README.md` applies: provider/tool orchestration and
validation are Rust-first. Justify any SDK/harness overhead; measure local runner
cost separately from network/model time.

| ID | Pass | Verify | Evidence | Failure response | Recheck |
|---|---|---|---|---|---|
| C01 | Decision identifies supported APIs, approved-feed package availability, required capabilities, and the smallest suitable implementation. Cheap/stronger deployment IDs are verified, not inferred from model names. | Inspect official sources and approved registry settings; resolve chosen dependencies; check authorized deployment configuration. | Decision, source references, resolver results, deployment identifiers without secrets. | Choose a supported alternative within scope or stop for owner exception. | C01 and C02. |
| C02 | Authorized identity performs a bounded real request. Missing/expired identity, unavailable deployment, timeout, cancellation, and provider errors are visible. No secret enters source/logs. | Run live smoke test within approved limits and injected failure tests; inspect configuration and sanitized logs. | Authentication outcome, request ID, error/cancellation results. | Repair provider/auth handling; stop on missing access. | C02 and C03. |
| C03 | Only typed allowlisted tools run; malformed calls, disallowed data transmission, budget exhaustion, and unapproved effects are blocked outside the model. | Test adversarial outputs, tool schemas, data boundaries, routing, and enforced request/step limits. | Guard tests and operation counters. | Repair runtime policy; do not expand permissions to pass. | C03 and affected C02. |

## QUALITY

Model preferences are not availability guarantees. Separate model proposals from
application authority. Never fall back silently to a different paid model.
Document actual limits and observed latency without invented targets.

## CONTEXT

Required: PRD, authorized Foundry configuration and owner-approved limits.
Unresolved: tenant, endpoint, deployment IDs, auth method, allowable transmitted
data, model routing, live-call budget, and latency acceptance threshold.

## CONSTRAINTS

Inherit shared contract. No access-token extraction from this chat or other apps.
No workplace/MCP access or unrestricted shell tools. Credentials use an approved
secure mechanism selected before implementation.

## STAGES

1. Bind connection/data policy and compare supported runtime options.
2. Implement provider and bounded tool interfaces.
3. Verify authorized calls and failure/safety cases.

## STOP-CAPS

Inherit shared caps; live-call counts must be approved before testing.
Example repair: malformed tool arguments reach an adapter; validate before
dispatch and rerun C03 and dependent provider tests.
First-run learning: record request counts/latency to calibrate routing and caps.
