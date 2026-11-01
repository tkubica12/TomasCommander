# Commands and intent

Ask (Ctrl+Space) dispatches typed allowlisted application commands. Text does not
become arbitrary shell code. Deterministic file commands use the same planners,
guards, workers and journal as keyboard/mouse actions.

The experimental Natural-language file command mode sends one confirmed
Czech/English prompt to the configured primary model. Candidate resolution is
local within the two pane roots; paths/candidates/contents are not transmitted.
Ambiguity asks for a choice. Unsupported/malformed outputs fail explicitly.

Supported pilot intents find, copy, move or recycle one file. They never grant
overwrite, merge, permanent deletion or effect approval, and a move stays on one
drive: cross-drive moves are refused at planning. A successful intent
builds the existing immutable exact-path review; Cancel has no file effect.
Execution still needs explicit confirmation and revalidated snapshots.

`command-scenarios.json` retains the frozen pilot scenarios. The configuration-
bound grant adds four exact CommandIntent attempts to the original five-attempt
history, with a cumulative ceiling of nine; it does not reset history or enable
general automation. No automatic retries/fallbacks.

Free-text Ask prompts that ask to organize, rename, move, copy or delete files
go to the read-only file harness instead, which can end with a reviewed
multi-step proposal ([AI change proposals](ai-changes.md)); its selected steps
use the same planners, Stop-on-conflict policy, workers and journal.

Developer launchers are separate explicit commands. VS Code receives the folder
as a lossless argument. Copilot App requests `copilot app` for that folder.
Copilot CLI requests `copilot --yolo` through the Windows terminal host, which
allows the external agent to act without its normal permission prompts.
Installed tools and handlers are required; failures are surfaced.

See [operation policy](file-operations.md) and [AI runtime](ai-runtime.md).
