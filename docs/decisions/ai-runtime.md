# Foundry connection and bounded AI runtime

Date: 2026-10-08. Status: bounded native pilot verified against its owner-approved
AI-runtime performance gate. Broader application/hardware acceptance is
separate. Scope: AI-01 only.

## Decision and authorization

Use the owner's existing, verified `gpt-6-luna` deployment, not the unavailable
proposed `gpt-6.1-luna`. No deployment, resource, key, role, consent or subscription
change is part of this implementation. A separately configured `gpt-6-sol`
deployment is checked read-only; its inference route is blocked. There is no
automatic fallback, retry, model selection or agent loop.

Use a small typed Rust runner, serde JSON validation and direct Azure OpenAI
v1 `POST /openai/v1/responses`, not Copilot SDK, Foundry Agent Service or another
agent harness. AI-01 needs one explicit text request, auth, cancellation,
bounded parsing and policy enforcement. Persisted conversations, autonomous
planning, MCP and shell tools do not justify their overhead here.

The actual provider implementation is `src\ai.rs` and `src\ai\windows.rs`.
Native presentation is `src\ai_ui.rs`, wired into Ledger's header and palette.
The filesystem, operation approval, journal and search modules do not depend
on AI. Model text is displayed as untrusted plain text; it is never interpreted
as a path, command, approval or executable instruction.

## API and authentication

The official Responses guide lists the selected model/version and Sweden
Central support. The v1 lifecycle removes the dated API-version requirement;
the REST reference documents `store`, output token limits, usage and
`apim-request-id`. These are documented capabilities, not a guarantee of
availability or permission in any individual account.

Use the standard Azure CLI MSI installation's Python entry point
(`python.exe -IBm azure.cli`) with separate `Command` arguments. This executes
the installed CLI without `cmd.exe`, PowerShell, PATH-resolved shims or shell
interpolation. No token is in a process argument or environment variable.
CLI telemetry and dynamic extension installation are disabled for these reads.
The app does not automatically install the CLI, log in, switch accounts,
request interactive credentials, change permissions or retrieve resource keys.
An authentication failure instructs the user to sign in explicitly.

On a cold connection or explicit Check, read the specifically configured
subscription's account, resource and deployments, projecting only the configured
primary/stronger names and validation fields. Require the expected user identity, tenant,
enabled subscription, AzureCloud, AIServices resource, exact endpoint and
successful OpenAI deployments. These are targeted reads, not inventory scans.
Expected underlying model names are bound separately from deployment names and
checked too; renaming/replacing a deployment cannot silently change model routes.
Then acquire a CLI token for `https://cognitiveservices.azure.com/`,
checking its tenant, subscription and expiry margin.
An exact configuration/path-bound worker-memory cache reuses that proof/token
for at most 60 seconds and only while token expiry exceeds the full request
deadline plus 60 seconds. Prompt text is not part of or stored in this cache.
Check always forces a fresh proof/token acquisition. Configuration changes,
Disconnect, panel/app close, cancellation, deadline, errors and 60 seconds idle
clear/terminate the session; the UI controller checks idle expiry every second.
Continuous activity cannot extend proof freshness. Azure CLI logout or account/
deployment changes outside the app are not instantly detected within this
window: use Disconnect or Check after such a change. Returned response model
names must match the verified primary name or its verified version-qualified
name; no mismatching model answer is accepted.
The CLI rejects simultaneous `--tenant` and `--subscription` token arguments.
Use the explicitly verified subscription argument and validate the returned
tenant, rather than switching CLI state or inferring identity from a model name.
The REST reference specifies this Cognitive Services scope; the newer guide
also shows `https://ai.azure.com/.default`. The implementation deliberately
uses the documented REST Cognitive Services audience, not an inferred scope.
Management-plane deployment visibility is not proof of inference permission;
only an actual successful request establishes that.

Tokens handled by the app exist only in worker memory and its CLI pipe.
Azure CLI manages its own standard sign-in cache; the app neither reads that
credential store directly nor copies it into application state. Owned raw token buffers
and UTF-16 authorization buffers use volatile clearing on normal drop; no
secret has a Debug implementation, and raw CLI/provider errors are suppressed.
Parser temporaries, operating-system/WinHTTP buffers and process crash dumps
cannot be promised to be securely erased by Rust, including forced process
termination. The bounded cache is worker-memory-only: no token file, plaintext
API-key setting or reuse of another app's credentials.

WinHTTP uses Windows TLS validation and automatic system proxy discovery.
The host is constructed solely from a validated resource DNS label and the
fixed `.cognitiveservices.azure.com` suffix, then checked against the actual
resource endpoint. No arbitrary endpoint URL, userinfo, path, port or redirect
can receive the token. Redirects, cookies and ambient HTTP authentication are
disabled; TLS 1.2/1.3 are explicitly selected and no certificate validation is
disabled. A response-header timeout is bound separately from WinHTTP's receive
timeout. Its notification can lag when no data arrives, so the independent
parent deadline, not a socket timer alone, is the overall wall-clock bound.

## Cancellation and process boundary

Blocking authentication/WinHTTP calls run in a bounded session instance of the
same executable, before renderer initialization. A Rust background controller
owns that worker in a Windows Job Object with kill-on-close. The worker waits
for its bounded newline-framed stdin job until containment is assigned;
request/result frames have independent byte ceilings and fail on incomplete
frames or malformed/deep/unknown-field JSON. Azure CLI descendants
inherit containment. Parent cancellation/deadline closes the job and confirms
worker termination. There is no unsafe cross-thread close of a synchronous
WinHTTP request handle and no async runtime.

Cancelling transport cannot cancel billing for an already submitted request.
An uncertain outcome is reported explicitly and never retried. A request
attempt is durably reserved before POST; a terminated/failed transport preserves
a blocking spending lease. Inspect the record and establish actual state before
owner-approved recovery; do not delete it or reset counters automatically.
Connection checks acquire a token but send no inference request.

## Data and limits

Only the user's explicitly submitted prompt and fixed application instructions
are transmitted. No pane path, selection, filename, content, history, private
code, workplace material, credential, configuration identity or conversation
history is added to the request. The UI requires prompt-only confirmation,
cleared after editing. Typed metadata/content/workplace/credential inputs are
denied; obvious credential markers are rejected. This is not a classifier
capable of guaranteeing that arbitrary pasted text is non-private: the user
must not paste prohibited data.

`store=false`, `background=false`, `stream=false`, an empty tools array and
`tool_choice=none` make each request standalone. This disables Responses
storage, not Azure's service-side abuse-monitoring or other platform retention.
Unexpected tool/action outputs are rejected. The bounded typed dispatch
scaffold allows only `runtime_capabilities` with an empty object and returns
static capability text. It is intentionally not advertised to the model;
effects and further model calls remain blocked.

| Limit | Pilot default and hard ceiling |
|---|---|
| Prompt | 2,048 UTF-8 bytes |
| Input tokens | 3,072; preflight uses bytes + fixed instruction bytes as a conservative text-token bound, then verifies reported usage |
| Output | 512 tokens and 8,192 text bytes |
| Provider response | 65,536 bytes; bounded JSON parser depth |
| Inference attempts | 5 durable lifetime pilot attempts, shared by UI and CLI; includes uncertain/failed submissions |
| Tool steps | At most 4, including rejected dispatches; no model-driven loop |
| Time | 60-second overall worker deadline; 5-second DNS/connect/send stage bounds |
| Concurrent work | One UI job, bounded result channel; spending lease serializes POST attempts |

Settings can reduce these ceilings, never increase them. Token usage is not
a currency budget: the app does not estimate prices or claim a spending amount.
These conservative runtime defaults are distinct from the subsequently
owner-approved, scoped measurement contract in the AI-01 card. More calls,
stronger inference or expanded data scope
require explicit approval, not a silent counter reset.

Connection configuration is nonsecret and user-local:
`%LOCALAPPDATA%\TomasCommander\ai-connection.json`. Attempt count and lease are
`ai-budget.json` and `ai-budget.lock` beside it. The generic repository has no
resource-specific identity, subscription or endpoint defaults. Existing
preferences remain compatible. Prompts and answers are not persisted.
Invalid configuration, state transaction, expiry, mismatch, deployment,
HTTP error, timeout, cancellation, output shape and budget errors are visible.

## Dependencies and alternatives

Existing effective Cargo source has no project/user override or registry
environment configuration. The coordinator approved offline use of the existing
cached source, not a new public download/source override.
`cargo check --offline --all-targets` resolved cached serde_json 1.0.151,
itoa 1.0.18 and zmij 1.0.23. Serde/derive/core 1.0.229 were already locked.
An initial 1.0.228 pin was unavailable in the cache and was corrected to the
existing locked version; no network retry or feed bypass occurred.
`Cargo.lock` retained all existing versions and added only those three packages.
The final Windows target inventory has 124 unique package/version entries using
`cargo tree --no-dedupe` followed by package/version deduplication. This method
avoids counting repeated `(*)` tree markers as extra packages; it is not a
like-for-like comparison with older inventory counts using a different method.

Serde supplies typed, deny-unknown-fields configuration/job decoding.
Serde_json supplies proper escaping, parsing and bounded-depth validation,
instead of handwritten JSON or subprocess HTTP interpolation.
Existing windows/windows-core 0.62.2 gain WinHTTP, Job Object, Threading and
Security bindings; transport/TLS/process containment use OS facilities.
No HTTP SDK, TLS crate, tokio, webview, JavaScript or agent framework was added.
Four CLI process launches on a cold connection add measured auth latency.
The prior runner repeated five launches on every operation; the repair combines
the two deployment validations into one targeted resource-scoped list read and
reuses a contained worker/token briefly. Three independent nonbillable final
release checks measured cold authentication at 17,697 / 21,603 / 21,879 ms;
nine immediate verified-session checks measured authentication at 0 ms and
controller elapsed at 0-2 ms (millisecond resolution, not zero CPU work).
Cold account, resource, deployment and token stages are recorded separately.
This removes repeated startup/management costs from an immediate Send without
claiming to accelerate the model or an expired/cold connection.

An Azure Identity SDK would remove the installed CLI dependency but adds
token-cache/login/runtime integration and approved-source requirements.
Async WinHTTP would avoid worker-process overhead but needs callback-lifetime
and cancellation ownership. Reconsider these if measured overhead fails
owner-bound thresholds, not merely because an SDK or harness exists.

## Validation and evidence

Commands: `cargo test --locked --offline`,
`cargo clippy --locked --offline --all-targets -- -D warnings`,
`cargo fmt --all -- --check`, `cargo build --release --locked --offline`.
The same executable exposes `--ai-check` (no inference),
`--ai-check-session` (one forced cold check and three memory-only proof reuses,
no inference), and `--ai-smoke`
(one fixed harmless synthetic prompt); neither accepts arbitrary prompt text
from command arguments. Both use the production provider and shared durable
budget. Native checks must exercise real Check/Send/Cancel controls.

Unit checks cover injected malformed/expired identity, account/endpoint
mismatch, unavailable deployment, HTTP failures, redirect rejection,
token/byte limits, malformed JSON/tool outputs, denied data/effects/routes,
step exhaustion, durable request exhaustion and actual contained-process
cancellation/deadline, actual descendant termination, cache key/expiry/forced
check/failure invalidation, persistent frame reuse/idle/disconnect, interrupted
spending leases/transactions and native pending/resubmission/cancel/focus wiring.
Real WinHTTP tests use only controlled loopback fixtures under `cfg(test)`:
invalid TLS, blocked redirect and delayed response. They have no Entra token,
model call, production endpoint override or relaxed production TLS policy.
WinHTTP's nominal one-second receive timeout did not reject a 1.5-second
fixture on this host; a four-second delayed fixture did fail. The parent
deadline/cancellation tests remain the strict independent wall-clock evidence.
Test doubles do not replace real inference evidence.
Only named synthetic temporary state is created and cleaned up.

Run identity: `AI-01-20261008-foundry-pilot`.
Sanitized status, final source/build hashes, actual usage/request IDs and
native observations belong in the bound shared session evidence directory.
Auth, provider wait and local guard/parse timing are reported separately.
The owner subsequently bound the exact AI-only workload/ceilings and permitted
reuse of the two earlier provider receipts plus one final native request.
The repaired build's native request completed in 3,600 ms, with 0 ms cached auth,
3,530 ms provider time and 319 us guards/parsing. Actual native nonbillable
Check cancellation and separate real worker/descendant cancellation passed.
The scoped C04-PERF results and coarse memory-sampling limits are recorded in
the AI-01 card and `AI-01-scoped-performance.json`; neither claims full product
startup, physical keyboard or DPI acceptance.

## Primary-source evidence manifest

Public official documentation only; no workplace or community research was
required to choose these API/auth/process signatures.

| Source | Retrieved via / result | Supports |
|---|---|---|
| [Responses guide](https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/responses), updated 2026-08-18 | web_fetch, success; relevant prerequisites/model/text examples read | Selected model, region, direct REST and Entra examples |
| [v1 lifecycle](https://learn.microsoft.com/en-us/azure/foundry/openai/api-version-lifecycle), updated 2026-06-05 | web_fetch, success | v1 path/version and identity requirements |
| [Responses REST reference](https://learn.microsoft.com/en-us/rest/api/microsoft-foundry/azureopenai/responses), updated 2026-07-09 | web_fetch, success; Create response section read | OAuth scope, body fields, output/usage/request-ID shape |
| [Azure CLI account reference](https://learn.microsoft.com/en-us/cli/azure/account?view=azure-cli-latest#az-account-get-access-token) | web_fetch, initial reference plus token argument section; installed CLI help/wrapper and actual adapter additionally checked | Standard CLI token mechanism and explicit subscription/tenant arguments |
| [WinHttpCloseHandle](https://learn.microsoft.com/en-us/windows/win32/api/winhttp/nf-winhttp-winhttpclosehandle), updated 2024-02-22 | web_fetch, relevant remarks read | Async close/lifetime complexity; avoid closing sync handles across threads |
| [Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects), updated 2025-07-16 | web_fetch, relevant creation/descendant/termination sections read | Worker/descendant containment and termination |
| Learn search and code-sample search | Both attempted, blocked: cached MCP catalog changed/no native execution backend | No successful sample-search claimed; fetched official guide includes actual REST/code examples |
| Old Responses create reference URL | web_fetch, 404 | Unused; replaced by current reference above |
| Existing Cargo source/cache | Local configuration/cache inspection and offline resolver, success | Actual package availability, not upstream latest availability |
| [WinHTTP options](https://learn.microsoft.com/en-us/windows/win32/winhttp/option-flags), updated 2026-03-27 | web_fetch, relevant timeout/redirect/TLS sections read; follow-up Learn search succeeded | Separate response-header timer, notification limitation, redirect policy and TLS 1.2/1.3 |
| [WinHttpSetTimeouts](https://learn.microsoft.com/en-us/windows/win32/api/winhttp/nf-winhttp-winhttpsettimeouts), updated 2024-02-22 | web_fetch, signature/parameters/remarks read | Stage timers are not a substitute for the parent deadline |
| Follow-up Learn code-sample search | Tool recovered; successful search, 20 returned snippets and zero relevant WinHTTP snippets | No unrelated sample used; signatures grounded in official API pages above |
