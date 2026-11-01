# AI change proposals

Ask prompts that ask to organize, rename, move, copy or delete files (Czech or
English, for example "Utřiď Downloads podle typu do podsložek" or "přejmenuj
fotky v D:\Foto\2026 podle data ve formátu YYYY-MM-DD_nazev") use the same
read-only [file harness](ai-search.md). The model inspects only what it needs
and may end the run with one `propose_changes` call. **The model only proposes:
nothing runs until the user selects changes and approves Run, and every step is
then an ordinary operation through the existing planners, guards and journal.**
The older narrow command pilot ([commands](command-execution.md)) is unchanged
and not used for this route.

## Tool contract

`propose_changes` (non-strict function schema; the validator is the contract):
`summary` in the user's language and `operations`, an ordered list of at most
500 items, each with a short `reason`:

| `op` | Fields | Result |
|---|---|---|
| `new_folder` | `parent`, `name`, optional `ref` | Empty folder; the n-th `new_folder` is ref `fn` (`f1`, `f2`, ...) |
| `move` / `copy` | `source`, `destination` | Item placed into the destination folder |
| `rename` | `source`, `name` | Same folder, new name |
| `recycle` | `source` | Item sent to the Recycle Bin |

`source` is always a numbered real-file ID returned by a tool in this run,
never a path. `parent`/`destination` is an existing folder ID, an existing
folder path (absolute inside an approved root, or relative to exactly one
approved root) or the ref of an earlier `new_folder`. The model receives no
conflict policy: an existing target is an error (Stop on conflict).

## Validation (outside the model)

The contained worker validates the call against the run's observed files:
unknown IDs (including IDs from another run), IDs whose observed version
(identity, size, modification time) changed since observation, folders that
were replaced, approved roots themselves, paths outside approved roots,
links/junctions and protected credential paths (the same exclusions as reads),
invalid Windows names, case-only or unchanged renames, existing targets (no
overwrite), case-insensitive collisions between targets, moving or copying a
folder into itself or a descendant, the same item in two operations, an item
inside another operation's source, a parent/destination changed by another
operation, refs used before their `new_folder` or unknown, unknown fields and
more than 500 operations. Every problem (up to 20) is reported. An invalid
proposal ends the segment as *rejected*: nothing changes, the problems are
returned to the model as the tool result, and explicit Continue lets it fix
them under the normal ten-call/two-minute segment rules. A valid proposal is
answered "ready for review" and ends the run; nothing has changed.

The call may carry up to 256 KiB of arguments; the harness's provider response
ceiling is 512 KiB and its output ceiling 16,384 tokens, so very large
reorganizations may need to be split.

## Review

The bell shows *Proposal ready: N changes* without interrupting work; opening
it shows the review (not a modal that appears on its own). It shows the
summary and rows grouped into New folders, Moves, Copies, Renames and Deletes
(Recycle Bin), each `before → after` relative to the common folder when
shorter, with the reason on hover and every exact path and reason under
Details. Non-delete changes start checked; **deletes start unchecked** and are
marked. Unchecking a new folder unchecks and disables the changes that need it
(their rows say why); checking it again restores them.

Keyboard: Run has initial focus; arrows and Tab move focus, Space toggles the
focused row, Enter confirms **only while Run has focus** (Enter on Cancel
cancels, on a row does nothing), a held or repeated Enter never confirms, and
Escape cancels. Cancel keeps the proposal: the bell reopens it.

## Execution

Run queues the selected steps (renumbering new-folder dependencies; a step
whose new folder is unselected cannot run) to the file worker:

1. **Check.** Every selected step is planned without effects, in order,
   simulating folders created by earlier steps; targets may not collide. Any
   problem (for example a source changed or a target appeared) is listed in the
   review and nothing starts. AI access settings must be unchanged.
2. **Run.** Each step is located again by identity inside the approved roots,
   planned again against the current state and executed like the manual
   action: new folder and rename use the name planner; move and copy use the
   copy/move planner with the Stop-on-conflict policy (cross-drive moves copy,
   verify and recycle as usual); recycle uses the Recycle Bin path with no
   permanent-delete fallback. Each step claims and finishes its own journal
   record with the shared `batch=ai-<key>` line.
3. **Stop.** The batch stops at the first failing step; cancellation (Cancel
   work) is checked before every step and also stops a running step like any
   operation. Completed steps stay done. Operations lists every step as DONE,
   FAILED (with the error and partial paths), CANCELLED or NOT RUN, and the
   panes refresh.

## Undo

Each step is an ordinary journaled record, so Operations offers per-step Undo.
Steps of one batch are grouped, and **Undo batch** prepares a reviewed plan
that undoes every undoable step newest first. Before approval only the cheap
Operations checks run, because earlier undo steps change what later ones are
checked against (a created folder is empty again only after the items moved
into it have gone back). After approval each step's inverse is prepared with
the full undo validation just before it runs and journaled with `undo_of`; the
batch undo stops at the first refusal or failure and reports what was not
undone. Already undone steps are skipped; a step that cannot be undone refuses
the whole batch undo (undo the others one by one).

## Usage log

`ai_proposal` records operation counts by kind, checked changes, unchecked
deletes, failed checks and the outcome (`run`/`cancelled`) when the review
closes. `ai_batch` records steps, how many ran and finished, the failed step
and the duration. No summaries or reasons are logged.

## Limits

The model sees only what its read-only tools returned; a proposal can still be
wrong, and review is the safeguard. Validation and checks use identity, size
and modification time, not cryptographic hashes. Steps are not atomic as a
whole: a failure leaves earlier steps done (and undoable). Folder targets are
compared by identity only, because earlier steps fill them.
