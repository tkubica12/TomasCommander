# Native Windows architecture

Tomas Commander uses Rust, egui/eframe 0.36.2, Glow and AccessKit. It is a native
Windows executable with Rust-rendered controls, not stock Win32 widgets. It does
not ship a browser, WebView, JavaScript runtime or localhost server.

## Boundaries

| Area | Implementation |
|---|---|
| Window, commands, pane state and interaction | `src\main.rs`, `src\app.rs` |
| Files, snapshots and native adapters | `src\files.rs`, `src\platform.rs` |
| Reviewed effects and durable receipts | `src\journal.rs`, `src\naming.rs`, `src\changes.rs`, `src\archive_changes.rs` |
| Aggregate metadata/watch invalidation | `src\directory_cache.rs` |
| Readers and archive navigation | `src\viewer.rs`, `src\viewer_ui.rs`, `src\html.rs`, `src\pdf.rs`, `src\archives.rs` |
| AI/provider boundary and contained workers | `src\ai.rs`, `src\ai\windows.rs`, `src\ai_ui.rs` |
| Typed intent/search and connected answers | `src\commands.rs`, `src\search.rs`, `src\answers.rs`, `src\answers_ui.rs`, `src\answers\windows.rs` |
| Nonsecret user preferences | `src\preferences.rs` |

Filesystem scans, search, decoding and effect planning/execution use bounded
background work rather than blocking painting. Generation IDs reject stale
results. The two panes share typed operations and a metadata cache.
External processes use separate arguments rather than interpolated shell text.

## Build and validation

`.\scripts\build.ps1` builds the locked default-only release using the standard
Cargo cache and atomically publishes `release\tomas-commander.exe`.
A failure leaves the previous executable intact. No versioned local EXE copies
or per-task target trees are part of delivery; GitHub Releases hold published
semantic versions.

`native-search-tests`, `native-answer-tests` and `native-desktop-tests` are opt-in,
nonshipping diagnostics. Build only via Diagnostic mode, use isolated owned
fixtures, close the host and use CleanDiagnostic afterwards.
`src\desktop_metrics.rs` samples controller/widget CPU; it does not measure
full render/GPU or physical input-to-display latency.

Automated Rust tests cover service effects and egui events; native PowerShell
smoke/visual scripts exercise owned Windows fixtures. Neither establishes full
physical CZ/US, multi-DPI, accessibility or hardware performance acceptance.
No universal speed/memory guarantee is claimed.

The original browser design exploration and session-specific evidence diaries
are not production dependencies and have been removed from this release tree.
