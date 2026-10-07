# Goal Card: AI-assisted file discovery

## OBJECTIVE

Card: SEARCH-01 | Version: 1 | Readiness: Draft - not ready to run
Mode: bounded repair loop | Owner: project owner

Find useful existing file candidates from simple Czech/English prompts such as
"images from Dovolene, Srbsko". Combine scoped deterministic discovery with
bounded cheap-model interpretation/ranking. Do not confuse filename/metadata
matching with visual understanding of image content.

## OUTPUT

Implement search service and keyboard-navigable results at bound source paths.
Create `docs\decisions\ai-search.md` defining scope, indexing, transmitted fields,
and content-analysis capability. Add a synthetic corpus and owner-reviewed
query/expected-result inventory; record evidence under the shared contract.

## DONE WHEN

Mandatory C04-PERF from `README.md` applies: discovery/indexing/ranking orchestration
is Rust-first; benchmark the agreed corpus, bounded payload creation, and
responsiveness during search separately from model latency.

| ID | Pass | Verify | Evidence | Failure response | Recheck |
|---|---|---|---|---|---|
| C01 | Every query in the frozen owner-reviewed corpus meets its agreed expected-result/ranking rule; returned paths exist and satisfy scope. No result is fabricated. | Run corpus evaluation against synthetic files and real bounded model requests; compare candidates and ranking with expected sets. | Corpus/version, per-query results, request counts and observed latency. | Repair discovery/ranking, not expected answers; missing rubric blocks execution. | C01 and C02/C03 affected. |
| C02 | Results are keyboard/mouse navigable with path and match explanation; empty, ambiguous, cancelled, unavailable-provider, and failed-search states are distinct. | Run actual UI task matrix; select/reveal fixture results and inject each state. | UI observations and path read-back. | Repair result/focus/error handling. | C02 and affected C01. |
| C03 | Discovery stays inside approved roots and excludes forbidden locations; only approved data reaches Foundry; requests obey caps. Search never mutates files. | Inspect observed payloads and test excluded roots, reparse points, large candidate sets, malicious filenames, and budget exhaustion. | Sanitized payload inspection, guard tests, mutation inventory. | Stop unsafe requests; repair scoping and minimization. | C03 and C01. |

## QUALITY

Report uncertainty and why a candidate matched. Empty results are acceptable only
for corpus queries whose expected result is empty. Preserve Czech diacritics.
Do not claim photos were visually understood without tested content analysis.

## CONTEXT

Required: FILE-01 discovery and AI-01 provider.
Unresolved: authorized roots/exclusions, metadata versus image/content analysis,
index freshness, evaluation queries/ranking thresholds, request/latency limits.
Bind these before live implementation.

## CONSTRAINTS

Inherit shared contract. Synthetic evaluation files only. No whole-disk scan,
background indexing, file upload, embeddings service, or persistent content store
without explicit authorization.

## STAGES

1. Bind corpus, search boundaries, and relevance checks.
2. Implement discovery, intent interpretation, and candidate ranking.
3. Verify real returned paths, UI behavior, and payload safety.

## STOP-CAPS

Inherit shared caps and owner-approved AI call limits.
Example repair: a Serbian holiday query misses a Czech synonym; repair query
interpretation, then rerun C01 and C03 without rewriting expected results.
First-run learning: categorize false positives/misses before adding indexing.
