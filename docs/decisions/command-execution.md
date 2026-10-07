# Native command execution

Date: 2026-10-07. Deterministic slice only; AI planning is not connected.

The typed registry in `src\app.rs` exposes local commands through the native
Ctrl+Shift+P palette and actual buttons. Matching is a case-insensitive substring
of command titles. Arrows choose, Enter dispatches, Escape closes. There is no
natural-language inference, shell execution, or canned model response.

Supported actions: copy, move, recycle, refresh, favorites, pin folder, sort,
filter, balance panes, open VS Code, activity, theme, and accent. Theme/accent
commands make appearance controls accessible without using the mouse.
The palette's file commands use the same typed executor and modal approval as
the buttons; see [file operation policy](file-operations.md).

Text fields retain editing shortcuts. The egui 0.36 text-edit accessibility
bridge explicitly implements AccessKit SetValue requests for path, filter, and
palette fields, including cursor reset and filter revision. Native UI Automation
verifies exact editor-value read-back, not just successful invocation.

AI intents and multi-step plans remain BLOCKED on actual Foundry endpoint,
deployment, authentication, data boundary and cost/operation limits. The Penta
presentation/Kosik natural-language scenario is not yet delivered.
Copilot App sessions, WorkIQ/WebIQ/MCP and voice are likewise not deterministic
substitutes. Their Goal Card checks remain unpassed.
