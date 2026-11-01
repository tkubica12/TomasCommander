# Local usage log

Tomas Commander keeps a local log of how it is used so that the owner can
periodically review real behaviour, alone or with an AI assistant, and find
improvements: commands still invoked by mouse although they have a shortcut,
keys pressed in vain, Ask texts that found nothing, dialogs that are mostly
cancelled, folders reached by long routes, errors and slow work.

The log is **local only**. Nothing is uploaded, synchronized or sent anywhere
automatically. It deliberately contains full folder and file paths and Ask
texts; it is meant for the owner's own analysis.

## Storage, retention and caps

| Item | Policy |
|---|---|
| Location | `usage\` beside the preferences file: `%LOCALAPPDATA%\TomasCommander\usage\` by default; `--settings <file>` moves it beside that file. |
| Files | One JSON Lines file per local day, `YYYY-MM-DD.jsonl`; exported reports are `report-YYYY-MM-DD.md`. |
| Retention | When the app starts, the writer deletes `YYYY-MM-DD.jsonl` files older than 90 days. Other files are never touched. |
| Caps | 20 MB per day file: further events of that day are not written and one `cap_reached` line records it. 200 MB for all day files: at startup the oldest earlier days are deleted until the total fits; during a session writing stops with one `cap_reached`. |
| Links | A usage folder or day file that is a link/junction is refused: nothing is written, deleted or followed, and Settings shows the problem. |
| Disabled runs | Fixture runs (`--fixture-root`) keep no log unless `--settings` names their own settings file. Unit and egui tests never log unless a test opts in with an owned temporary folder. |
| Toggle | Settings > Usage log; on by default. Stored as `usage_log=` in preferences v7 (v1-v6 stay readable and mean on). |

The toggle lives in preferences rather than a separate file because it is an
ordinary user setting saved with the others, follows `--settings` and needs no
second migration path; deleting usage data deliberately keeps it.

**Delete usage data** (Settings > Usage log, with the usual confirmation) runs
on the writer thread: it closes the current file and deletes only
`YYYY-MM-DD.jsonl` and `report-YYYY-MM-DD.md` files that are regular files in
the usage folder. Recording continues into a new file if the log stays on.

## Line format

Every line is one JSON object:

```json
{"t":1793448000123,"tz":60,"s":"8f3a...","k":"command","cmd":"/copy","via":"shortcut","ok":true,"shortcut":"Ctrl Shift C","surface":"files","pane":0,"tab":0,"tabs":1,"selected":3,"kind":"file","ext":"pdf","focused":"D:\\Docs\\a.pdf"}
```

`t` is Unix time in milliseconds (UTC), `tz` the local UTC offset in minutes,
`s` a random session id, `k` the event kind. Field names are stable.

| Kind | Fields | Recorded when |
|---|---|---|
| `session_start` | `version`, `window_w`, `window_h`, `zoom`, `theme`, `accent`, `scale`, `native_scale`, `root_arg`, `fixture`, `settings_arg`, `startup_ms`, `tabs_left`, `tabs_right`, `places` | first frame |
| `session_end` | `duration_ms`, `active_ms`, `idle_ms` (no input for 5 min = idle), `dropped` | exit |
| `summary` | `period_ms`, `frames`, `slow_frames`, `max_frame_ms`, `active_ms`, `idle_ms`, `counts` (`key.<name>` keys pressed outside text fields, `typed.<focus>` typed characters, `pointer.<button>`/`pointer.wheel`, `folder.loads`, `zoom.wheel_steps`) | every 5 minutes and at exit |
| `command` | `cmd` (slash name), `via`, `ok`, `shortcut` (its keyboard shortcut, if any), `surface` (`files`/`zip`/`preview`/`rail`), `pane`, `tab`, `tabs`, `selected`, `kind` (`file`/`folder`/`zip`/`zip_member`/`none`), `ext`, `focused` (full path) | every dispatched command and equivalent direct actions (`/ask`, `/open`, `/open-zip`, `/parent` by Backspace or `..`, header sort, history buttons, tab strip, drag-and-drop, zoom keys/Ctrl+wheel, `/operations`, `/cancel-work`) |
| `key_unhandled` | `key` (for example `Ctrl+F5`, `F8`, `Delete`, `Ctrl+C`, `Shift+Delete`), `focus` (`pane`, `zip`, `rail`, `preview`, `filter`, `path`, `editor`, `dialog`, `ask`) | a Ctrl/Alt chord, function key or Escape/Delete/Insert press that no handler consumed; a held clipboard chord counts once per press, not per auto-repeat |
| `navigate` | `pane`, `tab`, `from`, `to`, `via`, `ms_since_last_nav` | the folder of a pane changed |
| `filter` | `pane`, `folder`, `query`, `matches`, `opened`, `outcome` (`opened`/`cleared`/`navigated`/`left`), `duration_ms` | once per filter session, when it ends |
| `search` | `root`, `query`, `results`, `duration_ms`, `truncated`, `ai`, `outcome`, `opened_rank` | when a search's result is opened, a new search starts or the dialog closes |
| `ask` | `text` (full), `outcome` (`command:/x`, `ai`, `no_match`, `invalid`, `cancelled`), `candidates` | Ask closes |
| `ai_task` | `kind`, `prompt` (full), `outcome` (`completed`/`clarify`/`capped`/`failed`/`cancelled`/`not_started`), `calls`, `results`, `duration_ms`, `continuation` | an AI file task ends |
| `ai_result` | `rank`, `preview`, `path` | an AI result is opened |
| `dialog` | `name`, `outcome` (`confirmed`/`cancelled`/`closed`), `open_ms`, `extra` (for example `policy=Replace older changes=2`) | a tracked modal closes |
| `operation` | `op`, `items`, `dest`, `strategy`, `policy`, `conflicts`, `detail` (ZIP or undo label), `duration_ms`, `completed`, `created`, `replaced`, `merged`, `skipped`, `recycled`, `kept`, `incomplete`, `cancelled`, `error` | an approved operation (including undo, save, rename, new folder, ZIP) finishes |
| `preview` | `path`, `member`, `ext`, `kind`, `action` (`open`/`page`/`rotate`/`edit`), `open_ms`, `ok` | a reader load finishes |
| `zip` | `action` (`open`/`reload`/`member_open`), `path`, `ms`, `ok` | ZIP browsing |
| `editor_save` | `ext` | a source Save is approved |
| `open_with` | `choice` (`vscode`/`copilot_cli`/`copilot_app`) | Open with choice |
| `external_drag` | `items`, `zip`, `alt`, `result` (`accepted`/`cancelled`/`error`) | Copy-only drag to another application |
| `tab` | `action` (`new`/`close`/`switch`/`next`/`previous`), `via`, `pane`, `count` | tab changes |
| `recent_rail` | `action`, `via` | rail open/close/tabs, Recent and Places actions |
| `settings_change` | `key` (`theme`, `accent`, `zoom`, `places`, `recent_roots`, `usage_log`), `value` | a preference changes |
| `error` | `message`, `context` (last command), `dialog` | a user-visible error |
| `slow_frame` | `ms`, `context` (dialog, preview kind, loading, drag, entry-count bucket) | UI work of a frame over 50 ms; at most one per second |
| `slow_folder` | `path`, `entries`, `ms` | a folder listing took over 300 ms |
| `slow_action` | `cmd`, `ms` | preparing an operation kept the worker busy over 500 ms |
| `cap_reached` | `scope` (`file`/`total`), `limit_bytes` | a cap stopped writing |

`via` values: `shortcut`; mouse: `button`, `context_menu`, `click`,
`double_click`, `drag`, `rail`, `tab_strip`, `header`, `mouse_button`,
`wheel`; Ask: `palette` (picked from the list), `slash` (typed `/command`).
Navigation `via`: `enter`, `double_click`, `backspace`, `parent`, `return`,
`breadcrumb`, `place`, `recent`, `drive`, `history_back`, `history_forward`,
`typed_path`, `tab_switch`, `search_result`, `ai_result`, `command_result`,
`other`.

### Never recorded

File contents, editor text, clipboard contents, AI answers or interpretations,
credentials, endpoints, keys and other AI connection values, and environment
variables. Plain typing is never logged per key: characters are only counted in
`summary`. AI connection changes appear only as the `foundry` dialog opening
and closing.

## Performance design

- Recording runs on the UI thread but only builds one small event and calls
  `try_send` on a bounded channel (4096 events). A full channel increments a
  dropped counter; it never blocks, waits or touches the disk.
- A disabled log checks one atomic flag; the event-building closure is not
  run, so nothing is allocated.
- One writer thread serializes JSON Lines into a 64 KB buffer and flushes every
  5 seconds or every 256 events. Retention, caps and deletion run there.
- Per-frame hooks are a few comparisons: input counting before the handlers,
  and unhandled keys, dialog/Ask/filter state, busy time and preference changes
  after them. Per-keystroke data is aggregated in memory.
- On exit `session_end` is recorded and the final flush waits at most 300 ms;
  exit never hangs.
- A unit test records 100,000 events from the caller side within 500 ms in a
  debug build; disabled recording of 1,000,000 events stays under 200 ms.

## Analysis

- **Usage insights** (`/usage`, Ask "Usage insights", or Settings > Usage log)
  computes a report for the last 7, 30 or 90 days on a background thread.
  **Export Markdown report** writes `usage\report-YYYY-MM-DD.md` and copies its
  path; **Open folder** opens the usage folder in Explorer.
- Command line, without opening a window:

  ```powershell
  tomas-commander.exe --usage-report [--days N] [--settings <preferences file>] [--out <report.md>]
  ```

  `--days` defaults to 30 (1-365). The report goes to `--out`, otherwise to
  standard output. Release builds are Windows-subsystem executables without a
  console, so prefer `--out` (redirecting standard output to a file or pipe
  also works). Errors go to standard error with exit code 1.
- The Markdown report explains itself ("How to read this") and contains:
  period and totals, commands with keyboard/mouse/Ask shares and shortcut
  opportunities, unhandled keys, Ask outcomes and unmet texts, dialogs by
  cancellation, errors, slow folders/actions/frames, most visited folders and
  their routes, Places candidates (folders reached at least twice at the end of
  3+ navigation steps less than 15 s apart), tabs and history, filters,
  recursive search, AI tasks, previews, operations, settings changes, input
  counters and a pointer to the raw JSONL files. Malformed lines are counted
  and skipped.

## Limits

- Frame timing covers application UI work between the start of `logic` and
  the end of `ui`, not GPU rendering or input-to-display latency.
- While the window is minimized, eframe still calls `logic` but runs no egui
  pass, so the input it sees is that of the last shown frame. Input is counted
  only for a pass with a new input time; minimized time without input becomes
  idle.
- Unhandled-key detection sees keys that no handler consumed in that frame.
  Text editors and selectable previews read editing keys without consuming
  them, so Ctrl+A/C/V/X/Z/Y, Ctrl+arrows, Delete, Escape, Enter and Tab are not
  reported while they are focused; Alt+F4 and AltGr combinations are ignored.
- Clipboard chords reach the app as bare Copy/Cut/Paste events without their
  key, so the chord is inferred from the held modifiers: Shift+Delete and
  Shift+Insert are told apart from Ctrl+X and Ctrl+V, but Ctrl+Insert is
  reported as Ctrl+C. A press ends when its key or a modifier is released.
- AI connection settings, connected answers and the natural-language file
  command are recorded only as dialogs.
- Times are local wall-clock time with the current Windows time-zone offset.
