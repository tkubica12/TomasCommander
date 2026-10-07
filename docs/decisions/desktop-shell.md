# Desktop shell: native Rust Ledger

Date: 2026-10-07
Status: implemented and release-built native slice; real-window smoke passed.
Full APP-01 and measured-performance acceptance remain incomplete.
Owner direction: Ledger / alternative 02 is sufficient to proceed to the real
product. Stop additional visual batches and tune the native implementation later.

## Decision

Use a native Rust Windows application with egui for the UI and eframe for the
window/input/rendering integration. Recreate Ledger's layout, not its HTML/DOM.
Do not ship a browser, WebView, localhost server, or JavaScript production logic.

Here "native" means a real Windows executable with Rust-rendered controls and a
native window. egui does not use standard Win32 controls or promise stock Windows
appearance. This is a good fit for our deliberately custom terminal-like Ledger.

This selection is based on documented capabilities and product fit,
not a measured claim that egui is faster or smaller than every alternative.
The resolved implementation is eframe 0.36.2 with Glow and AccessKit.
Representative performance thresholds and the full viability gate remain open.

## Selected presentation

- Ledger's dense two-pane workspace, location rail, flat rows, visible command
  strip, obvious focus, and restrained grayscale structure.
- Convert the left rail into a contextual panel. Favorites is its first mode,
  not its permanent meaning. Future modes can show search scope/results,
  operation progress, sources, or task context.
- Keep panel modes explicit and keyboard-accessible. Do not silently repurpose
  the panel while the user is navigating it; preserve focus and appropriate
  per-mode state.
- Keep a real command palette, source/destination previews, remappable bindings,
  one-click light/dark switching, and the four canonical configurable accents.
- Layout selection is not acceptance of the prototype's mock functionality.
  All product features retain their real-integration Goal Card checks.

## Options considered

| Option | Evidence and fit | Decision |
|---|---|---|
| egui + eframe | Rust UI; Windows integration; custom styling; panels/scroll areas; AccessKit. Immediate-mode UI maps well to a command-driven workbench. egui has few dependencies, but its own README explicitly says eframe has many transitive dependencies. | First implementation candidate. Reduce enabled features and measure the actual executable. |
| Iced | Rust UI with explicit state/messages/update/view and concurrent tasks. MIT-licensed. Its own documentation describes it as experimental. | Viable reserve option, not rejected as slow. Less direct reason to choose it over egui for this dense custom workbench; no head-to-head benchmark performed. |
| Slint | Native compiled declarative UI, Rust business logic, configurable GPU/software renderers, stable 1.x API. Adds a UI DSL and a licensing choice distinct from this repository's MIT license. | Credible alternative if the egui gate fails. Licensing terms need explicit review; do not imply Slint necessarily requires payment or that it is incompatible with every MIT application. |
| Tauri | Rust backend with HTML/JS/CSS frontend and the system WebView; avoids bundling a whole browser engine. | Not selected: the owner prefers a native renderer and as much Rust as practical. A small executable alone would not establish low total WebView memory. |
| Direct Win32/custom renderer | Could tailor Windows dependencies precisely, but we would own substantial rendering, text, focus, DPI, accessibility, and widget infrastructure. | Not selected initially. That maintenance burden is not justified merely to minimize direct crate count. No equivalent implementation was benchmarked. |

Sources: [egui](https://github.com/emilk/egui),
[eframe features](https://docs.rs/eframe/latest/eframe/),
[Iced docs](https://docs.iced.rs/iced/),
[Slint](https://github.com/slint-ui/slint),
[Tauri](https://v2.tauri.app/start/).

## Dependency and renderer policy

Do not use the full default eframe feature set blindly. The inspected
documentation exposes independent accessibility, font, link, persistence,
platform, and renderer features.

Start the resolution investigation with defaults disabled and an explicit
Windows-relevant set: AccessKit, fonts, and exactly one renderer. Retain
accessibility; fewer dependencies is not permission to break keyboard/accessibility.
Default fonts can be a small initial choice; validate Czech glyphs and licensing
before replacing them with system fonts.

Evaluate Glow first as the lean-renderer candidate because eframe documents
that it can significantly reduce binary size relative to wgpu. This is not
evidence of lower startup time, RAM, or better driver compatibility.
Compare actual Windows behavior with a separately built wgpu candidate when
necessary; do not enable both backends in the shipping executable by default.
Driver, DPI, remote-session behavior, and idle CPU are part of that decision.

Do not add egui_extras, image decoders, embedded browsers, a database, a large
agent harness, or multiple async runtimes preemptively. Add a production crate
only for a required capability with source/license/dependency review.
Development-only UI testing tools are separate from production dependencies.

The authorized implementation now pins eframe 0.36.2 with defaults disabled and
only `accesskit`, `default_fonts`, and `glow`. Windows adapters use windows and
windows-core 0.62.2. The latter is a direct dependency required by the COM callback
macro and was already present transitively; offline resolution preserved its
version. `Cargo.lock` is retained and release/test commands use `--locked`.

The lockfile spans multiple platforms. The Windows target dependency inventory
contains 139 unique package/version entries, including this package. Few direct
dependencies does not mean few transitive dependencies. No database, WebView,
JavaScript runtime, async runtime, or agent harness was added.

## Application structure

Begin with one small Cargo package and well-separated modules. Introduce
additional workspace crates only when isolation/reuse warrants them.

| Boundary | Responsibilities |
|---|---|
| `core` | Typed commands, pane state, selection, sorting/filtering, operation plans, approval state, deduplication and domain rules. No egui dependency in core logic. |
| `ui` | Ledger widgets, focus controller, command palette, contextual rail, themes and accents. Dispatch typed commands; never perform blocking filesystem/network work during drawing. |
| `services` | Actual filesystem discovery/operations, persistence, and later AI/MCP/speech adapters. Background execution with cancellation and progress. |
| `platform` | Small Windows-specific boundaries for app launching, path semantics, recovery, accessibility/platform integration where needed. |

Use real `PathBuf`/filesystem identities rather than copying the prototype's
string-based fake tree. Stable selection identities must survive sorting and
refresh. Port behavioral requirements and test scenarios, not JavaScript APIs.

Use standard worker threads and bounded channels for initial filesystem tasks;
do not add an async runtime until actual provider integration requires it.
Tag results with a request/generation ID so a slow previous folder scan cannot
overwrite the latest pane. Explicitly cap queues and stale work.

Render only visible rows. Cache discovery, metadata, sorting and filtering
results by revision; never enumerate directories, sort every file, clone the
whole file list, or call a model on each UI frame.
Request repaint on input/progress/completion; do not install a continuous
animation/timer merely to keep the application running.

Keep operation approval and execution independent of rendering. A repaint must
never retry a copy/move/delete. Exact targets, operation IDs, renewed approval
after material changes, and actual-state read-back remain mandatory.

## First real vertical slice

1. Native Ledger window: two panes, contextual rail initially showing favorites,
   command registry, keyboard focus, all appearance combinations.
2. Real read-only folder discovery, selection, filtering, sorting and persisted
   favorites. Workers and virtualized rows from the start.
3. Real copy through the same typed plan/executor, initially exercised only on
   inventoried synthetic files. Verify actual bytes, source preservation,
   conflict refusal, cancellation and visible errors.
4. Move/delete after operation/recovery policies are approved; real VS Code
   launch once the adapter is wired. Copilot launching is a separate feasibility
   gate, not a fake success button.
5. Bind Foundry/data/cost settings before adding real AI search, planning, MCP
   answers, or speech. Native UI delivery must not falsely imply those providers
   are already connected.

This slice establishes an app doing real things. It is not another mockup batch.
Existing user files are never used as destructive test inputs.

## Measured viability gate

Measure a release build, not a debug build or the browser design lab:

- Cold/warm startup to a usable Ledger workspace; distinguish first font/GPU
  initialization and directory loading.
- Executable size, shipped assets, direct/transitive dependencies, and features.
- Idle private memory/working set and CPU; include any child processes if present.
- Input-to-render responsiveness and scan cancellation under a frozen generated
  corpus, including a large folder. Report trial conditions and statistics.
- Czech filenames/text input, physical CZ/US shortcuts, DPI scaling, and actual
  AccessKit tree/keyboard behavior.
- Responsiveness during long filesystem work; later separate local overhead from
  network/model latency.

Numeric acceptance thresholds, reference hardware, corpus sizes, repeat policy
and measurement commands remain to be bound with the owner. Initial measurements
can establish a baseline but cannot be called a performance PASS without those
limits. egui documentation's performance descriptions are not our benchmarks.

If native keyboard/accessibility, appearance, renderer compatibility or measured
resource use fails the agreed gate, repair the failed check within the Goal Card
limits or stop for a renderer/toolkit decision. Do not silently switch to a WebView
or weaken the finish line.

## Local prerequisites and observed baseline

Before installation, inspection on 2026-10-07 found missing Rust tooling and no
compatible MSVC installation. Following the owner's explicit installation/build
authorization, the machine now has:

- Rust stable 1.99.0, Cargo 1.99.0, rustfmt, Clippy and rustup.
- Visual Studio 2022 Build Tools 17.14.41, C++ workload and Windows SDK.
- No Cargo source override at the inspected conventional project/user config
  paths or registry-related environment settings. The supplied approved feeds
  concern npm/PyPI/NuGet; Cargo resolution used its existing configuration.
  This observation is not a general managed-machine registry approval.

Final release: `target\release\tomas-commander.exe`, 7,596,544 bytes (7.24 MiB).
Build command: `cargo build --release --locked`.
Formatting, 16 default tests, the separately opted-in real recycle/restore test,
strict all-target Clippy and the release build passed. The actual native-window
smoke passed 21 checks, including approved copy byte read-back and immediate-close
preference persistence. Testing mutated only inventoried synthetic fixtures.

Latest baseline on this Intel Core i7-13800H (14 cores / 20 logical processors):

| Measurement | Observed value |
|---|---|
| Startup to accessible controls | 733.2 ms |
| Idle private memory | 60.5 MiB |
| Idle working set | 114.9 MiB |
| Process CPU over five idle seconds | 78.1 ms |

This is one warm local synthetic-folder trial with UI Automation/AccessKit active,
not a cold-start distribution, representative workload benchmark or C04-PERF PASS.
An earlier build observed 868.5 ms startup and 60.3 MiB private memory; results are
not claimed stable. The debug filesystem test listed 1,001 real entries in
59.35 ms on its latest run, not a release latency acceptance check.

Evidence resides in the session artifacts: `native-ledger-smoke.json` and
`APP-01-native-ledger-status.json`, with final binary/source identities.
Physical CZ/US keyboards, pixel/DPI/driver coverage, agreed performance thresholds,
actual VS Code workspace read-back and supported Copilot session launch remain
unverified. AI/MCP/voice providers are not connected.

## Evidence manifest

Scope: public primary technical documentation, implementation/release and license
metadata, plus local prerequisite inspection. Internal workplace content is not
relevant to this GUI/toolchain decision and was not accessed. Community sentiment
and comparative benchmarks were not required or collected.

Coverage: primary docs and implementation/license metadata searched successfully;
local prerequisites inspected. No claim of exhaustive framework comparison.

| Source / URL | Class / date or version | Retrieval and outcome | Supports |
|---|---|---|---|
| https://github.com/emilk/egui | Maintainer implementation docs; retrieved 2026-10-07 | WebIQ browse returned retry guidance/no content; built-in web_fetch then retrieved two bounded segments successfully. One source. | Native support, dependency caveat, custom styling, repaint behavior, worker guidance, license declarations. |
| https://docs.rs/eframe/latest/eframe/ | Package documentation; mutable latest | WebIQ browse returned retry guidance/no content; web_fetch retrieved feature documentation successfully. | AccessKit, fonts, explicit renderers, documented Glow binary-size opportunity. |
| https://docs.rs/crate/eframe/latest/features | Package feature metadata; mutable latest | web_fetch; one successful page. | Default feature inventory; not an approved resolver result. |
| https://api.github.com/repos/emilk/egui/releases/latest | Release metadata; 0.36.2 / 2026-09-08 | `gh api`; one release object. | Upstream release observation only. |
| https://github.com/emilk/egui/blob/main/docs/accessibility.md | Maintainer implementation guide; mutable main | web_fetch; one successful page. | Built-in/custom widget accessibility and AccessKit-based UI test option. |
| https://docs.rs/egui/latest/egui/containers/scroll_area/struct.ScrollArea.html | API docs; mutable latest | web_fetch returned introductory scroll-area content; method details not extracted. | Scrolling support only; not relied on for an uninspected API signature. |
| https://docs.iced.rs/iced/ | Maintainer docs; retrieved 2026-10-07 | WebIQ browse returned retry guidance/no content; web_fetch retrieved the relevant pocket guide, with unrelated remainder truncated. | State/update/view architecture, tasks, experimental status. |
| https://github.com/iced-rs/iced | Maintainer README; mutable default branch | web_fetch; one successful page. | Architecture and experimental status; no speed comparison. |
| https://api.github.com/repos/iced-rs/iced/license | License metadata | `gh api`; one object, MIT. | Iced license declaration. |
| https://slint.dev/license | Original candidate URL | WebIQ explicit temporary service failure; web_fetch HTTP 404. No usable source. | None. |
| https://slint.dev/license.html -> https://slint.dev/pricing | Pricing/licensing landing page | web_fetch redirect; extracted only one FAQ answer. Insufficient for overall licensing decision. | Not relied upon. |
| https://github.com/slint-ui/slint | Maintainer docs; mutable default branch | web_fetch; one successful page. | Compiled declarative UI, Rust integration, renderers, stable-API statement, listed licensing choices. Actual license obligations not adjudicated. |
| https://v2.tauri.app/start/ | First-party docs; v2 | web_fetch; one successful page. | HTML/JS frontend, Rust integration, system WebView. Advertised size not treated as our measured footprint. |
| https://rust-lang.github.io/rustup/installation/windows-msvc.html | Official rustup prerequisites | web_fetch two bounded raw-HTML segments; main prerequisite paragraph obtained. | MSVC linker/library prerequisite; published installation commands were not executed. |
| Local tool/source inspection | Windows machine, 2026-10-07 | PowerShell command discovery, conventional Cargo-path inspection, `vswhere`; initial inspection preceded installation. | Initial missing/discoverability findings, not a complete inventory of every possible custom toolchain path. |
| Local saved build and real native window | eframe 0.36.2; Windows MSVC, 2026-10-07 | Locked Cargo tests/lint/release build; Windows UI Automation smoke: 21 PASS, synthetic fixtures cleaned. | Actual standalone slice and bounded baseline, not full product acceptance. |

WebIQ's failed primary-page retrieval was replaced by direct reads of the same
public sources. No access restriction was bypassed and no private project code
was submitted to a public service.
