# TomasCommander

Status: initial product brief. The first implementation milestone is mock UI
exploration, not a working filesystem or AI integration.

## Product purpose

A Windows desktop workspace inspired by Total Commander and Norton Commander.
It is an application over Windows, not a replacement operating system.
Interaction priorities are keyboard first, command/voice second, mouse third.
Every input method should invoke the same underlying commands.

## Agreed requirements

- Extremely lightweight and fast, with as few dependencies as practical.
  Rust-first production architecture: put filesystem, search/indexing, command
  execution, safety checks, and AI/MCP orchestration in Rust wherever practical.
  JavaScript must not become the production business-logic layer.
- Prefer a Rust-native UI when it meets interaction/accessibility needs and
  measured resource goals. A thin web presentation over a Rust core remains an
  option only after comparing its actual overhead and obtaining owner agreement.
  Do not select a JavaScript-heavy shell or framework by default.
- Prove performance through reproducible measurements, not language choice.
  Measure cold startup, idle memory/CPU, input-to-render responsiveness, and
  filesystem/search behavior against an agreed workload and numeric limits.
  Network/model latency must be reported separately from local application work.
- Two file panes with browsing, single and multiple selection, copy, move,
  filtering, searching, and quickly accessible favorite locations.
- Modern, clean, terminal-inspired presentation using real graphical controls.
- One-click light/dark switching, grayscale structure, and one accent family.
  A simple visible Accent button cycles blue, red-orange, green, and yellow;
  preserve the selected family when switching themes. Use the canonical palette
  from the user's html-docs skill.
- Final delivery is a standalone Windows desktop application, not a browser
  page or a localhost service the user must open in a browser. Prefer a native
  Rust renderer and Rust core. A web-view renderer is not the default and needs
  explicit owner agreement after measuring its overhead and explaining why a
  native option cannot meet the agreed needs.
- No required function keys, Z/Y shortcuts, or punctuation shortcuts.
- Ctrl+Shift+P opens the command palette.
- Clear keyboard focus, visible active pane, discoverable command hints, and
  mouse access to commands. Shortcuts should eventually be remappable.
- Open a folder in VS Code. Investigate starting a GitHub Copilot App session;
  do not promise support until a supported integration is verified.
- AI-assisted file discovery from simple prompts, such as finding holiday images.
- Natural-language file commands with explicit approval before move or delete.
  Overwrites and other destructive effects must also require approval.
- Later, longer questions with a stronger model and connected sources such as
  WorkIQ and WebIQ. Answers should expose sources, short source summaries, and
  keyboard-navigable links.
- Voice is part of the product direction, but voice capture is outside the
  first mock UI milestone.

## First milestone: five interactive UI alternatives

Owner feedback, 2026-10-07: choose alternative 02, Ledger, as the direction for
the real application. This is enough visual exploration for now; do not generate
more five-design batches. Tune the real app later.

Ledger's left rail is a reusable contextual panel, initially favorites. It may
show other content for other use cases; mode changes must be explicit,
keyboard-accessible, and preserve focus. It is not hardwired to pinned locations.

The recommended Rust port is recorded in `docs\decisions\desktop-shell.md`.
Architecture recommendation is egui/eframe with an explicitly lean feature set,
subject to approved dependency resolution and real native performance checks.

Create two Commander-dense designs, two modern terminal-inspired designs, and
one minimal design. Keep their dataset and behavior identical so comparison is
about presentation and interaction, not unequal feature coverage.

All alternatives use shared in-memory mock files and a shared command registry.
Copy, move, delete, search, AI responses, and external application actions are
simulations, visibly labeled where relevant. No real filesystem access,
Foundry requests, microphone capture, or MCP connections.

Include browsing, selections, filtering, favorites, copy/move/delete previews,
sorting, pane resizing, the command palette, and example AI result states.
Include idle, loading, empty, error, approval, cancellation, and completion
states where the simulated workflow requires them.

Present one batch for review. The user chooses a direction or gives changes.
Do not automatically generate additional five-design batches while waiting.
Separate visual selection from final approval of an iterated design.

## Presentation and architecture

The implementation language preference is Rust, including UI where feasible.
Every production dependency needs a concrete capability justification. Prefer
standard-library functionality and maintained small crates over a large runtime
or agent framework when they satisfy requirements. No dependency should be added
merely for convenience without considering startup, memory, packaging, and
maintenance cost. Preserve responsiveness during directory scans and model calls.

The browser UI lab is a separate design artifact, not the production shell or a
performance benchmark. Its mock JavaScript is disposable and must not be promoted
into production services. Choosing a visual design does not choose its renderer.

The UI is a replaceable presentation layer over shared application state and
typed commands. Views must not implement their own filesystem or AI behavior.
Use real semantic controls; terminal styling does not justify inaccessible
canvas-only interactions or decorative text posing as controls.

Proposed layers:

1. Presentation: views, themes, focus management, and input bindings.
2. Application: command registry, pane state, selection, and operation plans.
3. Services: mock adapters first; filesystem, launch, AI, voice, and MCP later.
4. Safety: path validation, permissions, conflict handling, approvals, and
   operation read-back independent of model output.

Do not choose a desktop framework or add a larger agent harness solely for
mock UI exploration. The product shell decision should consider Windows
integration, keyboard behavior, accessibility, packaging, and maintained APIs.

## Proposed prototype shortcuts

These are pilot bindings, not verified behavior. Validate on actual CZ and US
keyboard layouts and preserve standard text editing inside input fields.

| Action | Binding |
|---|---|
| Command palette | Ctrl+Shift+P |
| Navigate rows | Up/Down |
| Switch panes | Tab; Shift+Tab reverses focus traversal |
| Open item / parent folder | Enter / Backspace |
| Toggle selection | Space |
| Extend selection | Shift+Up/Down |
| Select all in active pane | Ctrl+A |
| Copy to opposite pane | Ctrl+Shift+C |
| Move to opposite pane | Ctrl+Shift+M |
| Delete with approval | Delete |
| Sort chooser | Ctrl+Shift+S |
| Resize pane divider | Ctrl+Shift+Left/Right in file-pane context |
| Cancel or close overlay | Escape |

Do not depend on Ctrl+Alt+arrows or Alt+Shift combinations because of possible
system shortcut and layout-switching conflicts. If a pilot binding conflicts,
record the conflict and propose a replacement rather than hiding the failure.

## Appearance defaults

Pilot defaults: English UI, system fonts, canonical blue accent, and synthetic
Czech/English filenames. Allow theme switching in every design.

Canonical accent pairs, light/dark:

| Family | Light | Dark |
|---|---|---|
| Blue | #006da0 | #00a4ef |
| Red-orange | #bc3a16 | #f25022 |
| Green | #4c7100 | #7fba00 |
| Yellow | #805b00 | #ffb900 |

Use only one active family at a time. Selection, focus, warnings, and errors
must remain distinguishable through text and structure, not color alone.

## Filesystem testing safety

Never perform destructive tests on existing user files.

Future integration tests create a dedicated test root with synthetic folders
and files and keep an inventory of owned fixtures. Resolve and validate paths
before mutation, including junction/symlink escape risks. Only test-owned
fixtures inside that root may be changed or removed.

Cleanup targets explicitly identified test-owned artifacts, not arbitrary
folders, broad paths, or wildcard recursive deletion. Preserve unrelated
files even if they appear inside a test location. Failed cleanup must be
reported, not treated as successful.

## AI strategy: provisional

Start with a small bounded runner exposing typed, allowlisted tools.
The model proposes an operation plan; application code validates and executes
it. Never treat model text as unrestricted shell instructions.

Use the cheapest suitable verified Foundry deployment for intent recognition
and candidate ranking. The requested Luna model is a preference, not verified
availability. Prefer local deterministic discovery before sending a bounded
candidate set to a model. Image-content understanding is a separate capability
from matching filenames and metadata.

Evaluate a larger harness or Copilot integration only when longer tasks need
its capabilities. Verify SDKs, authentication, deployment availability, and
approved package feeds before selecting APIs or dependencies.

Chat-session MCP tools and credentials do not automatically become app
integrations. The app needs its own authorized connection configuration.

## Decisions deferred until real integrations

- Desktop shell/framework and packaging.
- Rust-native UI versus a thin web view over Rust; owner-approved startup,
  memory, responsiveness, and workload thresholds on the target Windows PC.
- Foundry endpoint, available deployment IDs, tenant, and authentication for
  tomas@tomasonline.net. Do not put secrets in chat or source control.
- Search roots, excluded locations, indexing, and permitted metadata/content
  transmission.
- Measured latency targets and enforceable AI operation/cost limits.
- Filesystem conflict policy, Recycle Bin behavior, and recovery mechanisms.
- Voice privacy, capture controls, and speech provider.
- Supported Copilot App launch/session mechanism, if any.
- WorkIQ/WebIQ MCP endpoints, permissions, authentication, and data boundaries.

## Work contracts

All product milestones after UI exploration require real software and real
integrations. In-memory mocks, canned AI responses, fake search results, simulated
file operations, and disabled integration buttons cannot satisfy their acceptance
checks. Synthetic files are safe inputs for real filesystem operations, not a
substitute for those operations. Live AI/MCP/speech checks must exercise the
actual authorized provider through the application.

Missing access or an unsupported integration means BLOCKED, not a completed
feature. Removing or replacing a requested capability requires an explicit
owner-approved scope revision; an unavailable placeholder is not delivery.

The complete milestone inventory, dependencies, shared evidence rules, and
readiness status are in `docs\goals\README.md`. Separate Goal Cards cover UI
exploration, desktop integration, safe file management, the AI runtime,
AI-assisted search, natural-language actions, connected answers, and voice.

Only mock UI exploration is currently ready for implementation. The remaining
cards are bounded drafts; unresolved permissions and provider details are
explicit blockers, not permission to infer access or spending.

Do not add project-specific skills yet. Add `.agents\skills` procedures only
when a repeatable project workflow needs portable instructions.
