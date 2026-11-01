//! App-side recording for the local usage log (`tomas_commander::usage`): invocation sources,
//! per-frame trackers (unhandled keys, dialogs, Ask, filters, slow frames), the Settings tab
//! and the Usage insights dialog. Every hook is a few comparisons when the log is off.

use super::*;
use tomas_commander::{
    files::MoveStrategy,
    usage::{self, Activity, Counters, Event, UsageLog, report},
};

const SLOW_FRAME_MS: u64 = 50;
const SLOW_FOLDER_MS: u64 = 300;
const SLOW_ACTION_MS: u64 = 500;
const ZOOM_SETTLE: Duration = Duration::from_secs(1);
/// Seconds between clipboard events beyond which a held chord counts as pressed again even
/// without a release; OS auto-repeat starts within one second.
const CLIPBOARD_REPEAT_GAP: f64 = 1.5;

/// Command-line facts for `session_start`; `allowed` decides whether this run keeps a usage log.
#[derive(Clone, Copy, Debug, Default)]
pub struct UsageLaunch {
    pub allowed: bool,
    pub root: bool,
    pub fixture: bool,
    pub settings: bool,
}

/// How a command was invoked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Via {
    Shortcut,
    Button,
    Palette,
    Slash,
    ContextMenu,
    Drag,
    DoubleClick,
    Click,
    Rail,
    TabStrip,
    Header,
    MouseButton,
    Wheel,
    /// Diagnostic test hooks only.
    #[cfg_attr(
        not(any(feature = "native-answer-tests", feature = "native-search-tests")),
        allow(dead_code)
    )]
    Other,
}

impl Via {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Shortcut => "shortcut",
            Self::Button => "button",
            Self::Palette => "palette",
            Self::Slash => "slash",
            Self::ContextMenu => "context_menu",
            Self::Drag => "drag",
            Self::DoubleClick => "double_click",
            Self::Click => "click",
            Self::Rail => "rail",
            Self::TabStrip => "tab_strip",
            Self::Header => "header",
            Self::MouseButton => "mouse_button",
            Self::Wheel => "wheel",
            Self::Other => "other",
        }
    }
}

/// How a folder was reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NavVia {
    Enter,
    DoubleClick,
    Backspace,
    Parent,
    Return,
    Breadcrumb,
    Place,
    Recent,
    Drive,
    HistoryBack,
    HistoryForward,
    TypedPath,
    TabSwitch,
    SearchResult,
    AiResult,
    CommandResult,
    Other,
}

impl NavVia {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Enter => "enter",
            Self::DoubleClick => "double_click",
            Self::Backspace => "backspace",
            Self::Parent => "parent",
            Self::Return => "return",
            Self::Breadcrumb => "breadcrumb",
            Self::Place => "place",
            Self::Recent => "recent",
            Self::Drive => "drive",
            Self::HistoryBack => "history_back",
            Self::HistoryForward => "history_forward",
            Self::TypedPath => "typed_path",
            Self::TabSwitch => "tab_switch",
            Self::SearchResult => "search_result",
            Self::AiResult => "ai_result",
            Self::CommandResult => "command_result",
            Self::Other => "other",
        }
    }
}

/// Modal dialogs tracked for `dialog` events, in [`Ledger::usage_open_dialogs`] order.
pub(super) const DIALOG_NAMES: [&str; 27] = [
    "drive_picker",
    "place_name",
    "settings",
    "operations",
    "open_with",
    "clone",
    "rename",
    "new_folder",
    "inline_rename",
    "zip_name",
    "help",
    "places",
    "file_command",
    "search",
    "review_save",
    "review_zip",
    "review_rename",
    "review_undo",
    "review_copy",
    "review_move",
    "review_delete",
    "error",
    "ai_task",
    "foundry",
    "answers",
    "usage_insights",
    "ai_proposal",
];

/// One pane's filter "session": from the first typed character until cleared or left.
#[derive(Default)]
pub(super) struct FilterTrack {
    query: String,
    path: PathBuf,
    tab: usize,
    started: Option<Instant>,
    matches: usize,
    pub(super) opened: u32,
}

/// An approved operation waiting for its outcome.
pub(super) struct PendingOperation {
    op: &'static str,
    items: usize,
    destination: Option<String>,
    strategy: Option<&'static str>,
    pub(super) policy: Option<&'static str>,
    conflicts: Option<String>,
    detail: Option<String>,
    started: Instant,
}

impl PendingOperation {
    pub(super) fn named(op: &'static str, items: usize, destination: Option<&Path>) -> Self {
        Self {
            op,
            items,
            destination: destination.map(files::display_path),
            strategy: None,
            policy: None,
            conflicts: None,
            detail: None,
            started: Instant::now(),
        }
    }

    pub(super) fn plan(plan: &Plan) -> Self {
        let summary = plan.conflicts();
        Self {
            strategy: plan.move_strategy().map(|strategy| match strategy {
                MoveStrategy::SameVolume => "same_volume",
                MoveStrategy::CrossVolume {
                    recycle_originals: true,
                } => "cross_volume_recycle",
                MoveStrategy::CrossVolume { .. } => "cross_volume_keep",
            }),
            policy: summary.any().then(|| plan.policy().label()),
            conflicts: summary.any().then(|| {
                format!(
                    "conflicts={} new={} merged={} newer={} older_or_same={} replace={} skip={} rename={}",
                    summary.conflicts,
                    summary.new_items,
                    summary.merged,
                    summary.newer,
                    summary.older_or_same,
                    summary.replace,
                    summary.skip,
                    summary.rename
                )
            }),
            ..Self::named(
                operation_name(plan.operation()),
                plan.sources().len(),
                plan.destination(),
            )
        }
    }

    pub(super) fn change(change: &Change) -> Self {
        match change {
            Change::Save(plan) => Self::named("save", 1, Some(plan.source())),
            Change::Archive(plan) => Self {
                detail: Some(plan.label.to_owned()),
                ..Self::named("zip", plan.member_count(), Some(plan.destination()))
            },
            Change::Name(plan) => Self::named(
                if plan.source().is_some() {
                    "rename"
                } else {
                    "new_folder"
                },
                1,
                Some(plan.target()),
            ),
            Change::Undo(plan) => Self {
                detail: Some(plan.label().to_owned()),
                ..Self::named("undo", plan.rows().len(), None)
            },
            Change::UndoBatch(plan) => Self {
                detail: Some(plan.label().to_owned()),
                ..Self::named("undo", plan.len(), None)
            },
        }
    }
}

pub(super) fn operation_name(operation: Operation) -> &'static str {
    match operation {
        Operation::Copy => "copy",
        Operation::Move => "move",
        Operation::Recycle => "delete",
    }
}

pub(super) fn review_name(plan: &Plan) -> &'static str {
    match plan.operation() {
        Operation::Copy => "review_copy",
        Operation::Move => "review_move",
        Operation::Recycle => "review_delete",
    }
}

pub(super) fn change_name(change: &Change) -> &'static str {
    match change {
        Change::Save(_) => "review_save",
        Change::Archive(_) => "review_zip",
        Change::Name(_) => "review_rename",
        Change::Undo(_) | Change::UndoBatch(_) => "review_undo",
    }
}

#[derive(Clone, Copy, PartialEq)]
struct PrefsSnapshot {
    dark: bool,
    accent: usize,
    zoom: f32,
    places: usize,
    recent_roots: Option<usize>,
}

/// The Usage insights dialog: a report computed on a background thread.
pub(super) struct Insights {
    pub(super) days: u32,
    loading: Option<Receiver<FileResult<String>>>,
    pub(super) markdown: Option<String>,
    error: Option<String>,
    exporting: Option<Receiver<FileResult<PathBuf>>>,
    exported: Option<String>,
    cache: egui_commonmark::CommonMarkCache,
}

/// A clipboard event; egui-winit sends these instead of the key press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClipboardKind {
    Copy,
    Cut,
    Paste,
}

impl ClipboardKind {
    const ALL: [Self; 3] = [Self::Copy, Self::Cut, Self::Paste];

    fn of(event: &egui::Event) -> Option<Self> {
        match event {
            egui::Event::Copy => Some(Self::Copy),
            egui::Event::Cut => Some(Self::Cut),
            egui::Event::Paste(_) => Some(Self::Paste),
            _ => None,
        }
    }

    /// The chord behind the event, from the modifiers held when it arrived. egui-winit maps
    /// Ctrl+C/X/V and, on Windows, Ctrl+Insert, Shift+Delete and Shift+Insert; Ctrl+Insert
    /// cannot be told from Ctrl+C.
    pub(super) fn label(self, modifiers: Modifiers) -> String {
        let command = modifiers.ctrl || modifiers.command || modifiers.mac_cmd;
        let key = match self {
            Self::Copy if command => Key::C,
            Self::Cut if command => Key::X,
            Self::Paste if command => Key::V,
            Self::Cut if modifiers.shift => Key::Delete,
            Self::Paste if modifiers.shift => Key::Insert,
            Self::Copy => Key::Copy,
            Self::Cut => Key::Cut,
            Self::Paste => Key::Paste,
        };
        key_label(key, modifiers)
    }

    /// Whether releasing `key` ends a held chord of this kind.
    fn released_by(self, key: Key) -> bool {
        match self {
            Self::Copy => matches!(key, Key::C | Key::Insert | Key::Copy),
            Self::Cut => matches!(key, Key::X | Key::Delete | Key::Cut),
            Self::Paste => matches!(key, Key::V | Key::Insert | Key::Paste),
        }
    }
}

/// Clipboard chords repeat their event on every OS auto-repeat without a `repeat` flag, so a
/// held chord is one press until its key or a modifier is released.
#[derive(Default)]
struct ClipboardKeys {
    /// Input time of the latest event of each kind while its chord is held.
    held: [Option<f64>; 3],
    /// Presses that started in this pass, with their labels.
    pressed: Vec<(ClipboardKind, String)>,
}

impl ClipboardKeys {
    fn event(&mut self, kind: ClipboardKind, modifiers: Modifiers, time: f64) {
        let held = &mut self.held[kind as usize];
        if held.is_none_or(|last| time - last > CLIPBOARD_REPEAT_GAP) {
            self.pressed.push((kind, kind.label(modifiers)));
        }
        *held = Some(time);
    }

    fn release(&mut self, key: Key) {
        for kind in ClipboardKind::ALL {
            if kind.released_by(key) {
                self.held[kind as usize] = None;
            }
        }
    }

    fn release_all(&mut self) {
        self.held = [None; 3];
    }
}

pub(super) struct UsageState {
    pub(super) log: UsageLog,
    launch: UsageLaunch,
    started: Instant,
    pub(super) activity: Activity,
    pub(super) counters: Counters,
    /// Input time of the last pass whose input was counted.
    counted_time: Option<f64>,
    /// Modifiers at the end of the last counted pass, where the next pass's events start.
    modifiers: Modifiers,
    clipboard: ClipboardKeys,
    last_summary: Instant,
    frames: u64,
    total_frames: u64,
    slow_frames: u64,
    max_frame_ms: u64,
    frame_start: Option<Instant>,
    last_slow_frame: Option<Instant>,
    focus: &'static str,
    last_nav: [Option<Instant>; 2],
    pub(super) filters: [FilterTrack; 2],
    dialogs: [Option<Instant>; DIALOG_NAMES.len()],
    outcomes: Vec<(&'static str, &'static str, Option<String>)>,
    palette_open: bool,
    pub(super) ask_done: bool,
    pub(super) ask_candidates: usize,
    busy_since: Option<(Instant, &'static str)>,
    pub(super) last_command: &'static str,
    pub(super) via: Option<Via>,
    pub(super) errors: u64,
    pub(super) policy_changes: u32,
    pub(super) operation: Option<PendingOperation>,
    prefs: Option<PrefsSnapshot>,
    zoom_changed: Option<Instant>,
    last_wheel_zoom: Option<Instant>,
    pub(super) insights: Option<Insights>,
    pub(super) delete_confirm: bool,
    delete_focus: bool,
    deleting: Option<Receiver<FileResult<usize>>>,
    pub(super) notice: Option<String>,
}

impl UsageState {
    pub(super) fn new(log: UsageLog, launch: UsageLaunch, started: Instant) -> Self {
        Self {
            log,
            launch,
            started,
            activity: Activity::new(started),
            counters: Counters::default(),
            counted_time: None,
            modifiers: Modifiers::NONE,
            clipboard: ClipboardKeys::default(),
            last_summary: Instant::now(),
            frames: 0,
            total_frames: 0,
            slow_frames: 0,
            max_frame_ms: 0,
            frame_start: None,
            last_slow_frame: None,
            focus: "pane",
            last_nav: [None; 2],
            filters: Default::default(),
            dialogs: [None; DIALOG_NAMES.len()],
            outcomes: Vec::new(),
            palette_open: false,
            ask_done: false,
            ask_candidates: 0,
            busy_since: None,
            last_command: "",
            via: None,
            errors: 0,
            policy_changes: 0,
            operation: None,
            prefs: None,
            zoom_changed: None,
            last_wheel_zoom: None,
            insights: None,
            delete_confirm: false,
            delete_focus: false,
            deleting: None,
            notice: None,
        }
    }

    /// Starts the writer for `settings` when this run may log; otherwise a disabled log.
    pub(super) fn start(
        settings: &Path,
        enabled: bool,
        launch: UsageLaunch,
        started: Instant,
    ) -> Self {
        let log = if launch.allowed {
            UsageLog::start(
                usage::Config::new(usage::folder_for_settings(settings)),
                enabled,
            )
        } else {
            UsageLog::disabled()
        };
        Self::new(log, launch, started)
    }

    /// Sets the outcome of `dialog` if it closes in this frame.
    pub(super) fn outcome(&mut self, dialog: &'static str, outcome: &'static str) {
        self.outcomes.push((dialog, outcome, None));
    }

    pub(super) fn outcome_with(
        &mut self,
        dialog: &'static str,
        outcome: &'static str,
        extra: String,
    ) {
        self.outcomes.push((dialog, outcome, Some(extra)));
    }

    fn take_outcome(&mut self, dialog: &str) -> (&'static str, Option<String>) {
        match self
            .outcomes
            .iter()
            .rposition(|(name, _, _)| *name == dialog)
        {
            Some(index) => {
                let (_, outcome, extra) = self.outcomes.swap_remove(index);
                (outcome, extra)
            }
            None => ("closed", None),
        }
    }
}

/// Focus where text widgets or selectable previews read keys without consuming them.
fn text_focus(focus: &str) -> bool {
    matches!(
        focus,
        "ask" | "dialog" | "filter" | "path" | "editor" | "preview"
    )
}

fn function_key(key: Key) -> bool {
    let name = key.name();
    name.len() >= 2 && name.starts_with('F') && name[1..].bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether an unconsumed key press is worth reporting: chords, function keys and Escape-like
/// keys, but not text typing, AltGr characters or what text editors handle without consuming.
pub(super) fn reportable_key(key: Key, modifiers: Modifiers, text: bool) -> bool {
    if modifiers.ctrl && modifiers.alt {
        return false;
    }
    let chord = modifiers.ctrl || modifiers.command || modifiers.mac_cmd || modifiers.alt;
    if !(chord || function_key(key) || matches!(key, Key::Escape | Key::Delete | Key::Insert)) {
        return false;
    }
    if modifiers.alt && key == Key::F4 {
        return false;
    }
    if text {
        if matches!(key, Key::Escape | Key::Enter | Key::Tab) {
            return false;
        }
        let editing = matches!(
            key,
            Key::A
                | Key::C
                | Key::V
                | Key::X
                | Key::Z
                | Key::Y
                | Key::ArrowLeft
                | Key::ArrowRight
                | Key::ArrowUp
                | Key::ArrowDown
                | Key::Home
                | Key::End
                | Key::Backspace
                | Key::Delete
                | Key::Insert
        );
        if editing && !modifiers.alt {
            return false;
        }
    }
    true
}

pub(super) fn key_label(key: Key, modifiers: Modifiers) -> String {
    let mut label = String::new();
    if modifiers.ctrl || modifiers.command || modifiers.mac_cmd {
        label.push_str("Ctrl+");
    }
    if modifiers.alt {
        label.push_str("Alt+");
    }
    if modifiers.shift {
        label.push_str("Shift+");
    }
    label.push_str(key.name());
    label
}

fn button_name(button: egui::PointerButton) -> &'static str {
    match button {
        egui::PointerButton::Primary => "primary",
        egui::PointerButton::Secondary => "secondary",
        egui::PointerButton::Middle => "middle",
        egui::PointerButton::Extra1 => "back",
        egui::PointerButton::Extra2 => "forward",
    }
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
}

/// Keyboard hint of a command from the palette table, empty without a shortcut.
pub(super) fn shortcut_hint(command: Command) -> &'static str {
    COMMANDS
        .iter()
        .find(|(candidate, _, _)| *candidate == command)
        .map_or("", |(_, _, hint)| hint)
}

impl Ledger {
    /// Selection and focus at the moment a command runs.
    fn usage_command_event(
        &self,
        name: &'static str,
        via: Via,
        shortcut: &'static str,
        ok: bool,
    ) -> Event {
        let pane = &self.panes[self.active];
        let (surface, focused, kind) = if self.archive.target == Some(self.active) {
            ("zip", Some(self.archive.archive_location().0), "zip_member")
        } else if self.reader.target == Some(self.active) {
            ("preview", Some(self.reader.archive_location().0), "file")
        } else {
            let entry = pane
                .focused
                .as_ref()
                .and_then(|path| pane.entries.iter().find(|entry| &entry.path == path));
            let kind = match entry {
                None => "none",
                Some(entry) if entry.directory => "folder",
                Some(entry)
                    if entry
                        .path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip")) =>
                {
                    "zip"
                }
                Some(_) => "file",
            };
            (
                if self.rail_index.is_some() {
                    "rail"
                } else {
                    "files"
                },
                entry.map(|entry| entry.path.clone()),
                kind,
            )
        };
        let mut event = Event::new("command")
            .with("cmd", name)
            .with("via", via.as_str())
            .with("ok", ok);
        if !shortcut.is_empty() {
            event = event.with("shortcut", shortcut);
        }
        event
            .with("surface", surface)
            .with("pane", self.active)
            .with("tab", pane.tab)
            .with("tabs", pane.tabs.len())
            .with("selected", pane.selected.len())
            .with("kind", kind)
            .with(
                "ext",
                focused
                    .as_deref()
                    .filter(|_| kind == "file" || kind == "zip")
                    .and_then(extension),
            )
            .with("focused", focused.as_deref().map(files::display_path))
    }

    /// Records a command that runs outside [`Ledger::dispatch`] (opening Ask, zoom keys...).
    pub(super) fn usage_action(&mut self, name: &'static str, via: Via, shortcut: &'static str) {
        self.usage.last_command = name;
        self.usage
            .log
            .record(|| self.usage_command_event(name, via, shortcut, true));
    }

    /// Records `command` around its execution; `ok` is false when it ended in an error.
    pub(super) fn usage_dispatch(
        &mut self,
        via: Via,
        command: Command,
        run: impl FnOnce(&mut Self),
    ) {
        let errors = self.usage.errors;
        let enabled = self.usage.log.is_enabled();
        // Selection and focus are captured before the command changes them.
        let event = enabled
            .then(|| self.usage_command_event(command.slash(), via, shortcut_hint(command), true));
        let previous = self.usage.via.replace(via);
        self.usage.last_command = command.slash();
        run(self);
        self.usage.via = previous;
        if let Some(mut event) = event {
            if errors != self.usage.errors
                && let Some((_, ok)) = event.fields.iter_mut().find(|(key, _)| *key == "ok")
            {
                *ok = usage::Value::Bool(false);
            }
            self.usage.log.record(|| event);
        }
    }

    pub(super) fn usage_navigate(&mut self, index: usize, to: &Path, via: NavVia) {
        if self.usage.log.is_enabled() {
            let from = self.panes[index].path.clone();
            self.usage_navigation(index, &from, to, via);
        }
    }

    /// Records a navigation that already moved pane `index` away from `from`.
    pub(super) fn usage_arrived(&mut self, index: usize, from: PathBuf, via: NavVia) {
        if self.usage.log.is_enabled() && self.panes[index].path != from {
            let to = self.panes[index].path.clone();
            self.usage_navigation(index, &from, &to, via);
        }
    }

    fn usage_navigation(&mut self, index: usize, from: &Path, to: &Path, via: NavVia) {
        let now = Instant::now();
        let since = self.usage.last_nav[index]
            .replace(now)
            .map(|last| now.duration_since(last));
        let tab = self.panes[index].tab;
        self.usage.log.record(|| {
            Event::new("navigate")
                .with("pane", index)
                .with("tab", tab)
                .with("from", files::display_path(from))
                .with("to", files::display_path(to))
                .with("via", via.as_str())
                .with("ms_since_last_nav", since)
        });
    }

    pub(super) fn usage_tab(&mut self, index: usize, action: &'static str) {
        let via = self.usage.via.unwrap_or(Via::TabStrip);
        let count = self.panes[index].tabs.len();
        self.usage.log.record(|| {
            Event::new("tab")
                .with("action", action)
                .with("via", via.as_str())
                .with("pane", index)
                .with("count", count)
        });
    }

    /// Finished AI tasks, previews and ZIP reads reported by their panels.
    pub(super) fn usage_panels(&mut self) {
        if let Some(task) = self.file_agent.finished.take() {
            self.usage.log.record(|| {
                Event::new("ai_task")
                    .with("kind", "file_search")
                    .with("prompt", task.prompt)
                    .with("outcome", task.outcome)
                    .with("calls", task.calls)
                    .with("results", task.results)
                    .with("duration_ms", task.duration_ms)
                    .with("continuation", task.continuation)
            });
        }
        if let Some(preview) = self.reader.usage.take() {
            self.usage.log.record(|| {
                let ext = match &preview.member {
                    Some(member) => extension(Path::new(member)),
                    None => extension(&preview.path),
                };
                Event::new("preview")
                    .with("path", files::display_path(&preview.path))
                    .with("member", preview.member)
                    .with("ext", ext)
                    .with("kind", preview.kind)
                    .with("action", preview.action)
                    .with("open_ms", preview.ms)
                    .with("ok", preview.ok)
            });
        }
        if let Some(read) = self.archive.usage.take() {
            self.usage.log.record(|| {
                Event::new("zip")
                    .with(
                        "action",
                        if read.action == "open" {
                            "open"
                        } else {
                            "reload"
                        },
                    )
                    .with("path", files::display_path(&read.path))
                    .with("ms", read.ms)
                    .with("ok", read.ok)
            });
        }
    }

    /// Records a Recent/Places rail action.
    pub(super) fn usage_rail(&mut self, action: &'static str, via: Via) {
        self.usage.log.record(|| {
            Event::new("recent_rail")
                .with("action", action)
                .with("via", via.as_str())
        });
    }

    pub(super) fn usage_error(&mut self, message: &str) {
        self.usage.errors += 1;
        let context = self.usage.last_command;
        let dialog = self.usage_first_dialog();
        self.usage.log.record(|| {
            Event::new("error")
                .with("message", message.to_owned())
                .with("context", context)
                .with("dialog", dialog)
        });
    }

    /// The file opened from a pane while its filter is active counts for the filter session.
    pub(super) fn usage_filter_opened(&mut self, index: usize) {
        if !self.panes[index].filter.is_empty() {
            self.usage.filters[index].opened += 1;
        }
    }

    pub(super) fn usage_operation_finished(&mut self, outcome: &Outcome) {
        let pending = self.usage.operation.take();
        self.usage.log.record(|| {
            let pending = pending.unwrap_or_else(|| {
                PendingOperation::named(
                    outcome.operation.map_or("other", operation_name),
                    outcome.completed.len(),
                    None,
                )
            });
            Event::new("operation")
                .with("op", pending.op)
                .with("items", pending.items)
                .with("dest", pending.destination)
                .with("strategy", pending.strategy)
                .with("policy", pending.policy)
                .with("conflicts", pending.conflicts)
                .with("detail", pending.detail)
                .with("duration_ms", pending.started.elapsed())
                .with("completed", outcome.completed.len())
                .with("created", outcome.created.len())
                .with("replaced", outcome.replaced.len())
                .with("merged", outcome.merged.len())
                .with("skipped", outcome.skipped.len())
                .with("recycled", outcome.recycled.len())
                .with("kept", outcome.kept.len())
                .with("incomplete", outcome.incomplete.len())
                .with("cancelled", outcome.cancelled)
                .with("error", outcome.error.clone())
        });
    }

    /// The worker's listing time, without queueing or frame latency.
    pub(super) fn usage_scan_finished(&mut self, index: usize, elapsed: Duration) {
        self.usage.counters.bump("folder", "loads");
        if elapsed >= Duration::from_millis(SLOW_FOLDER_MS) {
            let pane = &self.panes[index];
            self.usage.log.record(|| {
                Event::new("slow_folder")
                    .with("path", files::display_path(&pane.path))
                    .with("entries", pane.entries.len())
                    .with("ms", elapsed)
            });
        }
    }

    /// Ctrl+wheel and Ctrl+plus/minus/0 zoom runs outside `dispatch`; `wheel` tells which.
    pub(super) fn usage_zoom_input(&mut self, before: f32, wheel: bool) {
        let after = self.preferences.zoom;
        if (after - before).abs() < 0.001 || !self.usage.log.is_enabled() {
            return;
        }
        let name = if (after - 1.0).abs() < 0.001 && (before - after).abs() > 0.11 {
            "/zoom-reset"
        } else if after > before {
            "/zoom-in"
        } else {
            "/zoom-out"
        };
        let now = Instant::now();
        if !wheel {
            let shortcut = shortcut_hint(match name {
                "/zoom-in" => Command::ZoomIn,
                "/zoom-out" => Command::ZoomOut,
                _ => Command::ZoomReset,
            });
            self.usage_action(name, Via::Shortcut, shortcut);
        } else if self
            .usage
            .last_wheel_zoom
            .is_none_or(|last| now.duration_since(last) > ZOOM_SETTLE)
        {
            // One event per wheel gesture; individual notches only feed the counters.
            self.usage_action(name, Via::Wheel, "");
        }
        if wheel {
            self.usage.last_wheel_zoom = Some(now);
            self.usage.counters.bump("zoom", "wheel_steps");
        }
    }

    /// Cancels the Ask palette explicitly before its text is cleared (Ctrl+Space toggling).
    pub(super) fn usage_ask_cancelled(&mut self) {
        if !self.usage.ask_done {
            self.usage_ask("cancelled");
        }
    }

    pub(super) fn usage_ask(&mut self, outcome: &'static str) {
        self.usage.ask_done = true;
        let candidates = self.usage.ask_candidates;
        let text = &self.palette_query;
        self.usage.log.record(|| {
            Event::new("ask")
                .with("text", text.clone())
                .with("outcome", outcome)
                .with("candidates", candidates)
        });
    }

    pub(super) fn usage_ask_command(&mut self, command: Command) {
        self.usage.ask_done = true;
        let candidates = self.usage.ask_candidates;
        let text = &self.palette_query;
        self.usage.log.record(|| {
            Event::new("ask")
                .with("text", text.clone())
                .with("outcome", format!("command:{}", command.slash()))
                .with("candidates", candidates)
        });
    }

    fn usage_focus(&self, ctx: &egui::Context) -> &'static str {
        if self.palette {
            return "ask";
        }
        if self.usage_open_dialogs().iter().any(|open| *open) {
            return "dialog";
        }
        let focused = ctx.memory(|memory| {
            if memory.has_focus(Id::new("reader-editor")) {
                Some("editor")
            } else if memory.has_focus(Id::new("archive-filter"))
                || (0..2).any(|pane| memory.has_focus(Id::new(("filter", pane))))
            {
                Some("filter")
            } else if (0..2).any(|pane| memory.has_focus(Id::new(("path", pane)))) {
                Some("path")
            } else if memory.has_focus(Id::new("reader-source-preview")) {
                Some("preview")
            } else {
                None
            }
        });
        focused.unwrap_or(if self.rail_index.is_some() {
            "rail"
        } else if self.reader.target == Some(self.active) {
            "preview"
        } else if self.archive.target == Some(self.active) {
            "zip"
        } else {
            "pane"
        })
    }

    pub(super) fn usage_open_dialogs(&self) -> [bool; DIALOG_NAMES.len()] {
        let name = self.name_dialog.as_ref();
        let plan = self.plan.as_ref().map(Plan::operation);
        [
            self.drives_popup.is_some(),
            self.place_edit.is_some(),
            self.settings_open,
            self.operations_open,
            self.open_with.is_some(),
            name.is_some_and(|dialog| dialog.clone),
            name.is_some_and(|dialog| !dialog.clone && !dialog.inline && dialog.source.is_some()),
            name.is_some_and(|dialog| !dialog.inline && dialog.source.is_none()),
            name.is_some_and(|dialog| dialog.inline),
            self.zip_dialog.is_some(),
            self.help_open,
            self.places_popup.is_some(),
            self.command_dialog.is_some(),
            self.search_dialog.is_some(),
            matches!(self.change_plan, Some(Change::Save(_))),
            matches!(self.change_plan, Some(Change::Archive(_))),
            matches!(self.change_plan, Some(Change::Name(_))),
            matches!(
                self.change_plan,
                Some(Change::Undo(_) | Change::UndoBatch(_))
            ),
            plan == Some(Operation::Copy),
            plan == Some(Operation::Move),
            plan == Some(Operation::Recycle),
            self.error.is_some(),
            self.file_agent.open,
            self.ai.open,
            self.answers.open,
            self.usage.insights.is_some(),
            self.proposal_review.is_some(),
        ]
    }

    fn usage_first_dialog(&self) -> Option<&'static str> {
        self.usage_open_dialogs()
            .iter()
            .position(|open| *open)
            .map(|index| DIALOG_NAMES[index])
    }

    /// Counts input and starts the frame timer; called before any handler consumes events.
    pub(super) fn usage_frame_begin(&mut self, ctx: &egui::Context) {
        if !self.usage.log.is_enabled() {
            return;
        }
        // While the window is hidden, eframe calls `logic` without an egui pass, and
        // `ctx.input()` still holds the last shown pass; only a pass with new input time counts.
        let time = ctx.input(|input| input.time);
        if self.usage.counted_time == Some(time) {
            return;
        }
        self.usage.counted_time = Some(time);
        let now = Instant::now();
        self.usage.frame_start = Some(now);
        let focus = self.usage_focus(ctx);
        self.usage.focus = focus;
        let text = text_focus(focus);
        let usage = &mut self.usage;
        usage.clipboard.pressed.clear();
        let input = ctx.input(|input| {
            let mut modifiers = usage.modifiers;
            let mut any = input.pointer.is_moving();
            for event in &input.events {
                match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        ..
                    } => {
                        any = true;
                        if !text {
                            usage.counters.bump("key", key.name());
                        }
                    }
                    egui::Event::Key {
                        key,
                        pressed: false,
                        ..
                    } => usage.clipboard.release(*key),
                    egui::Event::ModifiersChanged(changed) => {
                        if *changed != modifiers {
                            usage.clipboard.release_all();
                        }
                        modifiers = *changed;
                    }
                    egui::Event::WindowFocused(false) => {
                        usage.clipboard.release_all();
                        modifiers = Modifiers::NONE;
                    }
                    egui::Event::Text(typed) => {
                        any = true;
                        usage
                            .counters
                            .add("typed", focus, typed.chars().count() as u64);
                    }
                    egui::Event::PointerButton {
                        pressed: true,
                        button,
                        ..
                    } => {
                        any = true;
                        usage.counters.bump("pointer", button_name(*button));
                    }
                    egui::Event::MouseWheel { .. } => {
                        any = true;
                        usage.counters.bump("pointer", "wheel");
                    }
                    _ => {
                        if let Some(kind) = ClipboardKind::of(event) {
                            any = true;
                            usage.clipboard.event(kind, modifiers, input.time);
                        }
                    }
                }
            }
            usage.modifiers = input.modifiers;
            any
        });
        if input {
            self.usage.activity.input(now);
        }
    }

    /// Runs after every handler of the frame.
    pub(super) fn usage_frame_end(&mut self, ctx: &egui::Context) {
        self.poll_usage_tasks();
        if !self.usage.log.is_enabled() {
            return;
        }
        self.usage_unhandled_keys(ctx);
        self.usage_dialogs();
        self.usage_palette();
        self.usage_filters();
        self.usage_busy();
        self.usage_preferences();
        let now = Instant::now();
        if let Some(start) = self.usage.frame_start.take() {
            let ms = now.duration_since(start).as_millis() as u64;
            self.usage.frames += 1;
            self.usage.total_frames += 1;
            self.usage.max_frame_ms = self.usage.max_frame_ms.max(ms);
            if ms > SLOW_FRAME_MS {
                self.usage.slow_frames += 1;
                // Startup frames build font atlases; startup time is in session_start.
                if self.usage.total_frames > 3
                    && self
                        .usage
                        .last_slow_frame
                        .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
                {
                    self.usage.last_slow_frame = Some(now);
                    let context = self.usage_frame_context(ctx);
                    self.usage.log.record(|| {
                        Event::new("slow_frame")
                            .with("ms", ms)
                            .with("context", context)
                    });
                }
            }
        }
        if now.duration_since(self.usage.last_summary) >= usage::SUMMARY_INTERVAL {
            self.usage_summary(now);
        }
    }

    /// Stable grouping key of what the window showed during a slow frame.
    fn usage_frame_context(&self, ctx: &egui::Context) -> String {
        let mut parts = Vec::new();
        if self.palette {
            parts.push("ask".to_owned());
        }
        if let Some(dialog) = self.usage_first_dialog() {
            parts.push(format!("dialog:{dialog}"));
        }
        if self.reader.target.is_some() {
            parts.push(format!("preview:{}", self.reader.kind()));
        }
        if self.archive.target.is_some() {
            parts.push("zip".into());
        }
        if self.panes.iter().any(|pane| pane.busy) {
            parts.push("loading".into());
        }
        if egui::DragAndDrop::has_payload_of_type::<DragItems>(ctx) {
            parts.push("drag".into());
        }
        let entries = self
            .panes
            .iter()
            .map(|pane| pane.entries.len())
            .max()
            .unwrap_or(0);
        parts.push(
            match entries {
                0..1_000 => "entries:<1k",
                1_000..10_000 => "entries:1k-10k",
                _ => "entries:>10k",
            }
            .into(),
        );
        parts.join(" ")
    }

    fn usage_summary(&mut self, now: Instant) {
        let period = now.duration_since(self.usage.last_summary);
        self.usage.last_summary = now;
        let frames = std::mem::take(&mut self.usage.frames);
        let slow = std::mem::take(&mut self.usage.slow_frames);
        let max = std::mem::take(&mut self.usage.max_frame_ms);
        if frames == 0 && self.usage.counters.is_empty() {
            return;
        }
        let counts = self.usage.counters.take();
        let (active, idle) = self.usage.activity.totals(now);
        self.usage.log.record(|| {
            Event::new("summary")
                .with("period_ms", period)
                .with("frames", frames)
                .with("slow_frames", slow)
                .with("max_frame_ms", max)
                .with("active_ms", active)
                .with("idle_ms", idle)
                .with("counts", counts)
        });
    }

    pub(super) fn usage_session_start(&mut self, ctx: &egui::Context) {
        let rect = ctx.viewport_rect();
        let launch = self.usage.launch;
        let startup = self.usage.started.elapsed();
        self.usage.log.record(|| {
            Event::new("session_start")
                .with("version", env!("CARGO_PKG_VERSION"))
                .with("window_w", rect.width().round() as i64)
                .with("window_h", rect.height().round() as i64)
                .with("zoom", self.preferences.zoom)
                .with(
                    "theme",
                    if self.preferences.dark {
                        "dark"
                    } else {
                        "light"
                    },
                )
                .with("accent", ACCENTS[self.preferences.accent].0)
                .with("scale", ctx.pixels_per_point())
                .with("native_scale", ctx.native_pixels_per_point())
                .with("root_arg", launch.root)
                .with("fixture", launch.fixture)
                .with("settings_arg", launch.settings)
                .with("startup_ms", startup)
                .with("tabs_left", self.panes[0].tabs.len())
                .with("tabs_right", self.panes[1].tabs.len())
                .with("places", self.preferences.favorites.len())
        });
    }

    /// Final summary and `session_end`, then a bounded flush; exit never waits longer.
    pub(super) fn usage_session_end(&mut self) {
        let now = Instant::now();
        if self.usage.log.is_enabled() {
            self.usage_summary(now);
            let (active, idle) = self.usage.activity.totals(now);
            let duration = now.duration_since(self.usage.started);
            let dropped = self.usage.log.dropped();
            self.usage.log.record(|| {
                Event::new("session_end")
                    .with("duration_ms", duration)
                    .with("active_ms", active)
                    .with("idle_ms", idle)
                    .with("dropped", dropped)
            });
        }
        self.usage.log.shutdown(usage::SHUTDOWN_WAIT);
    }

    fn usage_unhandled_keys(&mut self, ctx: &egui::Context) {
        let focus = self.usage.focus;
        let text = text_focus(focus);
        let pressed = std::mem::take(&mut self.usage.clipboard.pressed);
        let keys: Vec<String> = ctx.input(|input| {
            let keys = input.events.iter().filter_map(|event| match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if reportable_key(*key, *modifiers, text) => Some(key_label(*key, *modifiers)),
                _ => None,
            });
            // Clipboard chords arrive without a key event; text and previews use them. A new
            // press is unhandled when its event is still unconsumed.
            let clipboard = pressed
                .into_iter()
                .filter(|(kind, _)| {
                    !text
                        && input
                            .events
                            .iter()
                            .any(|event| ClipboardKind::of(event) == Some(*kind))
                })
                .map(|(_, label)| label);
            keys.chain(clipboard).collect()
        });
        for key in keys {
            self.usage.log.record(|| {
                Event::new("key_unhandled")
                    .with("key", key)
                    .with("focus", focus)
            });
        }
    }

    fn usage_dialogs(&mut self) {
        let open = self.usage_open_dialogs();
        let now = Instant::now();
        for (index, is_open) in open.into_iter().enumerate() {
            match (is_open, self.usage.dialogs[index]) {
                (true, None) => {
                    self.usage.dialogs[index] = Some(now);
                    if DIALOG_NAMES[index].starts_with("review_") {
                        self.usage.policy_changes = 0;
                    }
                }
                (false, Some(since)) => {
                    self.usage.dialogs[index] = None;
                    let name = DIALOG_NAMES[index];
                    let (outcome, extra) = self.usage.take_outcome(name);
                    self.usage.log.record(|| {
                        Event::new("dialog")
                            .with("name", name)
                            .with("outcome", outcome)
                            .with("open_ms", now.duration_since(since))
                            .with("extra", extra)
                    });
                }
                _ => {}
            }
        }
        self.usage.outcomes.clear();
    }

    fn usage_palette(&mut self) {
        if self.palette == self.usage.palette_open {
            return;
        }
        self.usage.palette_open = self.palette;
        if self.palette {
            self.usage.ask_done = false;
        } else if !self.usage.ask_done {
            self.usage_ask("cancelled");
        }
    }

    fn usage_filters(&mut self) {
        let now = Instant::now();
        for index in 0..2 {
            let pane = &self.panes[index];
            let track = &mut self.usage.filters[index];
            let moved =
                !track.query.is_empty() && (track.path != pane.path || track.tab != pane.tab);
            if track.query == pane.filter && !moved {
                continue;
            }
            if !track.query.is_empty() && (pane.filter.is_empty() || moved) {
                let outcome = if track.opened > 0 {
                    "opened"
                } else if track.tab != pane.tab {
                    "left"
                } else if track.path != pane.path {
                    "navigated"
                } else {
                    "cleared"
                };
                let started = track.started.take();
                self.usage.log.record(|| {
                    Event::new("filter")
                        .with("pane", index)
                        .with("folder", files::display_path(&track.path))
                        .with("query", std::mem::take(&mut track.query))
                        .with("matches", track.matches)
                        .with("opened", track.opened)
                        .with("outcome", outcome)
                        .with(
                            "duration_ms",
                            started.map(|started| now.duration_since(started)),
                        )
                });
                track.query.clear();
                track.opened = 0;
            }
            if !pane.filter.is_empty() {
                if track.query.is_empty() {
                    track.started = Some(now);
                    track.path.clone_from(&pane.path);
                    track.tab = pane.tab;
                    track.opened = 0;
                }
                track.query.clone_from(&pane.filter);
                track.matches = pane.visible.len();
            }
        }
    }

    /// Preparation that kept the file worker busy for long becomes `slow_action`.
    fn usage_busy(&mut self) {
        match (self.operation_busy, self.usage.busy_since) {
            (true, None) => {
                // Executing an approved operation is timed by its `operation` event instead.
                let label = if self.usage.operation.is_some() {
                    ""
                } else {
                    self.usage.last_command
                };
                self.usage.busy_since = Some((Instant::now(), label));
            }
            (false, Some((since, label))) => {
                self.usage.busy_since = None;
                let elapsed = since.elapsed();
                if !label.is_empty() && elapsed >= Duration::from_millis(SLOW_ACTION_MS) {
                    self.usage.log.record(|| {
                        Event::new("slow_action")
                            .with("cmd", label)
                            .with("ms", elapsed)
                    });
                }
            }
            _ => {}
        }
    }

    fn usage_preferences(&mut self) {
        let current = PrefsSnapshot {
            dark: self.preferences.dark,
            accent: self.preferences.accent,
            zoom: self.preferences.zoom,
            places: self.preferences.favorites.len(),
            recent_roots: self.preferences.recent_roots.as_ref().map(Vec::len),
        };
        let Some(previous) = self.usage.prefs.replace(current) else {
            return;
        };
        let log = &self.usage.log;
        let change = |key: &'static str, value: usage::Value| {
            log.record(|| {
                Event::new("settings_change")
                    .with("key", key)
                    .with("value", value)
            })
        };
        if previous.dark != current.dark {
            change(
                "theme",
                usage::Value::Str(if current.dark { "dark" } else { "light" }),
            );
        }
        if previous.accent != current.accent {
            change("accent", usage::Value::Str(ACCENTS[current.accent].0));
        }
        if previous.places != current.places {
            change("places", current.places.into());
        }
        if previous.recent_roots != current.recent_roots {
            change("recent_roots", current.recent_roots.into());
        }
        if previous.zoom != current.zoom {
            self.usage.zoom_changed = Some(Instant::now());
        }
        if let Some(changed) = self.usage.zoom_changed
            && changed.elapsed() >= ZOOM_SETTLE
        {
            self.usage.zoom_changed = None;
            let zoom = (current.zoom * 100.0).round();
            self.usage.log.record(|| {
                Event::new("settings_change")
                    .with("key", "zoom")
                    .with("value", zoom)
            });
        }
    }

    /// Background results of the Settings and Insights actions; also runs while logging is off.
    fn poll_usage_tasks(&mut self) {
        if let Some(deleting) = &self.usage.deleting {
            match deleting.try_recv() {
                Ok(result) => {
                    self.usage.deleting = None;
                    self.usage.notice = Some(match result {
                        Ok(count) => format!("Deleted {count} usage files."),
                        Err(error) => error,
                    });
                    if let Some(insights) = &mut self.usage.insights {
                        insights.markdown = None;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.usage.deleting = None;
                    self.usage.notice =
                        Some("The usage writer stopped; nothing was deleted.".into());
                }
            }
        }
        if let Some(insights) = &mut self.usage.insights {
            if let Some(loading) = &insights.loading {
                match loading.try_recv() {
                    Ok(result) => {
                        insights.loading = None;
                        match result {
                            Ok(markdown) => insights.markdown = Some(markdown),
                            Err(error) => insights.error = Some(error),
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => {
                        insights.loading = None;
                        insights.error = Some("The usage report could not be computed.".into());
                    }
                }
            }
            if let Some(exporting) = &insights.exporting {
                match exporting.try_recv() {
                    Ok(result) => {
                        insights.exporting = None;
                        match result {
                            Ok(path) => {
                                insights.exported = Some(files::display_path(&path));
                            }
                            Err(error) => insights.error = Some(error),
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => insights.exporting = None,
                }
            }
        }
    }

    pub(super) fn set_usage_enabled(&mut self, enabled: bool) {
        if self.preferences.usage_log == enabled {
            return;
        }
        let record = |log: &UsageLog| {
            log.record(|| {
                Event::new("settings_change")
                    .with("key", "usage_log")
                    .with("value", enabled)
            })
        };
        if enabled {
            self.usage.log.set_enabled(true);
            record(&self.usage.log);
        } else {
            record(&self.usage.log);
            self.usage.log.set_enabled(false);
        }
        self.preferences.usage_log = enabled;
        self.save_pending = true;
    }

    pub(super) fn open_usage_insights(&mut self) {
        if self.usage.insights.is_none() {
            self.usage.insights = Some(Insights {
                days: 30,
                loading: None,
                markdown: None,
                error: None,
                exporting: None,
                exported: None,
                cache: Default::default(),
            });
        }
        self.load_usage_insights();
    }

    fn load_usage_insights(&mut self) {
        let folder = self.usage.log.folder().map(Path::to_owned);
        let log = self.usage.log.clone();
        let Some(insights) = &mut self.usage.insights else {
            return;
        };
        insights.markdown = None;
        insights.error = None;
        insights.exported = None;
        let Some(folder) = folder else {
            insights.error = Some(
                "This run keeps no usage log (fixture mode without --settings), so there is nothing to analyze."
                    .into(),
            );
            return;
        };
        let days = insights.days;
        let (sender, receiver) = mpsc::sync_channel(1);
        insights.loading = Some(receiver);
        thread::spawn(move || {
            // Events of this session are written first, then read back.
            log.flush(Duration::from_secs(2));
            let result = report::Report::load(&folder, days, usage::now_ms())
                .map(|report| report.markdown(&folder));
            let _ = sender.send(result);
        });
    }

    pub(super) fn open_usage_folder(&mut self) {
        let Some(folder) = self.usage.log.folder().map(Path::to_owned) else {
            self.fail("This run keeps no usage log.");
            return;
        };
        if let Err(error) = self.work.try_send(Work::OpenUsageFolder(folder)) {
            self.fail(format!("Opening the usage folder was not queued: {error}"));
        }
    }

    pub(super) fn usage_insights_dialog(&mut self, ctx: &egui::Context) -> bool {
        let Some(insights) = &mut self.usage.insights else {
            return false;
        };
        let mut close = false;
        let mut reload = None;
        let mut export = false;
        let mut open_folder = false;
        let folder = self.usage.log.folder().map(files::display_path);
        let modal = egui::Modal::new(Id::new("usage-insights")).show(ctx, |ui| {
            ui.set_width(760.0);
            ui.heading("Usage insights");
            ui.label(match &folder {
                Some(folder) => format!(
                    "Local usage log in {folder}. Nothing is sent anywhere; export the report to analyze it with an assistant."
                ),
                None => "This run keeps no usage log.".to_owned(),
            });
            ui.horizontal(|ui| {
                for days in [7, 30, 90] {
                    if ui
                        .selectable_label(insights.days == days, format!("Last {days} days"))
                        .clicked()
                        && insights.days != days
                    {
                        reload = Some(days);
                    }
                }
            });
            if insights.loading.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Reading the usage log...");
                });
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            if let Some(error) = &insights.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if let Some(markdown) = &insights.markdown {
                egui::ScrollArea::vertical()
                    .id_salt("usage-report")
                    .max_height(440.0)
                    .show(ui, |ui| {
                        egui_commonmark::CommonMarkViewer::new().show(
                            ui,
                            &mut insights.cache,
                            markdown,
                        );
                    });
            }
            if let Some(path) = &insights.exported {
                ui.label(format!("Report saved to {path} (path copied)."));
            }
            if insights.exporting.is_some() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            ui.separator();
            ui.horizontal(|ui| {
                export = ui
                    .add_enabled(
                        insights.markdown.is_some() && insights.exporting.is_none(),
                        egui::Button::new("Export Markdown report"),
                    )
                    .on_hover_text("Writes usage\\report-YYYY-MM-DD.md beside the log")
                    .clicked();
                open_folder = ui
                    .add_enabled(folder.is_some(), egui::Button::new("Open folder"))
                    .clicked();
                close = ui.button("Close").on_hover_text("Escape").clicked();
            });
        });
        if close || modal.should_close() {
            self.usage.insights = None;
            self.file_focus_requested = true;
            return true;
        }
        if let Some(days) = reload {
            insights.days = days;
            self.load_usage_insights();
        } else if export
            && let (Some(markdown), Some(folder)) =
                (insights.markdown.clone(), self.usage.log.folder())
        {
            let folder = folder.to_owned();
            let (sender, receiver) = mpsc::sync_channel(1);
            insights.exporting = Some(receiver);
            insights.exported = None;
            let path = report::export_path(&folder, usage::now_ms());
            ctx.copy_text(files::display_path(&path));
            thread::spawn(move || {
                let _ = sender.send(report::export(&folder, &markdown, usage::now_ms()));
            });
        }
        if open_folder {
            self.open_usage_folder();
        }
        true
    }

    /// Settings > Usage log.
    pub(super) fn usage_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Usage log");
        ui.label("Records locally what you use and how: commands and whether they came from the keyboard, mouse or Ask, keys that did nothing, navigation, cancelled dialogs, errors and slow work. Review it periodically, for example with an AI assistant. Nothing is sent anywhere.");
        ui.label("It contains full folder and file paths and Ask texts. It never contains file contents, edited text, clipboard contents, AI answers or credentials.");
        let Some(folder) = self.usage.log.folder().map(Path::to_owned) else {
            ui.label("Not recorded in this run: fixture mode without an explicit --settings file.");
            return;
        };
        let mut enabled = self.preferences.usage_log;
        if ui
            .checkbox(&mut enabled, "Record a local usage log")
            .changed()
        {
            self.set_usage_enabled(enabled);
        }
        ui.label(format!("Folder: {}", files::display_path(&folder)));
        ui.label(format!(
            "Size: {}. Kept for {} days; at most 20 MB per day and 200 MB in total.",
            format_size(self.usage.log.stored_bytes()),
            usage::RETENTION_DAYS
        ));
        if self.usage.log.dropped() > 0 {
            ui.label(format!(
                "{} events were dropped because recording fell behind.",
                self.usage.log.dropped()
            ));
        }
        if let Some(problem) = self.usage.log.problem() {
            ui.colored_label(ui.visuals().warn_fg_color, problem);
        }
        ui.horizontal(|ui| {
            if ui.button("Usage insights").clicked() {
                self.settings_open = false;
                self.open_usage_insights();
            }
            if ui.button("Open folder").clicked() {
                self.open_usage_folder();
            }
            if ui
                .add_enabled(
                    self.usage.deleting.is_none() && !self.usage.delete_confirm,
                    egui::Button::new("Delete usage data..."),
                )
                .clicked()
            {
                self.usage.delete_confirm = true;
                self.usage.delete_focus = true;
                self.usage.notice = None;
            }
        });
        if self.usage.delete_confirm {
            ui.separator();
            ui.label(format!(
                "Permanently delete all usage log files and exported usage reports in {}? Other files and settings stay.",
                files::display_path(&folder)
            ));
            let (confirm, cancel) =
                confirmation_buttons(ui, "Delete usage data", &mut self.usage.delete_focus);
            if cancel {
                self.usage.delete_confirm = false;
            } else if confirm {
                self.usage.delete_confirm = false;
                match self.usage.log.delete_all() {
                    Ok(receiver) => {
                        self.usage.deleting = Some(receiver);
                        self.usage.notice = Some("Deleting usage data...".into());
                    }
                    Err(error) => self.usage.notice = Some(error),
                }
            }
        }
        if let Some(notice) = &self.usage.notice {
            ui.label(notice);
        }
    }
}
