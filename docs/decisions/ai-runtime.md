# Foundry runtime

## Read-only file harness

The primary-deployment file harness is distinct from the older frozen pilots
below. It uses the same verified identity/TLS transport, but explicitly approved
roots and contents may be sent via read-only tools. It has ten model calls and
a two-minute segment deadline, explicit continuation, no monetary/lifetime
spending ceiling, and technical context/output limits. Roots/configuration are
persisted separately. `ai-file-agent-usage.json` records cumulative attempts;
`ai-file-agent-calls.jsonl` records request IDs and actual token usage without
prompts, paths or contents. Old counters/grants are not reset. The existing
`ai-budget.lock` serializes both routes; an uncertain lease remains blocking.
No automatic retry, stronger-model fallback, shell or filesystem mutation.
See [AI file discovery](ai-search.md) for scope, formats and controls.

Ask uses Medium reasoning on the configured primary deployment. Headless file
requests accept `reasoning_effort` as `none`, `low` or `medium`; omitted values
default to `medium`. This supports explicit controlled comparisons, not an
automatic deployment switch. Model/tool timing excludes worker startup and
identity/resource verification; end-to-end latency must include both.

## Original bounded pilots

AI is a narrowly bounded pilot, not an autonomous general agent. The app uses a
typed Rust runner and Azure OpenAI v1 Responses over WinHTTP, not an agent SDK.
There is no automatic retry, fallback, shell-tool route or model-driven loop.

## Configuration and identity

Nonsecret configuration lives at
`%LOCALAPPDATA%\TomasCommander\ai-connection.json`. No repository-specific
identity/resource/endpoint or credential is shipped. The installed Azure CLI
must already be signed in; the app never installs it, logs in, changes accounts
or requests permissions.

The standard MSI CLI Python entry point is invoked with separate arguments.
Checks verify configured identity, tenant, subscription, resource endpoint and
deployment/model names, then acquire a Cognitive Services token. Tokens stay
in contained worker memory/pipes, never arguments, configuration or environment.
Proof/token reuse is configuration-bound and capped at 60 seconds; Check forces
fresh verification. Disconnect/close/cancel/errors clear the session.
Changes outside the app may not be detected inside that window.

WinHTTP retains Windows TLS/proxy validation. Only the verified resource label
and fixed Cognitive Services host can receive the token. Redirects, cookies
and ambient authentication are disabled; TLS controls are not bypassed.

## Request boundary

Only an explicitly confirmed prompt plus fixed instructions is transmitted.
Pane paths, selections, enumerated filenames, file contents and history are not
automatically sent. Obvious credential markers are rejected, but this is not a
guarantee that pasted text is nonprivate: do not paste private or secret data.

Responses use `store=false`, no background/streaming and no tools. This is not a
promise about provider-side monitoring/retention. Unexpected tools/output shapes
and mismatching models fail. The generic primary route is bound to the configured
Luna deployment; stronger inference is not a generic fallback.

Workers run as contained instances of the same EXE in a kill-on-close Windows
Job Object. Framing/parsing/response sizes are bounded. Cancellation/deadline
terminates the job; uncertain submissions can still be billed and are not retried.
Owned secret buffers are cleared on normal drop, but OS/parser/crash buffers
cannot be promised complete secure erasure.

## Hard pilot limits

| Limit | Ceiling |
|---|---|
| Prompt | 2,048 UTF-8 bytes |
| Input / output | 3,072 / 512 tokens |
| Returned text / provider frame | 8,192 / 65,536 bytes |
| Generic inference | Five durable lifetime attempts |
| Runtime | 60-second parent deadline; 5-second network stage bounds |
| Concurrency | One UI job; durable spending lease serializes requests |

Settings can reduce ceilings, not silently increase them. Existing command/answer
grants admit only frozen prompts and preserve the cumulative history; grant CLI
commands are not permission to expand scope or reset counters.
`ai-budget.json` and `ai-budget.lock` live beside the configuration.
An uncertain/terminated spending lease blocks further submissions until
deliberate actual-state reconciliation. No price/spend guarantee is claimed.

Search and file commands use local deterministic resolution after interpretation.
Model output is untrusted and never approves a filesystem effect.
Connected answers have a separate narrow source/stronger-model grant.
These pilots are not broad production acceptance.
