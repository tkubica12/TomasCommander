# Goal Card: Voice as a secondary input

## OBJECTIVE

Card: VOICE-01 | Version: 1 | Readiness: Draft - not ready to run
Mode: bounded repair loop | Owner: project owner

Provide intentional voice input for the command palette while preserving
keyboard-first and mouse-accessible workflows. Transcribed speech enters the
same command/planning pipeline; voice never bypasses approval.
Do not introduce always-listening capture by default.

## OUTPUT

Implement speech adapter and visible capture/transcript/error states at bound
source paths. Create `docs\decisions\voice-privacy.md` documenting capture mode,
provider, language support, audio destinations/retention, and limits.
Add consented sample/task evaluations and shared evidence.

## DONE WHEN

Mandatory C04-PERF from `README.md` applies: capture lifecycle, cancellation, and
dispatch are Rust-first wherever platform APIs allow. Justify any speech SDK
exception; measure local overhead separately from recognition-service latency.

| ID | Pass | Verify | Evidence | Failure response | Recheck |
|---|---|---|---|---|---|
| C01 | Capture begins only through the approved intentional control, is visibly indicated, and ends on stop/cancel. Microphone denial/unavailability is actionable; keyboard palette remains usable. | Test real Windows microphone workflow, cancellation, permission failure, and provider-disabled behavior. | Capture state observations, permission/error outcomes, keyboard task results. | Repair lifecycle or controls; unavailable microphone check blocks completion. | C01 and C03. |
| C02 | Owner-reviewed CZ/US-language utterance cases meet their agreed transcription/intent rules. Users can inspect/correct the transcript before dispatch; all destructive plans retain CMD-01 approval. | Run consented sample inventory and synthetic-file commands through the shared executor; compare transcript, intent, and exact effects. | Sample consent, expected/observed transcript and intent, approvals and read-back. | Repair language/configuration or transcript review flow. | C02 and CMD-01 affected checks. |
| C03 | Audio goes only to approved destinations and follows agreed retention. Cancellation stops capture/transmission; raw audio is absent from ordinary logs. No hidden continuous listener or duplicate dispatch. | Inspect payload/storage behavior and exercise cancellation/retry/lifecycle cases. | Sanitized routing, retention inspection, counters and unchanged-state assertions. | Stop unsafe capture; repair lifecycle/data controls. | C03 and C01/C02. |

## QUALITY

Language and microphone availability are distinct from keyboard layout.
Recognition uncertainty must be visible and correctable. A successful transcript
does not prove a safe or correct command (C02). Do not record people without
consent or turn raw recordings into routine evidence.

## CONTEXT

Required: CMD-01, Windows capture capability, consented evaluation inputs.
Unresolved: push-to-talk versus toggle, language inventory, local/cloud provider,
audio privacy/retention, recognition expectations, and call/latency limits.

## CONSTRAINTS

Inherit shared contract. No always-on microphone, background recording, automatic
speech-based destructive approval, or unapproved audio upload.
Tests use synthetic filesystem effects and consented audio only.

## STAGES

1. Bind capture/privacy policy and evaluation inventory.
2. Implement speech-to-palette adapter and transcript review.
3. Verify real capture lifecycle, recognition, and unchanged command safeguards.

## STOP-CAPS

Inherit shared caps and approved speech-service limits.
Example repair: cancelling leaves capture active; repair lifecycle disposal,
then rerun C01, C03, and relevant C02 command cases.
First-run learning: record language/noise failures without retaining raw audio
unless explicitly authorized.
