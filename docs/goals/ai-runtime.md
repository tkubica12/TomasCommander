# Goal Card: Foundry connection and bounded AI runtime

## OBJECTIVE

Card: AI-01 | Version: 3 | Readiness: Scoped bounded runtime pilot verified
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
Bound: existing authorized Foundry resource and intended Azure CLI identity;
owner selected existing `gpt-6-luna`, without deployment changes.
User-specific tenant/subscription/resource settings remain outside the repository.
Only explicit harmless prompt text may be submitted; no file/workplace data.
Stronger `gpt-6-sol` is configured separately, verified read-only and disabled
for inference. Initial live smoke is capped at five tiny requests including
retries; no automatic retry occurs. Runtime pilot defaults and API/auth/dependency
decisions are in `docs\decisions\ai-runtime.md`.
Owner accepted the AI-runtime-only pilot performance contract below and
authorized its exact final native sequence. Full-application startup,
physical CZ/US responsiveness and product-wide DPI/acceptance remain separate
APP-01/FILE-01 gates; this AI pilot does not claim them.

## IMPLEMENTATION BINDING

Run: `AI-01-20261008-foundry-pilot`. Source: `src\ai.rs`,
`src\ai\windows.rs`, `src\ai_ui.rs`, `src\app.rs`, `src\main.rs`.
Runtime: Windows Rust/egui native application, same-executable contained session worker,
Azure CLI Entra and WinHTTP v1 Responses; no proxy/harness.
Configuration: `%LOCALAPPDATA%\TomasCommander\ai-connection.json`;
shared durable pilot counter/lease in that directory. Prompt/answer never saved.
Fixtures: harmless synthetic prompt, owned synthetic budget-test directory,
injected parser/auth errors, contained sleeping subprocess for stop tests.
Commands: offline locked tests, strict Clippy, formatting check, release build;
production `--ai-check`/`--ai-check-session`/`--ai-smoke`; native computer-use Check/Send/Cancel.
Evidence: shared-contract directory, `AI-01-20261008-foundry-pilot-status.json`.
Five-call cap is cumulative for CLI/native tests; cancellation/deadline counts
any POST reservation and preserves an uncertain-outcome spending lease.
C04-PERF passes the explicitly owner-bound AI runtime pilot scope below.

## PILOT OBSERVATIONS

The production provider performed one authorized synthetic request successfully
through Azure CLI Entra and the configured primary deployment: 42 input and
10 output tokens, 17,669 ms total, of which 15,526 ms authentication and targeted
management reads, 1,996 ms provider transport/wait, and 231 us local guards/parsing.
These are one-run observations, not latency acceptance or a price estimate.
After both separately approved native tests below, the cumulative durable count
is 3/5; no automatic retry and no stronger inference occurred.

Native header/palette entry, configuration read-back, accessible prompt editing,
Send gating and confirmation invalidation after editing were observed on the
real release build. The initial approval request returned user unavailable;
subsequently the coordinator obtained specific approval for one native Send of
`Reply with exactly: Native Foundry pilot OK.`. That exact request succeeded on
then-current release SHA256 `E41DC6CCCA6254C5C22160B89FC15F268B2E7CC440B6AE9C0943391D2A04163F`.
Native read-back showed `Native Foundry pilot OK.`, 42 input/10 output tokens,
28,320 ms total, 25,035 ms authentication, 3,066 ms provider and 322 us local
guards/parsing. The pending controls and completed request/response IDs were
observed; no retry, extra Check click or actual in-flight Cancel click occurred.
Cancellation/deadline and failure evidence remain the injected guards and real
contained-worker tests, not a claimed native cancellation experiment.
These pre-cache receipts remain historical evidence, not proof of the repaired
build's native path. General physical matrices were not inferred PASS.

The reliability/performance repair adds a 60-second, exact-configuration,
worker-memory-only verification/token cache, forced fresh Check and explicit
Disconnect. Cancellation/error/config change/close/idle invalidation and
expiry safety are tested; no token is persisted. Repeated CLI startup and
management reads, not provider generation, explain the earlier auth delay.
Final repaired release SHA256
`6A8C40DDDCFF9250E92BA1F0C0932E9A8594E6E59219BADF9AE442E46B3CA1B0`
passes 67 default tests, strict all-target Clippy, formatting and release checks.
Three nonbillable production cold checks measured auth at 17,697 / 21,603 /
21,879 ms; nine immediate cache reuses measured auth at 0 ms and controller
elapsed at 0-2 ms. These are bounded pilot observations, not owner acceptance.
The owner selected **Approve this exact test sequence**: fresh Check, one
synthetic Luna Send, Disconnect, then a second nonbillable Check cancelled
while pending. All actions ran on that exact repaired build. Native read-back:
`Native Foundry pilot OK.`, 42 input/10 output tokens, 3,600 ms total,
0 ms cached authentication, 3,530 ms provider and 319 us local guards/parsing.
Disconnect reset consent/disabled Send; actual pending Check cancellation
recovered controls and left zero descendants, unchanged 3/5 count and no lease.
Escape restored the main controls. Owned windows were closed after read-back.
C01, C02 and C03 pass on the final production logic/build.

## OWNER-BOUND AI PILOT PERFORMANCE

The owner selected **Approve this scoped pilot gate** for this machine and
AI runtime only. Statistic is the maximum/every observation, not a percentile.
Frozen workload: same release/nonsecret connection, one operation at a time;
three independent cold connection checks each followed by three immediate
cached checks; one specifically approved final native harmless prompt.
The two earlier provider receipts are reused only as expressly approved.

| Metric | Owner ceiling | Observed |
|---|---|---|
| Each of 3 cold checks | 30 s | 17.810 / 21.705 / 22.028 s |
| All 9 cached checks | 250 ms | 0-2 ms, millisecond resolution |
| Local controller/UI completion overhead, excluding auth/provider | 500 ms | Cold 102-149 ms; native Check 340 ms; native Send 70 ms |
| Guards/parsing | 5 ms | Three approved receipts 231 / 322 / 319 us |
| Worker-tree cancellation | 500 ms | Actual worker/descendant fixture, including setup and exit query, finished in 250 ms; native nonbillable Check cancellation separately passed |
| Sampled native app + CLI-tree working set | 512 MiB | 218,824,704 bytes (208.7 MiB) |
| Provider transport/wait | 10 s | Three expressly scoped receipts 1.996 / 3.066 / 3.530 s |

Working-set sampling used the actual native app alongside a production
nonbillable CLI Check and its contained descendants, with targeted CIM
traversal and 200 ms sleeps. Seven samples had actual roughly 1.8-3.4 s
intervals; this is a sampled peak, not a guaranteed maximum or allocation cap.
A separate pending native app/worker probe used 40 nominal 250 ms samples.
Cancellation timing uses a real contained subprocess/descendant and includes
its final exit query; it is not a mock or physical click-latency claim.
No extra paid trials were performed to meet the gate.

Evidence: `AI-01-scoped-performance.json`, `AI-01-session-benchmark.json`,
`AI-01-native-plus-cli-memory.json`, `AI-01-cancel-final-test.txt` and
`AI-01-native-cache-final-request.json` in the implementation session artifacts;
the shared status record points to these exact files and source/build hashes.
Repeat the same nonbillable check/test procedure; any additional provider
request requires fresh specific approval and remaining durable budget.

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
