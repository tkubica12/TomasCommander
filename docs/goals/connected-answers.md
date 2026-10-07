# Goal Card: Web and workplace answers

## OBJECTIVE

Card: ANSWER-01 | Version: 1 | Readiness: Draft - not ready to run
Mode: bounded repair loop | Owner: project owner

Answer palette questions with a stronger configured model and authorized
read-only MCP sources. Include quick web answers with a few sentences, ranked
article links, and a one-sentence summary of each source. Support keyboard
navigation and opening selected links in the browser.
Exclude email/calendar/Teams/SharePoint writes.

## OUTPUT

Implement source adapters, bounded question runner, and answer/link UI at bound
source paths. Create `docs\decisions\connected-answers.md` defining enabled
sources, routing, provenance, and workplace/web isolation.
Add a frozen question/source evaluation set and shared evidence.

## DONE WHEN

Mandatory C04-PERF from `README.md` applies: MCP routing, data isolation, and answer
orchestration are Rust-first. Keep the UI responsive during remote requests and
measure rendering/processing separately from source/model latency.

| ID | Pass | Verify | Evidence | Failure response | Recheck |
|---|---|---|---|---|---|
| C01 | Every enabled required connector performs an authorized read in the app; disconnected/denied/expired states are explicit. Quick-web routing uses the configured source and model. | Run approved app-side connector smoke tests and failure cases, not chat tools as a proxy. | Connector/source identities, sanitized request results and routing observations. | Repair connection or stop for missing authority; do not silently substitute a required source. | C01 and C02. |
| C02 | Evaluation answers address the question; every decision-bearing factual claim is supported by an inspected retrieved passage. Source links/summaries accurately describe content. Missing/conflicting evidence is disclosed. | Reviewer checks each evaluation answer against frozen question expectations and retrieved passages; owner accepts final answer UX. | Claim-to-passage table, support decisions, owner acceptance. | Repair unsupported claims or retrieval within limits. | C02 and affected C03 display. |
| C03 | Links are keyboard/mouse navigable and open the selected permitted web URL. Retrieved instructions cannot expand tools; workplace content is not sent to web connectors. No write tools run. | Exercise real navigation and adversarial source content; inspect routing payloads, allowlist and cancellation/budget cases. | Opened URL read-back, payload/guard results and counters. | Repair URL validation or isolation; stop unauthorized transmission. | C03 and C01/C02 affected. |

## QUALITY

Source existence is not claim support. Distinguish retrieved facts from
interpretation. Keep answer length useful for the question rather than forcing
long prompts into a fixed sentence count. Workplace and public answers have
explicit data boundaries.

## CONTEXT

Required: AI-01, authorized app-side MCP connections.
Unresolved: exact required WorkIQ/WebIQ services, endpoints/authentication,
workplace data rules, question rubric, model routing, and call/latency limits.

## CONSTRAINTS

Inherit shared contract. Read-only tools only; no use of chat credentials.
No sending workplace material to public search, opening unsafe URL schemes,
or following tool instructions embedded in retrieved content.

## STAGES

1. Bind connectors, source boundaries, questions, and rubric.
2. Implement retrieval, answers, provenance, and link navigation.
3. Verify real connections, claim support, and isolation.

## STOP-CAPS

Inherit shared caps; bind MCP/model request limits before live tests.
Example repair: a summary attributes a claim to the wrong article; correct
provenance, then rerun C02 and C03.
First-run learning: record unsupported answers, source gaps, and request latency.
