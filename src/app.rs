use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use eframe::egui::{self, Color32, Id, Key, Modifiers, RichText, Stroke, Vec2};
use tomas_commander::{
    commands,
    files::{self, Entry, FileResult, Listing, Operation, Outcome, Plan, Scope, Sort},
    platform,
    preferences::Preferences,
    search,
};

const ROW_HEIGHT: f32 = 29.0;
const SCROLL_GUTTER: f32 = 12.0;
pub const MIN_WINDOW_SIZE: [f32; 2] = [880.0, 560.0];

#[derive(Clone, Debug)]
pub(crate) enum DragItems {
    Files(Vec<PathBuf>),
    Zip {
        archive: PathBuf,
        prefix: String,
        selected: Vec<String>,
    },
}

impl DragItems {
    fn count(&self) -> usize {
        match self {
            Self::Files(paths) => paths.len(),
            Self::Zip { selected, .. } => selected.len(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum DropLocation {
    Folder(PathBuf),
    Zip { archive: PathBuf, prefix: String },
}

#[derive(Clone)]
struct DropHover {
    location: DropLocation,
    priority: u8,
    rect: egui::Rect,
}

pub(crate) fn register_drop_zone(
    ctx: &egui::Context,
    rect: egui::Rect,
    location: DropLocation,
    priority: u8,
) {
    if egui::DragAndDrop::has_payload_of_type::<DragItems>(ctx)
        && ctx.input(|input| {
            input
                .pointer
                .interact_pos()
                .is_some_and(|pos| rect.contains(pos))
        })
    {
        ctx.data_mut(|data| {
            let id = Id::new("file-drop-hover");
            if data
                .get_temp::<DropHover>(id)
                .is_none_or(|hover| priority > hover.priority)
            {
                data.insert_temp(
                    id,
                    DropHover {
                        location,
                        priority,
                        rect,
                    },
                );
            }
        });
    }
}

fn reveal_row(offset: f32, position: usize, viewport_height: f32) -> f32 {
    let top = position as f32 * ROW_HEIGHT;
    let bottom = top + ROW_HEIGHT;
    if top < offset {
        top
    } else if bottom > offset + viewport_height {
        (bottom - viewport_height).max(0.0)
    } else {
        offset
    }
}

fn tint(surface: Color32, accent: Color32, amount: u8) -> Color32 {
    let blend = |a: u8, b: u8| {
        ((u16::from(a) * u16::from(255 - amount) + u16::from(b) * u16::from(amount)) / 255) as u8
    };
    Color32::from_rgb(
        blend(surface.r(), accent.r()),
        blend(surface.g(), accent.g()),
        blend(surface.b(), accent.b()),
    )
}

struct RowColumns {
    name: egui::Rect,
    size: egui::Rect,
    age: egui::Rect,
}

impl RowColumns {
    fn new(rect: egui::Rect) -> Self {
        if rect.width() < 230.0 {
            let right = rect.right() - 10.0;
            return Self {
                name: egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 56.0, rect.top()),
                    egui::pos2(right.max(rect.left() + 56.0), rect.bottom()),
                ),
                size: egui::Rect::from_min_max(
                    egui::pos2(right, rect.top()),
                    egui::pos2(right, rect.bottom()),
                ),
                age: egui::Rect::from_min_max(
                    egui::pos2(right, rect.top()),
                    egui::pos2(right, rect.bottom()),
                ),
            };
        }
        let age_width = (rect.width() * 0.3).clamp(50.0, 132.0);
        let size_width = (rect.width() * 0.2).clamp(45.0, 82.0);
        let right = rect.right() - 10.0;
        Self {
            name: egui::Rect::from_min_max(
                egui::pos2(rect.left() + 56.0, rect.top()),
                egui::pos2(
                    (right - age_width - size_width - 16.0).max(rect.left() + 56.0),
                    rect.bottom(),
                ),
            ),
            size: egui::Rect::from_min_max(
                egui::pos2(right - age_width - size_width - 8.0, rect.top()),
                egui::pos2(right - age_width - 8.0, rect.bottom()),
            ),
            age: egui::Rect::from_min_max(
                egui::pos2(right - age_width, rect.top()),
                egui::pos2(right, rect.bottom()),
            ),
        }
    }
}
const ACCENTS: [(&str, [u8; 3], [u8; 3]); 4] = [
    ("Blue", [0, 109, 160], [0, 164, 239]),
    ("Red-orange", [188, 58, 22], [242, 80, 34]),
    ("Green", [76, 113, 0], [127, 186, 0]),
    ("Yellow", [128, 91, 0], [255, 185, 0]),
];

struct Pane {
    path: PathBuf,
    path_text: String,
    entries: Vec<Entry>,
    visible: Vec<usize>,
    filter: String,
    sort: Sort,
    descending: bool,
    focused: Option<PathBuf>,
    selected: BTreeSet<PathBuf>,
    anchor: usize,
    scroll_to_focus: bool,
    scroll_offset: f32,
    busy: bool,
    warning: Option<String>,
    parent_returns: Vec<ReturnLocation>,
}

struct ReturnLocation {
    path: PathBuf,
    focused: Option<PathBuf>,
    filter: String,
    scroll_offset: f32,
}

impl Pane {
    fn new(path: PathBuf) -> Self {
        Self {
            path_text: files::display_path(&path),
            path,
            entries: Vec::new(),
            visible: Vec::new(),
            filter: String::new(),
            sort: Sort::Name,
            descending: false,
            focused: None,
            selected: BTreeSet::new(),
            anchor: 0,
            scroll_to_focus: false,
            scroll_offset: 0.0,
            busy: true,
            warning: None,
            parent_returns: Vec::new(),
        }
    }

    fn rebuild(&mut self) {
        let query = self.filter.to_lowercase();
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.folded.contains(&query))
            .map(|(index, _)| index)
            .collect();
        if self.focused.is_some()
            && !self
                .visible
                .iter()
                .any(|index| Some(&self.entries[*index].path) == self.focused.as_ref())
        {
            self.focused = self
                .visible
                .first()
                .map(|index| self.entries[*index].path.clone());
            self.anchor = usize::from(!self.visible.is_empty());
            self.scroll_to_focus = true;
        }
        self.anchor = self.anchor.min(self.visible.len());
    }

    fn focus_position(&self) -> usize {
        self.visible
            .iter()
            .position(|index| Some(&self.entries[*index].path) == self.focused.as_ref())
            .map(|position| position + 1)
            .unwrap_or(0)
    }

    fn targets(&self) -> Vec<PathBuf> {
        if self.selected.is_empty() {
            self.focused
                .iter()
                .filter(|path| self.entries.iter().any(|entry| &entry.path == *path))
                .cloned()
                .collect()
        } else {
            self.selected.iter().cloned().collect()
        }
    }

    fn drag_targets(&self, path: &PathBuf) -> Vec<PathBuf> {
        if self.selected.contains(path) {
            self.selected.iter().cloned().collect()
        } else {
            vec![path.clone()]
        }
    }

    fn row_path(&self, position: usize) -> Option<PathBuf> {
        position
            .checked_sub(1)
            .and_then(|position| self.visible.get(position))
            .map(|index| self.entries[*index].path.clone())
    }

    fn jump(&mut self, down: bool) {
        let previous = self.focus_position();
        let next = if down {
            previous.saturating_add(10).min(self.visible.len())
        } else {
            previous.saturating_sub(10)
        };
        self.focused = self.row_path(next);
        self.anchor = next;
        self.scroll_to_focus = true;
    }

    fn begin_navigation(&mut self, path: PathBuf) {
        let refresh = self.path == path;
        let child = (self.path.parent() == Some(path.as_path())).then(|| self.path.clone());
        self.path_text = files::display_path(&path);
        self.path = path;
        if !refresh {
            self.parent_returns.clear();
            self.filter.clear();
            self.entries.clear();
            self.visible.clear();
            self.selected.clear();
            self.focused = child;
            self.scroll_offset = 0.0;
            self.scroll_to_focus = true;
        }
        self.busy = true;
        self.warning = None;
    }

    fn remember_parent_return(&mut self) -> Vec<ReturnLocation> {
        let location = ReturnLocation {
            path: self.path.clone(),
            focused: self.focused.clone(),
            filter: self.filter.clone(),
            scroll_offset: self.scroll_offset,
        };
        let mut history = std::mem::take(&mut self.parent_returns);
        if history.len() == 64 {
            history.remove(0);
        }
        history.push(location);
        history
    }
}

struct ScanRequest {
    path: PathBuf,
    generation: u64,
}
struct ScanResult {
    pane: usize,
    generation: u64,
    content: ScanContent,
}
enum ScanContent {
    Listing(FileResult<Listing>),
    Aggregate {
        path: PathBuf,
        result: FileResult<files::DirectoryStats>,
    },
}
struct ScanWorker {
    sender: SyncSender<ScanRequest>,
    generation: Arc<AtomicU64>,
    pending: Option<ScanRequest>,
}

enum Change {
    Save(tomas_commander::changes::Plan),
    Archive(tomas_commander::archive_changes::Plan),
    Name(tomas_commander::naming::Plan),
}
impl Change {
    fn id(&self) -> u64 {
        match self {
            Self::Save(plan) => plan.id,
            Self::Archive(plan) => plan.id,
            Self::Name(plan) => plan.id,
        }
    }
    fn label(&self) -> &'static str {
        match self {
            Self::Save(plan) => plan.label,
            Self::Archive(plan) => plan.label,
            Self::Name(plan) => plan.label(),
        }
    }
    fn summary(&self) -> String {
        match self {
            Self::Save(plan) => plan.summary(),
            Self::Archive(plan) => plan.summary(),
            Self::Name(plan) => plan.summary(),
        }
    }
}
struct ZipDialog {
    sources: Vec<PathBuf>,
    destination: PathBuf,
    name: String,
    focus: bool,
}
struct NameDialog {
    folder: PathBuf,
    source: Option<PathBuf>,
    name: String,
    focus: bool,
    error: Option<String>,
    inline: bool,
    pane: usize,
}
struct OpenWith {
    folder: PathBuf,
    index: usize,
}
enum Work {
    Change(Change),
    PrepareName {
        id: u64,
        folder: PathBuf,
        source: Option<PathBuf>,
        name: String,
    },
    PrepareArchive {
        id: u64,
        request: tomas_commander::archive_changes::Request,
    },
    ResolveCommand {
        roots: [PathBuf; 2],
        intent: commands::Intent,
    },
    PrepareCommand {
        id: u64,
        resolution: commands::Resolution,
        source: usize,
        destination: Option<usize>,
    },
    LocateCommand {
        pane: usize,
        resolution: commands::Resolution,
        source: usize,
    },
    Prepare {
        id: u64,
        operation: Operation,
        sources: Vec<PathBuf>,
        destination: PathBuf,
    },
    Execute(Plan),
    #[cfg(feature = "native-desktop-tests")]
    DesktopRejections(PathBuf),
    Save(Preferences),
    Code(PathBuf),
    Open(PathBuf),
    Copilot {
        path: PathBuf,
        app: bool,
    },
    Locate {
        pane: usize,
        result: search::Match,
    },
}

fn drop_work(
    id: u64,
    items: DragItems,
    target: DropLocation,
    copy: bool,
) -> FileResult<Option<Work>> {
    use tomas_commander::archive_changes::Request;
    Ok(Some(match (items, target) {
        (DragItems::Files(sources), DropLocation::Folder(destination)) => {
            if sources.is_empty() {
                return Err("Drag has no source files.".into());
            }
            if sources
                .iter()
                .any(|path| path.parent() == Some(destination.as_path()) || path == &destination)
            {
                return Ok(None);
            }
            Work::Prepare {
                id,
                operation: if copy {
                    Operation::Copy
                } else {
                    Operation::Move
                },
                sources,
                destination,
            }
        }
        (DragItems::Files(sources), DropLocation::Zip { archive, prefix }) => {
            Work::PrepareArchive {
                id,
                request: Request::Insert {
                    archive,
                    prefix,
                    sources,
                    move_sources: !copy,
                },
            }
        }
        (
            DragItems::Zip {
                archive,
                prefix,
                selected,
            },
            DropLocation::Folder(destination),
        ) => Work::PrepareArchive {
            id,
            request: Request::Extract {
                archive,
                prefix,
                selected,
                destination,
                move_entries: !copy,
            },
        },
        (DragItems::Zip { .. }, DropLocation::Zip { .. }) => {
            return Err(
                "ZIP-to-ZIP dragging is unsupported; extract to a real folder first.".into(),
            );
        }
    }))
}

enum WorkResult {
    NamePrepared(FileResult<tomas_commander::naming::Plan>),
    Changed {
        outcome: Outcome,
        editor: bool,
    },
    ArchivePrepared(FileResult<tomas_commander::archive_changes::Plan>),
    #[cfg(feature = "native-desktop-tests")]
    DesktopRejections(FileResult<String>),
    CommandResolved(FileResult<commands::Resolution>),
    Recovery(String),
    Prepared(FileResult<Plan>),
    Executed(Outcome),
    Saved(FileResult<()>),
    Code(FileResult<()>),
    Open(FileResult<PathBuf>),
    Copilot {
        app: bool,
        result: FileResult<()>,
    },
    Located {
        pane: usize,
        result: FileResult<PathBuf>,
    },
}

struct SearchRequest {
    generation: u64,
    root: PathBuf,
    query: String,
    intent: Option<search::Intent>,
    #[cfg(feature = "native-search-tests")]
    slow_test_walk: bool,
}

struct SearchDialog {
    pane: usize,
    root: PathBuf,
    query: String,
    focus_query: bool,
    busy: bool,
    index: usize,
    results: Option<search::Results>,
    error: Option<String>,
    scroll_offset: f32,
    ai_assisted: bool,
    consent: bool,
    interpreting: bool,
    interpretation: Option<String>,
}

struct CommandDialog {
    pane: usize,
    roots: [PathBuf; 2],
    prompt: String,
    consent: bool,
    focus_prompt: bool,
    interpreting: bool,
    resolving: bool,
    resolution: Option<commands::Resolution>,
    source: Option<usize>,
    destination: Option<usize>,
    details: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Rail {
    Favorites,
    Recent,
}

#[derive(Clone, Copy)]
enum Command {
    Rename,
    NewFolder,
    OpenWith,
    Settings,
    ClearSelection,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    CreateZip,
    EditSource,
    SaveSource,
    RotateLeft,
    RotateRight,
    Preview,
    Parent,
    ReturnInside,
    JumpUp,
    JumpDown,
    Places,
    Help,
    Copy,
    Move,
    Recycle,
    Refresh,
    Favorites,
    Recent,
    Pin,
    Sort,
    Filter,
    Search,
    AiSearch,
    AiPlan,
    Balance,
    Code,
    CopilotCli,
    CopilotApp,
    Activity,
    Theme,
    Accent,
    Foundry,
    Answers,
}

const COMMANDS: [(Command, &str, &str); 41] = [
    (
        Command::Rename,
        "Rename focused file / folder",
        "Ctrl Shift N",
    ),
    (Command::NewFolder, "New folder in active pane", "Ctrl N"),
    (
        Command::OpenWith,
        "Open with / development workspace",
        "Shift Enter",
    ),
    (
        Command::Settings,
        "Settings / appearance and connections",
        "",
    ),
    (
        Command::ClearSelection,
        "Clear selection in active pane",
        "",
    ),
    (
        Command::ZoomIn,
        "Zoom in / entire UI",
        "Ctrl + / Ctrl = / Ctrl wheel",
    ),
    (
        Command::ZoomOut,
        "Zoom out / entire UI",
        "Ctrl - / Ctrl wheel",
    ),
    (Command::ZoomReset, "Reset UI zoom to 100%", "Ctrl 0"),
    (
        Command::Preview,
        "Internal file preview / reading pane",
        "Ctrl Enter",
    ),
    (Command::Parent, "Parent folder", "Ctrl Left / Backspace"),
    (
        Command::ReturnInside,
        "Undo parent navigation",
        "Ctrl Right",
    ),
    (Command::JumpUp, "Jump up ten rows", "Ctrl Up"),
    (Command::JumpDown, "Jump down ten rows", "Ctrl Down"),
    (Command::Places, "Choose saved place", "Alt Up"),
    (Command::Help, "Keyboard shortcuts / Help", "?"),
    (
        Command::Answers,
        "Connected answers / public web and workplace boundaries",
        "",
    ),
    (Command::AiPlan, "Natural-language file command / CZ EN", ""),
    (
        Command::AiSearch,
        "AI-assisted file search / prompt only",
        "",
    ),
    (
        Command::Foundry,
        "Foundry connection and bounded text prompt",
        "",
    ),
    (Command::Copy, "Copy to opposite pane", "Ctrl Shift C"),
    (Command::Move, "Move to opposite pane", "Ctrl Shift M"),
    (Command::Recycle, "Delete selected items", "Ctrl Shift D"),
    (Command::Refresh, "Refresh folder", "Ctrl R"),
    (Command::Favorites, "Focus pinned locations", "Ctrl Shift B"),
    (Command::Recent, "Focus recent documents", "Ctrl Shift H"),
    (Command::Pin, "Pin current folder", ""),
    (Command::Sort, "Cycle sorting", "Ctrl Shift S"),
    (Command::Filter, "Filter active folder", "Ctrl F"),
    (
        Command::Search,
        "Search filenames recursively",
        "Ctrl Shift F",
    ),
    (Command::Balance, "Balance file panes", ""),
    (Command::Code, "Run VS Code", ""),
    (Command::CopilotCli, "Run GitHub Copilot CLI", ""),
    (Command::CopilotApp, "Run GitHub Copilot App", ""),
    (
        Command::Activity,
        "Show activity and exact operation results",
        "",
    ),
    (Command::Theme, "Toggle light and dark theme", ""),
    (Command::Accent, "Cycle accent color", ""),
    (
        Command::CreateZip,
        "Create ZIP from selected files / folders",
        "",
    ),
    (
        Command::EditSource,
        "Edit original text / Markdown source",
        "Reader Ctrl E",
    ),
    (
        Command::SaveSource,
        "Save edited source / exact approval",
        "Reader Ctrl S",
    ),
    (
        Command::RotateLeft,
        "Rotate reading-pane image left",
        "Reader Ctrl Left",
    ),
    (
        Command::RotateRight,
        "Rotate reading-pane image right",
        "Reader Ctrl Right",
    ),
];

pub struct Ledger {
    reader: crate::viewer_ui::ReaderPanel,
    archive: crate::viewer_ui::ReaderPanel,
    zip_dialog: Option<ZipDialog>,
    change_plan: Option<Change>,
    ai: crate::ai_ui::AiPanel,
    answers: crate::answers_ui::AnswerPanel,
    command_dialog: Option<CommandDialog>,
    panes: [Pane; 2],
    path_edits: [bool; 2],
    active: usize,
    scope: Scope,
    preferences: Preferences,
    name_dialog: Option<NameDialog>,
    open_with: Option<OpenWith>,
    settings_open: bool,
    operations_open: bool,
    viewport_zoom: f32,
    preferences_writable: bool,
    save_pending: bool,
    saves_in_flight: usize,
    close_after_save: bool,
    scans: Vec<ScanWorker>,
    aggregate_cache: Arc<tomas_commander::directory_cache::DirectoryCache>,
    cache_changes: Receiver<FileResult<tomas_commander::directory_cache::Changes>>,
    scan_results: Receiver<ScanResult>,
    work: SyncSender<Work>,
    work_results: Receiver<WorkResult>,
    cancellation: Arc<AtomicBool>,
    operation_busy: bool,
    plan: Option<Plan>,
    next_id: u64,
    last_outcome: Option<Outcome>,
    rail: Rail,
    rail_index: Option<usize>,
    activity: Vec<String>,
    status: String,
    error: Option<String>,
    palette: bool,
    palette_query: String,
    palette_focus: bool,
    palette_index: usize,
    places_popup: Option<usize>,
    place_edit: Option<(PathBuf, String)>,
    inline_pending: Option<(usize, PathBuf, f64)>,
    help_open: bool,
    approval_focus: bool,
    editor_focus: Option<Id>,
    file_focus_requested: bool,
    left_width: f32,
    workspace_width: f32,
    resize_requested: bool,
    balance_requested: bool,
    started: Instant,
    first_frame: bool,
    appearance_dirty: bool,
    search_dialog: Option<SearchDialog>,
    search_jobs: SyncSender<SearchRequest>,
    search_results: Receiver<(u64, FileResult<search::Results>)>,
    search_generation: Arc<AtomicU64>,
    copy_modifiers: Modifiers,
    #[cfg(feature = "native-desktop-tests")]
    desktop_metrics: Option<crate::desktop_metrics::Metrics>,
}

impl Ledger {
    #[cfg(feature = "native-desktop-tests")]
    pub fn enable_desktop_test(&mut self, metrics: crate::desktop_metrics::Metrics) {
        self.desktop_metrics = Some(metrics);
        self.ai.answer_test_mode = true;
        self.answers.enable_test();
    }
    #[cfg(feature = "native-answer-tests")]
    pub fn enable_answer_ui_test(&mut self) {
        self.ai.answer_test_mode = true;
        self.answers.enable_test();
        self.dispatch(Command::Answers);
    }

    #[cfg(feature = "native-search-tests")]
    pub fn enable_search_ui_test(&mut self) {
        self.ai.test_scenario = Some(crate::ai_ui::SearchTestScenario::Ambiguous);
        self.dispatch(Command::AiSearch);
        if let Some(dialog) = &mut self.search_dialog {
            dialog.query = "Synthetic failure-state test; no provider request.".into();
        }
    }

    pub fn new(
        creation: &eframe::CreationContext<'_>,
        initial: PathBuf,
        scope: Scope,
        settings: PathBuf,
        started: Instant,
    ) -> Self {
        let (preferences, error, preferences_writable) = match Preferences::load(&settings) {
            Ok(preferences) => (preferences, None, true),
            Err(error) => (Preferences::default(), Some(error), false),
        };
        let (scan_tx, scan_results) = mpsc::sync_channel(8);
        let aggregate_cache = Arc::new(tomas_commander::directory_cache::DirectoryCache::default());
        let (cache_sender, cache_changes) = mpsc::sync_channel(8);
        let cache_weak = Arc::downgrade(&aggregate_cache);
        let cache_context = creation.egui_ctx.clone();
        thread::spawn(move || {
            loop {
                let Some(cache) = cache_weak.upgrade() else {
                    break;
                };
                let result = cache.poll();
                drop(cache);
                let failed = result.is_err();
                if !matches!(&result, Ok(changes) if changes.paths.is_empty()
                    && changes.warnings.is_empty() && !changes.all)
                {
                    cache_context.request_repaint();
                    if cache_sender.send(result).is_err() {
                        break;
                    }
                    cache_context.request_repaint();
                }
                if failed {
                    break;
                }
                thread::sleep(Duration::from_millis(250));
            }
        });
        let mut scans = Vec::new();
        for pane in 0..2 {
            let (sender, receiver) = mpsc::sync_channel::<ScanRequest>(1);
            let generation = Arc::new(AtomicU64::new(0));
            let current = generation.clone();
            let results = scan_tx.clone();
            let scope = scope.clone();
            let context = creation.egui_ctx.clone();
            let cache = aggregate_cache.clone();
            thread::spawn(move || {
                while let Ok(request) = receiver.recv() {
                    if current.load(Ordering::Relaxed) != request.generation {
                        continue;
                    }
                    let listing = files::list_directory_cancellable(&scope, &request.path, || {
                        current.load(Ordering::Relaxed) != request.generation
                    });
                    if current.load(Ordering::Relaxed) != request.generation {
                        continue;
                    }
                    let directories: Vec<_> = listing
                        .as_ref()
                        .map(|listing| {
                            listing
                                .entries
                                .iter()
                                .filter(|entry| entry.directory && !entry.link)
                                .map(|entry| entry.path.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    if results
                        .send(ScanResult {
                            pane,
                            generation: request.generation,
                            content: ScanContent::Listing(listing),
                        })
                        .is_err()
                    {
                        break;
                    }
                    context.request_repaint();
                    for path in directories {
                        let stale = || current.load(Ordering::Relaxed) != request.generation;
                        if stale() {
                            break;
                        }
                        let result = cache.calculate(&scope, &path, stale);
                        if stale() {
                            break;
                        }
                        if results
                            .send(ScanResult {
                                pane,
                                generation: request.generation,
                                content: ScanContent::Aggregate { path, result },
                            })
                            .is_err()
                        {
                            return;
                        }
                        context.request_repaint();
                    }
                }
            });
            scans.push(ScanWorker {
                sender,
                generation,
                pending: None,
            });
        }
        let (work, jobs) = mpsc::sync_channel::<Work>(4);
        let (search_jobs, search_requests) = mpsc::sync_channel::<SearchRequest>(1);
        let (search_tx, search_results) = mpsc::sync_channel(2);
        let search_generation = Arc::new(AtomicU64::new(0));
        let generation = search_generation.clone();
        let search_scope = scope.clone();
        let search_context = creation.egui_ctx.clone();
        thread::spawn(move || {
            while let Ok(request) = search_requests.recv() {
                let cancelled = || {
                    #[cfg(feature = "native-search-tests")]
                    if request.slow_test_walk {
                        for _ in 0..300 {
                            if generation.load(Ordering::Relaxed) != request.generation {
                                return true;
                            }
                            thread::sleep(Duration::from_millis(10));
                        }
                    }
                    generation.load(Ordering::Relaxed) != request.generation
                };
                if cancelled() {
                    continue;
                }
                let result = match &request.intent {
                    Some(intent) => {
                        search::discover_intent(&search_scope, &request.root, intent, cancelled)
                    }
                    None => {
                        search::discover(&search_scope, &request.root, &request.query, cancelled)
                    }
                };
                if cancelled() {
                    continue;
                }
                if search_tx.send((request.generation, result)).is_err() {
                    break;
                }
                search_context.request_repaint();
            }
        });
        let (results, work_results) = mpsc::sync_channel(8);
        let cancellation = Arc::new(AtomicBool::new(false));
        let cancel = cancellation.clone();
        let worker_scope = scope.clone();
        let context = creation.egui_ctx.clone();
        thread::spawn(move || {
            let journal =
                tomas_commander::journal::Journal::new(settings.with_extension("operations"));
            if let Err(error) = journal.pending().and_then(|pending| {
                if pending.is_empty() {
                    Ok(())
                } else {
                    Err(pending.join("\n\n"))
                }
            }) {
                if results.send(WorkResult::Recovery(error)).is_err() {
                    return;
                }
                context.request_repaint();
            }
            while let Ok(job) = jobs.recv() {
                let result = match job {
                    Work::PrepareName {
                        id,
                        folder,
                        source,
                        name,
                    } => WorkResult::NamePrepared(journal.initialize().and_then(|()| {
                        tomas_commander::naming::Plan::prepare(
                            &worker_scope,
                            id,
                            &folder,
                            source.as_deref(),
                            &name,
                        )
                    })),
                    Work::Change(change) => {
                        let (outcome, editor) = match change {
                            Change::Save(plan) => {
                                (plan.execute(&worker_scope, &journal, true, &cancel), true)
                            }
                            Change::Archive(plan) => {
                                (plan.execute(&worker_scope, &journal, true, &cancel), false)
                            }
                            Change::Name(plan) => {
                                (plan.execute(&worker_scope, &journal, true, &cancel), false)
                            }
                        };
                        WorkResult::Changed { outcome, editor }
                    }
                    Work::PrepareArchive { id, request } => {
                        WorkResult::ArchivePrepared(journal.initialize().and_then(|()| {
                            tomas_commander::archive_changes::prepare(
                                &worker_scope,
                                id,
                                request,
                                &cancel,
                            )
                        }))
                    }
                    #[cfg(feature = "native-desktop-tests")]
                    Work::DesktopRejections(root) => WorkResult::DesktopRejections(
                        tomas_commander::journal::Journal::desktop_rejection_probe(
                            &worker_scope,
                            settings.with_extension("rejections"),
                            &root.join("alpha.txt"),
                            &root.join("Příliš žluťoučký folder"),
                        ),
                    ),
                    Work::ResolveCommand { roots, intent } => {
                        WorkResult::CommandResolved(journal.initialize().and_then(|()| {
                            commands::resolve(&worker_scope, &roots, &intent, &cancel)
                        }))
                    }
                    Work::PrepareCommand {
                        id,
                        resolution,
                        source,
                        destination,
                    } => WorkResult::Prepared(journal.initialize().and_then(|()| {
                        resolution.plan(&worker_scope, id, source, destination, &cancel)
                    })),
                    Work::LocateCommand {
                        pane,
                        resolution,
                        source,
                    } => WorkResult::Located {
                        pane,
                        result: resolution.locate(&worker_scope, source),
                    },
                    Work::Prepare {
                        id,
                        operation,
                        sources,
                        destination,
                    } => WorkResult::Prepared(journal.initialize().and_then(|()| {
                        files::plan_cancellable(
                            &worker_scope,
                            id,
                            operation,
                            &sources,
                            Some(&destination),
                            &cancel,
                        )
                    })),
                    Work::Execute(plan) => {
                        WorkResult::Executed(journal.execute(&worker_scope, &plan, true, &cancel))
                    }
                    Work::Save(preferences) => WorkResult::Saved(preferences.save(&settings)),
                    Work::Code(path) => WorkResult::Code(platform::open_vscode(&path)),
                    Work::Open(path) => WorkResult::Open(
                        worker_scope
                            .resolve(&path)
                            .and_then(|path| platform::open_file(&path).map(|()| path)),
                    ),
                    Work::Copilot { path, app } => WorkResult::Copilot {
                        app,
                        result: worker_scope
                            .resolve(&path)
                            .and_then(|path| platform::open_copilot(&path, app)),
                    },
                    Work::Locate { pane, result } => WorkResult::Located {
                        pane,
                        result: search::locate(&worker_scope, &result),
                    },
                };
                if results.send(result).is_err() {
                    break;
                }
                context.request_repaint();
            }
        });
        let mut app = Self {
            reader: crate::viewer_ui::ReaderPanel::new(scope.clone(), creation.egui_ctx.clone()),
            change_plan: None,
            archive: crate::viewer_ui::ReaderPanel::new(scope.clone(), creation.egui_ctx.clone()),
            zip_dialog: None,
            ai: crate::ai_ui::AiPanel::new(),
            answers: crate::answers_ui::AnswerPanel::new(),
            command_dialog: None,
            panes: [Pane::new(initial.clone()), Pane::new(initial.clone())],
            path_edits: [false; 2],
            active: 0,
            scope,
            preferences,
            name_dialog: None,
            open_with: None,
            settings_open: false,
            operations_open: false,
            viewport_zoom: creation.egui_ctx.zoom_factor(),
            preferences_writable,
            save_pending: false,
            saves_in_flight: 0,
            close_after_save: false,
            scans,
            aggregate_cache,
            cache_changes,
            scan_results,
            work,
            work_results,
            cancellation,
            operation_busy: false,
            plan: None,
            next_id: 1,
            last_outcome: None,
            rail: Rail::Favorites,
            rail_index: None,
            activity: Vec::new(),
            status: "Native Rust / loading folders".into(),
            error,
            palette: false,
            palette_query: String::new(),
            palette_focus: false,
            palette_index: 0,
            places_popup: None,
            place_edit: None,
            inline_pending: None,
            help_open: false,
            approval_focus: false,
            editor_focus: None,
            file_focus_requested: false,
            left_width: 450.0,
            workspace_width: 0.0,
            resize_requested: false,
            balance_requested: true,
            started,
            first_frame: true,
            appearance_dirty: false,
            search_dialog: None,
            search_jobs,
            search_results,
            search_generation,
            copy_modifiers: Modifiers::NONE,
            #[cfg(feature = "native-desktop-tests")]
            desktop_metrics: None,
        };
        if app.preferences.favorites.is_empty() {
            app.preferences.favorites.push(initial.clone());
        }
        #[cfg(windows)]
        {
            let mut fonts = egui::FontDefinitions::default();
            if let Some(windows) = std::env::var_os("WINDIR") {
                for (family, name, file) in [
                    (egui::FontFamily::Proportional, "ledger-sans", "segoeui.ttf"),
                    (egui::FontFamily::Monospace, "ledger-mono", "consola.ttf"),
                ] {
                    let path = PathBuf::from(&windows).join("Fonts").join(file);
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            fonts
                                .font_data
                                .insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
                            fonts
                                .families
                                .entry(family)
                                .or_default()
                                .insert(0, name.into());
                        }
                        Err(error) => app.log(format!(
                            "Appearance: could not read {}: {error}. Using bundled fallback fonts.",
                            files::display_path(&path),
                        )),
                    }
                }
                creation.egui_ctx.set_fonts(fonts);
            } else {
                app.log("Appearance: WINDIR is not defined. Using bundled fallback fonts.");
            }
        }
        app.apply_theme(&creation.egui_ctx);
        creation
            .egui_ctx
            .options_mut(|options| options.zoom_with_keyboard = false);
        creation.egui_ctx.set_zoom_factor(app.preferences.zoom);
        app.navigate(0, initial.clone());
        app.navigate(1, initial);
        app
    }

    fn accent(&self) -> Color32 {
        let (_, light, dark) = ACCENTS[self.preferences.accent];
        let [r, g, b] = if self.preferences.dark { dark } else { light };
        Color32::from_rgb(r, g, b)
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let mut visuals = if self.preferences.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let color = self.accent();
        visuals.selection.bg_fill = color.gamma_multiply(0.18);
        visuals.selection.stroke = Stroke::new(1.0, color);
        visuals.hyperlink_color = color;
        visuals.panel_fill = if self.preferences.dark {
            Color32::from_rgb(25, 25, 25)
        } else {
            Color32::WHITE
        };
        visuals.window_fill = visuals.panel_fill;
        let border = if self.preferences.dark {
            Color32::from_gray(54)
        } else {
            Color32::from_gray(222)
        };
        let muted = if self.preferences.dark {
            Color32::from_gray(181)
        } else {
            Color32::from_gray(86)
        };
        visuals.override_text_color = Some(if self.preferences.dark {
            Color32::from_gray(242)
        } else {
            Color32::from_gray(22)
        });
        visuals.weak_text_color = Some(muted);
        visuals.faint_bg_color = if self.preferences.dark {
            Color32::from_gray(31)
        } else {
            Color32::from_gray(249)
        };
        visuals.extreme_bg_color = if self.preferences.dark {
            Color32::from_gray(16)
        } else {
            Color32::from_gray(250)
        };
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border);
        visuals.widgets.inactive.bg_fill = visuals.panel_fill;
        visuals.widgets.inactive.weak_bg_fill = visuals.panel_fill;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, border);
        visuals.widgets.hovered.bg_fill = visuals.faint_bg_color;
        visuals.widgets.hovered.weak_bg_fill = visuals.faint_bg_color;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, muted);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, color);
        let theme = if self.preferences.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        };
        ctx.set_theme(theme);
        ctx.set_visuals(visuals);
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = Vec2::new(8.0, 6.0);
            style.spacing.button_padding = Vec2::new(10.0, 5.0);
            style.spacing.interact_size.y = 28.0;
            style.spacing.scroll.bar_width = 8.0;
            style.spacing.scroll.bar_inner_margin = 2.0;
            style.spacing.scroll.bar_outer_margin = 2.0;
            style.spacing.scroll.floating = false;
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(12.0));
            style
                .text_styles
                .insert(egui::TextStyle::Monospace, egui::FontId::monospace(12.0));
            style
                .text_styles
                .insert(egui::TextStyle::Small, egui::FontId::proportional(11.0));
            style
                .text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(18.0));
        });
    }

    fn log(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.status = message.clone();
        self.activity.insert(0, message);
        self.activity.truncate(30);
    }

    fn set_zoom(&mut self, zoom: f32) {
        let zoom = zoom.clamp(Preferences::MIN_ZOOM, Preferences::MAX_ZOOM);
        if Preferences::valid_zoom(zoom) && self.preferences.zoom != zoom {
            self.preferences.zoom = zoom;
            self.save_pending = true;
        }
    }

    fn zoom_input(&mut self, ctx: &egui::Context) {
        let requested = consume_ui_zoom(ctx, self.preferences.zoom);
        self.set_zoom(requested);
        if (ctx.zoom_factor() - self.preferences.zoom).abs() > 0.001 {
            ctx.set_zoom_factor(self.preferences.zoom);
        }
    }

    fn finish_drag(&mut self, ctx: &egui::Context) {
        let Some(payload) = egui::DragAndDrop::payload::<DragItems>(ctx) else {
            return;
        };
        let hover = ctx.data(|data| data.get_temp::<DropHover>(Id::new("file-drop-hover")));
        let copy = ctx.input(|input| input.modifiers.ctrl);
        let supported = !matches!(
            (&*payload, hover.as_ref().map(|hover| &hover.location)),
            (DragItems::Zip { .. }, Some(DropLocation::Zip { .. }))
        );
        let valid = self.drag_allowed() && hover.is_some() && supported;
        if let Some(hover) = &hover {
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                Id::new("file-drop-target"),
            ))
            .rect_stroke(
                hover.rect.shrink(1.0),
                2.0,
                Stroke::new(
                    2.0,
                    if valid {
                        self.accent()
                    } else {
                        Color32::from_rgb(220, 70, 70)
                    },
                ),
                egui::StrokeKind::Inside,
            );
        }
        ctx.set_cursor_icon(if !valid {
            egui::CursorIcon::NotAllowed
        } else if copy {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::Move
        });
        if let Some(pos) = ctx.input(|input| input.pointer.interact_pos()) {
            let destination = hover
                .as_ref()
                .map(|hover| match &hover.location {
                    DropLocation::Folder(path) => files::display_path(path),
                    DropLocation::Zip { archive, prefix } => {
                        format!("{} / {prefix}", files::display_path(archive))
                    }
                })
                .unwrap_or_else(|| "Drop on a folder or file pane".into());
            egui::Area::new(Id::new("file-drag-label"))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .fixed_pos(pos + Vec2::new(18.0, 18.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(format!(
                            "{} / {} item(s)",
                            if !valid {
                                "Cannot drop"
                            } else if copy {
                                "+ COPY"
                            } else {
                                "MOVE"
                            },
                            payload.count()
                        ));
                        ui.add(egui::Label::new(destination).truncate());
                        ui.label("Ctrl = Copy / Escape = Cancel / approval before effects");
                    });
                });
        }
        if ctx.input(|input| input.pointer.button_released(egui::PointerButton::Primary)) {
            egui::DragAndDrop::clear_payload(ctx);
            if let Some(hover) = hover {
                if !valid {
                    self.fail(if supported {
                        "Finish current work before dropping files."
                    } else {
                        "ZIP-to-ZIP dragging is unsupported; extract to a real folder first."
                    });
                    return;
                }
                self.prepare_drop((*payload).clone(), hover.location, copy);
            }
        }
    }

    fn drag_allowed(&self) -> bool {
        !self.operation_busy
            && self.plan.is_none()
            && self.change_plan.is_none()
            && self.zip_dialog.is_none()
            && self.name_dialog.is_none()
            && self.open_with.is_none()
            && !self.settings_open
            && !self.operations_open
            && self.error.is_none()
            && !self.palette
            && self.search_dialog.is_none()
            && self.command_dialog.is_none()
            && !self.ai.open
            && !self.answers.open
            && !self.help_open
            && self.places_popup.is_none()
            && self.place_edit.is_none()
            && !self.reader.dirty()
            && !self.panes.iter().any(|pane| pane.busy)
    }

    fn prepare_drop(&mut self, items: DragItems, target: DropLocation, copy: bool) {
        if !self.drag_allowed() {
            self.fail("Finish current work before dropping files.");
            return;
        }
        match drop_work(self.next_id, items, target, copy) {
            Ok(Some(job)) => {
                self.reader.close();
                self.cancellation.store(false, Ordering::Release);
                match self.work.try_send(job) {
                    Ok(()) => {
                        self.next_id += 1;
                        self.operation_busy = true;
                        self.log("Preparing drag-and-drop plan / no effects before approval.");
                    }
                    Err(error) => self.fail(format!("Could not queue drop plan: {error}")),
                }
            }
            Ok(None) => self.log("Drop cancelled: source is already here."),
            Err(error) => self.fail(error),
        }
    }

    fn fail(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.log(format!("ERROR / {message}"));
        self.error = Some(message);
    }

    fn navigate(&mut self, index: usize, path: PathBuf) {
        self.inline_pending = None;
        if self
            .name_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.inline)
        {
            self.name_dialog = None;
        }
        let worker = &mut self.scans[index];
        let generation = worker.generation.fetch_add(1, Ordering::Relaxed) + 1;
        worker.pending = Some(ScanRequest {
            path: path.clone(),
            generation,
        });
        self.panes[index].begin_navigation(path);
        self.file_focus_requested = true;
    }

    fn parent(&mut self, index: usize) {
        let path = self.panes[index].path.clone();
        if self.scope.root() == Some(path.as_path()) || path.parent().is_none() {
            self.log("Parent unavailable at filesystem root or fixture boundary.");
        } else if let Some(parent) = path.parent() {
            let history = self.panes[index].remember_parent_return();
            self.navigate(index, parent.to_owned());
            self.panes[index].parent_returns = history;
        }
    }

    fn return_inside(&mut self, index: usize) {
        let Some(location) = self.panes[index].parent_returns.last() else {
            return;
        };
        let path = match self.scope.resolve(&location.path) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => {
                self.fail("The saved return folder is no longer a directory.");
                return;
            }
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let mut history = std::mem::take(&mut self.panes[index].parent_returns);
        let location = history.pop().expect("Return history was checked above");
        self.navigate(index, path);
        let pane = &mut self.panes[index];
        pane.parent_returns = history;
        pane.focused = location.focused;
        pane.filter = location.filter;
        pane.scroll_offset = location.scroll_offset;
        pane.scroll_to_focus = false;
    }

    fn dispatch(&mut self, command: Command) {
        self.inline_pending = None;
        self.palette = false;
        if matches!(
            command,
            Command::ZoomIn | Command::ZoomOut | Command::ZoomReset
        ) {
            let zoom = match command {
                Command::ZoomIn => self.preferences.zoom + 0.1,
                Command::ZoomOut => self.preferences.zoom - 0.1,
                _ => 1.0,
            };
            self.set_zoom(zoom);
            return;
        }
        if self.change_plan.is_some() {
            self.fail("Confirm or cancel the pending exact source change first.");
            return;
        }
        if self.archive.is_archive() && matches!(command, Command::Copy | Command::Move) {
            if self.operation_busy || self.plan.is_some() || self.reader.dirty() {
                self.fail("Finish pending work and Save/discard edits before a ZIP operation.");
                return;
            }
            let (archive, prefix) = self.archive.archive_location();
            let request = if self.archive.target == Some(self.active) {
                tomas_commander::archive_changes::Request::Extract {
                    archive,
                    prefix,
                    selected: self.archive.archive_targets(),
                    destination: self.panes[1 - self.active].path.clone(),
                    move_entries: matches!(command, Command::Move),
                }
            } else {
                if self.reader.target == Some(self.active) {
                    self.fail("Close the preview before choosing files to insert into ZIP.");
                    return;
                }
                tomas_commander::archive_changes::Request::Insert {
                    archive,
                    prefix,
                    sources: self.panes[self.active].targets(),
                    move_sources: matches!(command, Command::Move),
                }
            };
            self.prepare_archive(request);
            return;
        }
        if self.archive.target == Some(self.active) {
            if matches!(command, Command::Preview) {
                self.archive.request_archive_preview();
                return;
            }
            if matches!(command, Command::Refresh) {
                if let Err(error) = self.archive.refresh_archive() {
                    self.fail(error);
                }
                return;
            }
            if !matches!(
                command,
                Command::Help
                    | Command::Settings
                    | Command::Theme
                    | Command::Accent
                    | Command::Foundry
                    | Command::Answers
                    | Command::Activity
                    | Command::Balance
            ) {
                self.fail("This command requires a regular filesystem pane; close ZIP or Tab to the other pane.");
                return;
            }
        }
        if self.reader.target == Some(self.active)
            && !matches!(
                command,
                Command::Help
                    | Command::Settings
                    | Command::Theme
                    | Command::Accent
                    | Command::Foundry
                    | Command::Answers
                    | Command::Activity
                    | Command::Balance
                    | Command::RotateLeft
                    | Command::RotateRight
                    | Command::EditSource
                    | Command::SaveSource
            )
        {
            self.fail("Tab back to the file pane before using a file command.");
            return;
        }
        match command {
            Command::Settings => self.settings_open = true,
            Command::OpenWith => {
                if self.operation_busy
                    || self.plan.is_some()
                    || self.reader.dirty()
                    || self.panes[self.active].busy
                {
                    self.fail("Finish pending work and edits before opening a workspace.");
                    return;
                }
                let pane = &self.panes[self.active];
                let folder = pane
                    .focused
                    .as_ref()
                    .filter(|path| {
                        pane.entries
                            .iter()
                            .any(|entry| &entry.path == *path && entry.directory && !entry.link)
                    })
                    .cloned()
                    .unwrap_or_else(|| pane.path.clone());
                self.open_with = Some(OpenWith { folder, index: 0 });
            }
            Command::Rename | Command::NewFolder => {
                if self.operation_busy
                    || self.plan.is_some()
                    || self.reader.dirty()
                    || self.panes[self.active].busy
                {
                    self.fail("Finish pending work and edits before changing names.");
                    return;
                }
                let pane = &self.panes[self.active];
                let source = if matches!(command, Command::Rename) {
                    let Some(path) = pane.focused.clone() else {
                        self.fail("Focus one file or folder to rename.");
                        return;
                    };
                    Some(path)
                } else {
                    None
                };
                let name = source
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.name_dialog = Some(NameDialog {
                    folder: pane.path.clone(),
                    source,
                    name,
                    focus: true,
                    error: None,
                    inline: false,
                    pane: self.active,
                });
            }
            Command::ClearSelection => {
                self.panes[self.active].selected.clear();
                self.log("Selection cleared.");
            }
            Command::ZoomIn | Command::ZoomOut | Command::ZoomReset => unreachable!(),
            Command::CreateZip => {
                if self.operation_busy
                    || self.plan.is_some()
                    || self.reader.dirty()
                    || self.archive.is_archive()
                {
                    self.fail("Finish work and use a regular filesystem pane before creating ZIP.");
                    return;
                }
                let sources = self.panes[self.active].targets();
                if sources.is_empty() {
                    self.fail("Select files or folders to archive.");
                    return;
                }
                let name = sources[0]
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Archive");
                self.zip_dialog = Some(ZipDialog {
                    name: format!("{name}.zip"),
                    sources,
                    destination: self.panes[1 - self.active].path.clone(),
                    focus: true,
                });
            }
            Command::EditSource => {
                if self.operation_busy || self.plan.is_some() {
                    self.fail("Finish the pending operation before editing.");
                    return;
                }
                let result = if self.reader.target.is_some() {
                    self.reader.begin_edit()
                } else if let Some(path) = self.panes[self.active].focused.clone() {
                    self.reader.start_edit(self.active, path)
                } else {
                    Err("Choose a text file before editing.".into())
                };
                if let Err(error) = result {
                    self.fail(error);
                }
            }
            Command::SaveSource => {
                if self.operation_busy || self.plan.is_some() {
                    self.fail("Finish the pending operation before requesting Save.");
                    return;
                }
                match self.reader.save_plan(self.next_id) {
                    Ok(plan) => {
                        self.next_id += 1;
                        self.change_plan = Some(Change::Save(plan));
                        self.approval_focus = true;
                    }
                    Err(error) => self.fail(error),
                }
            }
            Command::RotateLeft | Command::RotateRight => {
                if let Err(error) = self.reader.rotate(matches!(command, Command::RotateRight)) {
                    self.fail(error);
                }
            }
            Command::Preview => {
                if self.operation_busy || self.plan.is_some() {
                    self.fail("Finish or cancel the file operation before previewing.");
                    return;
                }
                let path = self.panes[self.active].focused.clone();
                match path {
                    Some(path) => {
                        if let Err(error) = self.reader.toggle(self.active, path) {
                            self.fail(error);
                        }
                    }
                    None => self.fail("Choose a regular file, not the parent row, for preview."),
                }
            }
            Command::Parent => self.parent(self.active),
            Command::ReturnInside => self.return_inside(self.active),
            Command::JumpUp | Command::JumpDown => {
                self.panes[self.active].jump(matches!(command, Command::JumpDown));
                self.file_focus_requested = true;
            }
            Command::Places => {
                self.rail_index = None;
                self.places_popup = Some(0);
            }
            Command::Help => self.help_open = true,
            Command::AiPlan => {
                if self.operation_busy
                    || self.ai.busy()
                    || self.plan.is_some()
                    || self.panes.iter().any(|pane| pane.busy)
                {
                    self.fail("Wait for current work before opening a natural-language command.");
                    return;
                }
                self.command_dialog = Some(CommandDialog {
                    pane: self.active,
                    roots: [
                        self.panes[self.active].path.clone(),
                        self.panes[1 - self.active].path.clone(),
                    ],
                    prompt: String::new(),
                    consent: false,
                    focus_prompt: true,
                    interpreting: false,
                    resolving: false,
                    resolution: None,
                    source: None,
                    destination: None,
                    details: None,
                    error: None,
                });
            }
            Command::Foundry => self.ai.open = true,
            Command::Answers => self.answers.open = true,
            Command::Copy | Command::Move | Command::Recycle => {
                if self.reader.dirty() {
                    self.fail("Save or explicitly discard edits before planning a file mutation.");
                    return;
                }
                self.reader.close();
                if self.operation_busy || self.plan.is_some() {
                    self.fail("Finish or cancel the current operation first.");
                    return;
                }
                if self.panes.iter().any(|pane| pane.busy) {
                    self.fail("Wait for folder loading before planning an operation.");
                    return;
                }
                let operation = match command {
                    Command::Copy => Operation::Copy,
                    Command::Move => Operation::Move,
                    _ => Operation::Recycle,
                };
                let sources = self.panes[self.active].targets();
                if sources.is_empty() {
                    self.fail("Select or focus an item first.");
                    return;
                }
                self.cancellation.store(false, Ordering::Relaxed);
                let job = Work::Prepare {
                    id: self.next_id,
                    operation,
                    sources,
                    destination: self.panes[1 - self.active].path.clone(),
                };
                self.next_id += 1;
                match self.work.try_send(job) {
                    Ok(()) => {
                        self.operation_busy = true;
                        self.log("Preparing a real operation plan...");
                    }
                    Err(error) => self.fail(format!("Could not queue operation: {error}")),
                }
            }
            Command::Refresh => {
                if let Err(error) = self.aggregate_cache.clear() {
                    self.fail(error);
                    return;
                }
                self.navigate(self.active, self.panes[self.active].path.clone());
            }
            Command::Favorites => {
                self.rail = Rail::Favorites;
                self.rail_index = Some(0);
            }
            Command::Recent => {
                self.rail = Rail::Recent;
                self.rail_index = Some(0);
            }
            Command::Activity => {
                self.operations_open = true;
            }
            Command::Theme => {
                self.preferences.dark = !self.preferences.dark;
                self.appearance_dirty = true;
                self.save_pending = true;
            }
            Command::Accent => {
                self.preferences.accent = (self.preferences.accent + 1) % ACCENTS.len();
                self.appearance_dirty = true;
                self.save_pending = true;
            }
            Command::Pin => match self.scope.resolve(&self.panes[self.active].path) {
                Ok(path) if path.is_dir() => {
                    if !self.preferences.favorites.contains(&path) {
                        self.preferences.favorites.push(path);
                        self.save_pending = true;
                        self.log("Current folder added to saved places.");
                    } else {
                        self.log("Current folder is already in saved places.");
                    }
                }
                Ok(_) => self.fail("Current location is no longer a directory."),
                Err(error) => self.fail(error),
            },
            Command::Sort => {
                let pane = &mut self.panes[self.active];
                if pane.descending {
                    pane.sort = match pane.sort {
                        Sort::Name => Sort::Size,
                        Sort::Size => Sort::Modified,
                        Sort::Modified => Sort::Name,
                    };
                }
                pane.descending = !pane.descending;
                files::sort_entries_ordered(&mut pane.entries, pane.sort, pane.descending);
                pane.rebuild();
                pane.scroll_to_focus = true;
            }
            Command::Filter => self.editor_focus = Some(Id::new(("filter", self.active))),
            Command::Search | Command::AiSearch => {
                self.search_dialog = Some(SearchDialog {
                    pane: self.active,
                    root: self.panes[self.active].path.clone(),
                    query: String::new(),
                    focus_query: true,
                    busy: false,
                    index: 0,
                    results: None,
                    error: None,
                    scroll_offset: 0.0,
                    ai_assisted: matches!(command, Command::AiSearch),
                    consent: false,
                    interpreting: false,
                    interpretation: None,
                });
            }
            Command::Balance => self.balance_requested = true,
            Command::Code => {
                if let Err(error) = self
                    .work
                    .try_send(Work::Code(self.panes[self.active].path.clone()))
                {
                    self.fail(format!("Could not queue VS Code launch: {error}"));
                }
            }
            Command::CopilotCli | Command::CopilotApp => {
                if let Err(error) = self.work.try_send(Work::Copilot {
                    path: self.panes[self.active].path.clone(),
                    app: matches!(command, Command::CopilotApp),
                }) {
                    self.fail(format!("Could not queue Copilot launch: {error}"));
                }
            }
        }
    }

    fn finish_operation(&mut self, outcome: Outcome) {
        self.operation_busy = false;
        self.log(format!(
            "Operation #{}: {} completed nodes, {} created paths, {} not completed{}",
            outcome.id,
            outcome.completed.len(),
            outcome.created.len(),
            outcome.incomplete.len(),
            if outcome.cancelled {
                " / cancelled"
            } else {
                ""
            }
        ));
        if let Some(error) = &outcome.error {
            self.fail(error.clone());
        }
        if outcome.cancelled && !outcome.created.is_empty() {
            self.fail("Cancelled operation left explicitly created partial destinations. Inspect them; no automatic deletion was performed.");
        }
        if let Err(error) = self.aggregate_cache.clear() {
            self.fail(error);
        }
        for index in 0..2 {
            self.navigate(index, self.panes[index].path.clone());
        }
        self.last_outcome = Some(outcome);
    }

    fn prepare_archive(&mut self, request: tomas_commander::archive_changes::Request) {
        self.cancellation.store(false, Ordering::Relaxed);
        let id = self.next_id;
        self.next_id += 1;
        match self.work.try_send(Work::PrepareArchive { id, request }) {
            Ok(()) => {
                self.operation_busy = true;
                self.log("Preparing exact ZIP plan; no filesystem effects...");
            }
            Err(error) => self.fail(format!("ZIP plan was not queued: {error}")),
        }
    }

    fn open_focused(&mut self) {
        let pane = &self.panes[self.active];
        if pane.focused.is_none() {
            self.parent(self.active);
            return;
        }
        if let Some(path) = &pane.focused {
            let entry = pane.entries.iter().find(|entry| &entry.path == path);
            if let Some(entry) = entry {
                if entry.directory {
                    self.navigate(self.active, path.clone());
                } else if path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
                {
                    if let Err(error) = self.archive.open_archive(self.active, path.clone()) {
                        self.fail(error);
                    }
                } else {
                    if let Err(error) = self.work.try_send(Work::Open(path.clone())) {
                        self.fail(format!("Could not queue file opening: {error}"));
                    }
                }
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self
            .name_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.inline)
        {
            if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
                self.name_dialog = None;
                self.file_focus_requested = true;
            }
            return;
        }
        if self.change_plan.is_some()
            || self.zip_dialog.is_some()
            || self.name_dialog.is_some()
            || self.open_with.is_some()
            || self.settings_open
            || self.operations_open
            || self.ai.open
            || self.answers.open
            || self.help_open
            || self.places_popup.is_some()
            || self.place_edit.is_some()
        {
            return;
        }
        let editing = ctx.memory(|memory| {
            if memory.has_focus(Id::new("reader-editor")) {
                return true;
            }
            if memory.has_focus(Id::new("archive-filter")) {
                return true;
            }
            (0..2).any(|pane| {
                memory.has_focus(Id::new(("path", pane)))
                    || memory.has_focus(Id::new(("filter", pane)))
            })
        });
        let copy = consume_file_copy(
            ctx,
            &mut self.copy_modifiers,
            !editing
                && self.reader.target != Some(self.active)
                && self.rail_index.is_none()
                && !self.palette
                && self.plan.is_none()
                && self.error.is_none()
                && self.search_dialog.is_none()
                && self.command_dialog.is_none(),
        );
        if self.plan.is_some()
            || self.error.is_some()
            || self.search_dialog.is_some()
            || self.command_dialog.is_some()
        {
            return;
        }
        let chord = Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        if ctx.input_mut(|input| input.consume_key(chord, Key::P)) {
            self.palette = !self.palette;
            self.palette_focus = self.palette;
            self.palette_query.clear();
            self.palette_index = 0;
        }
        if self.palette || self.plan.is_some() || self.error.is_some() {
            return;
        }
        for (key, command, rail) in [
            (Key::B, Command::Favorites, Rail::Favorites),
            (Key::H, Command::Recent, Rail::Recent),
        ] {
            if ctx.input_mut(|input| input.consume_key(chord, key)) {
                if self.rail_index.is_some() && self.rail == rail {
                    self.rail_index = None;
                    self.file_focus_requested = true;
                } else {
                    ctx.memory_mut(|memory| {
                        for pane in 0..2 {
                            memory.surrender_focus(Id::new(("path", pane)));
                            memory.surrender_focus(Id::new(("filter", pane)));
                        }
                        memory.move_focus(egui::FocusDirection::None);
                    });
                    self.dispatch(command);
                    self.file_focus_requested = false;
                }
                return;
            }
        }
        if self.reader.target.is_some()
            && (!editing || self.reader.target == Some(self.active))
            && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.reader.close();
            if self.reader.target.is_none() {
                self.active = self.reader.source;
                self.file_focus_requested = true;
            }
            return;
        }
        if self.reader.target == Some(self.active) {
            for (key, command) in [(Key::S, Command::SaveSource), (Key::E, Command::EditSource)] {
                if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, key)) {
                    self.dispatch(command);
                    return;
                }
            }
        }
        if self.archive.target == Some(self.active)
            && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.archive.close();
            if self.reader.is_virtual() {
                self.reader.close();
            }
            self.file_focus_requested = true;
            return;
        }
        if editing {
            return;
        }
        if consume_help(ctx) {
            self.dispatch(Command::Help);
            return;
        }
        if self.reader.target == Some(self.active) {
            if !self.reader.is_virtual()
                && ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::Enter))
            {
                self.reader.close();
                if self.reader.target.is_none() {
                    self.active = self.reader.source;
                    self.file_focus_requested = true;
                }
                return;
            }
            for (key, delta) in [(Key::PageUp, -1), (Key::PageDown, 1)] {
                if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, key)) {
                    if let Err(error) = self.reader.step_pdf(delta) {
                        self.fail(error);
                    }
                    return;
                }
            }
            if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab)) {
                self.active = 1 - self.active;
                self.file_focus_requested = true;
            }
            if self.reader.is_archive() {
                if self.reader.archive_keys(ctx) {
                    self.active = self.reader.source;
                    self.reader.close();
                    self.file_focus_requested = true;
                }
                return;
            }
            for (key, command) in [
                (Key::ArrowLeft, Command::RotateLeft),
                (Key::ArrowRight, Command::RotateRight),
            ] {
                if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, key)) {
                    self.dispatch(command);
                    return;
                }
            }
            for (key, delta) in [
                (Key::ArrowUp, -30.0),
                (Key::ArrowDown, 30.0),
                (Key::PageUp, -300.0),
                (Key::PageDown, 300.0),
            ] {
                if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, key)) {
                    ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                    self.reader.scroll(delta);
                }
            }
            return;
        }
        if self.archive.target == Some(self.active) {
            if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::F)) {
                ctx.memory_mut(|memory| memory.request_focus(Id::new("archive-filter")));
                return;
            }
            if copy || ctx.input_mut(|input| input.consume_key(chord, Key::C)) {
                self.dispatch(Command::Copy);
                return;
            }
            if ctx.input_mut(|input| input.consume_key(chord, Key::M)) {
                self.dispatch(Command::Move);
                return;
            }
            if ctx.input_mut(|input| input.consume_key(chord, Key::D)) {
                self.fail("ZIP member deletion is not a Recycle Bin operation; use an approved Move to a folder.");
                return;
            }
            if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab)) {
                self.active = 1 - self.active;
                return;
            }
            if self.archive.archive_keys(ctx) {
                self.archive.close();
                if self.reader.is_virtual() {
                    self.reader.close();
                }
                self.file_focus_requested = true;
            }
            return;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::Enter)) {
            self.dispatch(Command::Preview);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::SHIFT, Key::Enter)) {
            self.dispatch(Command::OpenWith);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(chord, Key::N)) {
            self.dispatch(Command::Rename);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::N)) {
            self.dispatch(Command::NewFolder);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::ALT, Key::ArrowUp)) {
            self.dispatch(Command::Places);
            return;
        }
        if let Some(index) = self.rail_index {
            let mut next = index;
            if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                next = next.saturating_sub(1);
                ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            }
            if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                next += 1;
                ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            }
            let paths = if self.rail == Rail::Recent {
                &self.preferences.recent_documents
            } else {
                &self.preferences.favorites
            };
            next = next.min(paths.len().saturating_sub(1));
            self.rail_index = Some(next);
            if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter)) {
                let path = paths.get(next).cloned();
                self.rail_index = None;
                self.file_focus_requested = true;
                if let Some(path) = path {
                    if self.rail == Rail::Recent {
                        if let Err(error) = self.work.try_send(Work::Open(path)) {
                            self.fail(format!("Could not open recent document: {error}"));
                        }
                    } else {
                        self.navigate(self.active, path);
                    }
                }
            }
            if ctx.input_mut(|input| {
                input.consume_key(Modifiers::NONE, Key::Escape)
                    || input.consume_key(Modifiers::NONE, Key::Tab)
            }) {
                self.rail_index = None;
                self.file_focus_requested = true;
            }
            return;
        }
        if copy {
            self.dispatch(Command::Copy);
            return;
        }
        for (key, command) in [
            (Key::ArrowLeft, Command::Parent),
            (Key::ArrowRight, Command::ReturnInside),
            (Key::ArrowUp, Command::JumpUp),
            (Key::ArrowDown, Command::JumpDown),
        ] {
            if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, key)) {
                ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                self.dispatch(command);
                return;
            }
        }
        for (key, command) in [
            (Key::C, Command::Copy),
            (Key::M, Command::Move),
            (Key::S, Command::Sort),
            (Key::F, Command::Search),
            (Key::D, Command::Recycle),
        ] {
            if ctx.input_mut(|input| input.consume_key(chord, key)) {
                self.dispatch(command);
                return;
            }
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::F)) {
            self.dispatch(Command::Filter);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::R)) {
            self.dispatch(Command::Refresh);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(chord, Key::ArrowLeft)) {
            self.left_width = (self.left_width - 35.0).max(240.0);
            self.resize_requested = true;
        }
        if ctx.input_mut(|input| input.consume_key(chord, Key::ArrowRight)) {
            self.left_width += 35.0;
            self.resize_requested = true;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab)) {
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            self.active = 1 - self.active;
            self.file_focus_requested = true;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter)) {
            self.open_focused();
            return;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Backspace)) {
            self.parent(self.active);
            return;
        }
        let pane = &mut self.panes[self.active];
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            pane.selected.clear();
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::A)) {
            pane.selected = pane
                .visible
                .iter()
                .map(|index| pane.entries[*index].path.clone())
                .collect();
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Space))
            && let Some(path) = &pane.focused
            && !pane.selected.remove(path)
        {
            pane.selected.insert(path.clone());
        }
        for key in [Key::ArrowUp, Key::ArrowDown, Key::Home, Key::End] {
            let Some(modifiers) = consume_navigation_key(ctx, key) else {
                continue;
            };
            // egui resolves focus traversal from raw input before consume_key.
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            let previous = pane.focus_position();
            let position = match key {
                Key::ArrowUp => previous.saturating_sub(1),
                Key::ArrowDown => (previous + 1).min(pane.visible.len()),
                Key::Home => 0,
                _ => pane.visible.len(),
            };
            pane.focused = pane.row_path(position);
            if modifiers.shift {
                pane.selected = (pane.anchor.min(position)..=pane.anchor.max(position))
                    .filter_map(|position| pane.row_path(position))
                    .collect();
            } else {
                pane.anchor = position;
            }
            pane.scroll_to_focus = true;
            self.file_focus_requested = true;
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.cache_changes.try_recv() {
            match result {
                Ok(changes) => {
                    for warning in changes.warnings {
                        self.log(warning);
                    }
                    let mut refreshed = false;
                    for index in 0..2 {
                        if changes.all
                            || self.panes[index]
                                .entries
                                .iter()
                                .any(|entry| changes.paths.contains(&entry.path))
                        {
                            self.navigate(index, self.panes[index].path.clone());
                            refreshed = true;
                        }
                    }
                    if refreshed {
                        self.log("Folder changed / refreshing cached sizes and dates.");
                    }
                }
                Err(error) => self.fail(error),
            }
        }
        if let Some(result) = self.ai.take_command_result()
            && let Some(dialog) = &mut self.command_dialog
            && dialog.interpreting
        {
            dialog.interpreting = false;
            match result {
                Ok((intent, reply)) => {
                    dialog.details = Some(format!(
                        "{}\nValidated intent: {}\nAuth {} ms / provider {} ms / guards {} us\nRequest {} / response {}",
                        self.ai.status(),
                        reply.text,
                        reply.authentication_ms,
                        reply.provider_ms,
                        reply.local_us,
                        reply.request_id,
                        reply.response_id
                    ));
                    self.resolve_command_intent(Ok(intent));
                }
                Err(error) => dialog.error = Some(error),
            }
        }
        if let Some(result) = self.ai.take_search_result()
            && let Some(dialog) = &mut self.search_dialog
            && dialog.interpreting
        {
            dialog.interpreting = false;
            match result {
                Ok((intent, reply)) => {
                    dialog.interpretation = Some(format!(
                        "{} / auth {} ms / provider {} ms / guards {} us / request {} / response {}\nName matching only; no image/content understanding. Interpretation may miss concepts.",
                        self.ai.status(),
                        reply.authentication_ms,
                        reply.provider_ms,
                        reply.local_us,
                        reply.request_id,
                        reply.response_id
                    ));
                    if let Some(details) = &mut dialog.interpretation {
                        details.push_str("\nValidated intent: ");
                        details.push_str(&reply.text);
                    }
                    match intent.status {
                        search::IntentStatus::Ambiguous => {
                            dialog.busy = false;
                            dialog.error = Some("Ambiguous prompt. Specify filename concepts, folder terms or a file type. No scan started.".into());
                        }
                        search::IntentStatus::Unsupported => {
                            dialog.busy = false;
                            dialog.error = Some("Unsupported request. Only filename/folder/type discovery is available. No scan started.".into());
                        }
                        search::IntentStatus::Ready => {
                            let generation =
                                self.search_generation.fetch_add(1, Ordering::Relaxed) + 1;
                            match self.search_jobs.try_send(SearchRequest {
                                generation,
                                root: dialog.root.clone(),
                                query: String::new(),
                                intent: Some(intent),
                                #[cfg(feature = "native-search-tests")]
                                slow_test_walk: self.ai.test_scenario
                                    == Some(crate::ai_ui::SearchTestScenario::LocalCancel),
                            }) {
                                Ok(()) => (),
                                Err(error) => {
                                    dialog.busy = false;
                                    dialog.error =
                                        Some(format!("Local search was not queued: {error}"));
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    dialog.busy = false;
                    dialog.error = Some(error);
                }
            }
        }
        while let Ok((generation, result)) = self.search_results.try_recv() {
            if generation != self.search_generation.load(Ordering::Relaxed) {
                continue;
            }
            if let Some(dialog) = &mut self.search_dialog {
                dialog.busy = false;
                match result {
                    Ok(results) => dialog.results = Some(results),
                    Err(error) => dialog.error = Some(error),
                }
            }
        }
        for index in 0..2 {
            if let Some(request) = self.scans[index].pending.take() {
                match self.scans[index].sender.try_send(request) {
                    Ok(()) => (),
                    Err(TrySendError::Full(request)) => {
                        self.scans[index].pending = Some(request);
                        ctx.request_repaint_after(Duration::from_millis(40));
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        self.fail("Directory worker disconnected.")
                    }
                }
            }
        }
        let mut aggregate_changed = [false; 2];
        while let Ok(result) = self.scan_results.try_recv() {
            if self.scans[result.pane].generation.load(Ordering::Relaxed) != result.generation {
                continue;
            }
            let pane = &mut self.panes[result.pane];
            let listing = match result.content {
                ScanContent::Listing(listing) => listing,
                ScanContent::Aggregate {
                    path,
                    result: stats,
                } => {
                    if let Some(entry) = pane.entries.iter_mut().find(|entry| entry.path == path) {
                        entry.aggregate = Some(stats);
                        aggregate_changed[result.pane] = true;
                    }
                    continue;
                }
            };
            pane.busy = false;
            match listing {
                Ok(mut listing) => {
                    pane.path = listing.path;
                    pane.path_text = files::display_path(&pane.path);
                    files::sort_entries_ordered(&mut listing.entries, pane.sort, pane.descending);
                    pane.entries = listing.entries;
                    let paths: BTreeSet<_> = pane
                        .entries
                        .iter()
                        .map(|entry| entry.path.clone())
                        .collect();
                    pane.selected.retain(|path| paths.contains(path));
                    pane.rebuild();
                    pane.anchor = pane.focus_position();
                    pane.warning = if listing.warnings.is_empty() {
                        None
                    } else {
                        Some(listing.warnings.join("\n"))
                    };
                    self.status = format!("{} items / ready", pane.entries.len());
                    if result.pane == self.active {
                        self.file_focus_requested = true;
                    }
                }
                Err(error) => {
                    pane.warning = Some(error.clone());
                    self.fail(error);
                }
            }
        }
        for (index, changed) in aggregate_changed.into_iter().enumerate() {
            if changed {
                let pane = &mut self.panes[index];
                files::sort_entries_ordered(&mut pane.entries, pane.sort, pane.descending);
                pane.rebuild();
                pane.scroll_to_focus = true;
            }
        }
        while let Ok(result) = self.work_results.try_recv() {
            match result {
                WorkResult::CommandResolved(result) => {
                    self.operation_busy = false;
                    if let Some(dialog) = &mut self.command_dialog {
                        dialog.resolving = false;
                        if self.cancellation.load(Ordering::Acquire) {
                            dialog.error =
                                Some("Command discovery cancelled. No plan or effects.".into());
                        } else {
                            match result {
                                Ok(resolution) => {
                                    dialog.source = (resolution.sources().len() == 1).then_some(0);
                                    dialog.destination =
                                        (resolution.destinations().len() == 1).then_some(0);
                                    dialog.resolution = Some(resolution);
                                }
                                Err(error) => dialog.error = Some(error),
                            }
                        }
                    }
                }
                WorkResult::Recovery(error) => self.fail(error),
                #[cfg(feature = "native-desktop-tests")]
                WorkResult::DesktopRejections(result) => {
                    self.operation_busy = false;
                    if let Some(dialog) = &mut self.command_dialog {
                        dialog.resolving = false;
                        match result {
                            Ok(report) => dialog.details = Some(report),
                            Err(error) => dialog.error = Some(error),
                        }
                    }
                }
                WorkResult::Prepared(result) => {
                    self.operation_busy = false;
                    if self.cancellation.load(Ordering::Relaxed) {
                        self.log("Planning cancelled. No changes made.");
                        continue;
                    }
                    match result {
                        Ok(plan) => {
                            self.plan = Some(plan);
                            self.approval_focus = true;
                        }
                        Err(error) => self.fail(error),
                    }
                }
                WorkResult::NamePrepared(result) => {
                    self.operation_busy = false;
                    match result {
                        Ok(plan) => {
                            self.change_plan = Some(Change::Name(plan));
                            self.approval_focus = true;
                        }
                        Err(error) => self.fail(error),
                    }
                }
                WorkResult::ArchivePrepared(result) => {
                    self.operation_busy = false;
                    if self.cancellation.load(Ordering::Acquire) {
                        self.log("ZIP planning cancelled; no effects.");
                    } else {
                        match result {
                            Ok(plan) => {
                                self.change_plan = Some(Change::Archive(plan));
                                self.approval_focus = true;
                            }
                            Err(error) => self.fail(error),
                        }
                    }
                }
                WorkResult::Changed { outcome, editor } => {
                    if editor
                        && outcome.error.is_none()
                        && !outcome.cancelled
                        && let Err(error) = self.reader.saved()
                    {
                        self.fail(error);
                    }
                    if !editor && self.archive.is_archive() {
                        if self.reader.is_virtual() {
                            self.reader.close();
                        }
                        if let Err(error) = self.archive.refresh_archive() {
                            self.fail(error);
                        }
                    }
                    self.finish_operation(outcome);
                }
                WorkResult::Executed(outcome) => self.finish_operation(outcome),
                WorkResult::Saved(result) => {
                    self.saves_in_flight -= 1;
                    if let Err(error) = result {
                        self.close_after_save = false;
                        self.fail(error);
                    }
                }
                WorkResult::Code(result) => match result {
                    Ok(()) => self.log("VS Code launch requested for the actual folder."),
                    Err(error) => self.fail(error),
                },
                WorkResult::Open(result) => match result {
                    Ok(path) => {
                        if self.preferences.record_document_open(path) {
                            self.save_pending = true;
                        }
                        self.log("Document opening requested.");
                    }
                    Err(error) => self.fail(error),
                },
                WorkResult::Copilot { app, result } => match result {
                    Ok(()) => self.log(if app {
                        "GitHub Copilot App launch requested."
                    } else {
                        "GitHub Copilot CLI terminal requested (--yolo)."
                    }),
                    Err(error) => self.fail(error),
                },
                WorkResult::Located { pane, result } => match result {
                    Ok(path) => {
                        if let Some(parent) = path.parent() {
                            self.active = pane;
                            self.navigate(pane, parent.to_owned());
                            self.panes[pane].focused = Some(path);
                            self.file_focus_requested = true;
                        }
                    }
                    Err(error) => self.fail(error),
                },
            }
        }
        if self.save_pending && self.preferences_writable {
            match self.work.try_send(Work::Save(self.preferences.clone())) {
                Ok(()) => {
                    self.save_pending = false;
                    self.saves_in_flight += 1;
                }
                Err(TrySendError::Full(_)) => ctx.request_repaint_after(Duration::from_millis(50)),
                Err(TrySendError::Disconnected(_)) => {
                    self.save_pending = false;
                    self.close_after_save = false;
                    self.fail("Preferences worker disconnected.");
                }
            }
            if self.close_after_save && !self.save_pending && self.saves_in_flight == 0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn pane_ui(&mut self, ui: &mut egui::Ui, index: usize) {
        if self.archive.target == Some(index) {
            ui.set_min_width(ui.available_width());
            if ui.rect_contains_pointer(ui.max_rect())
                && ui.input(|input| input.pointer.any_pressed())
            {
                self.active = index;
                self.rail_index = None;
            }
            if self.archive.show(ui) {
                self.archive.close();
                if self.reader.is_virtual() {
                    self.reader.close();
                }
                self.active = index;
                self.file_focus_requested = true;
            }
            let (archive, prefix) = self.archive.archive_location();
            if self.archive.is_archive() {
                register_drop_zone(
                    ui.ctx(),
                    ui.max_rect(),
                    DropLocation::Zip { archive, prefix },
                    0,
                );
            }
            return;
        }
        if self.reader.target == Some(index) {
            ui.set_min_width(ui.available_width());
            if ui.is_enabled()
                && ui.rect_contains_pointer(ui.max_rect())
                && ui.input(|input| input.pointer.any_pressed())
            {
                self.active = index;
            }
            if self.reader.show(ui) {
                self.reader.close();
                if self.reader.target.is_none() {
                    self.active = self.reader.source;
                    self.file_focus_requested = true;
                }
            }
            return;
        }
        if ui.input(|input| {
            input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|position| ui.max_rect().contains(position))
        }) {
            self.active = index;
            self.rail_index = None;
            self.file_focus_requested = false;
            ui.memory_mut(|memory| {
                for pane in 0..2 {
                    memory.surrender_focus(Id::new(("path", pane)));
                    memory.surrender_focus(Id::new(("filter", pane)));
                }
            });
        }
        let accent = self.accent();
        let active = self.active == index;
        let surface = ui.visuals().panel_fill;
        let muted = ui.visuals().weak_text_color();
        let stripe = ui.visuals().faint_bg_color;
        let border = ui.visuals().widgets.noninteractive.bg_stroke;
        let mut parent = false;
        let mut go = false;
        let mut pin = false;
        let mut breadcrumb = None;
        let mut row_command = None;
        let mut open = false;
        let mut inline_start = false;
        let mut inline_submit = false;
        let mut inline_cancel = false;
        let drag_allowed = self.drag_allowed() && ui.is_enabled();
        ui.spacing_mut().item_spacing.y = 0.0;
        let pane = &mut self.panes[index];
        let editable = self.plan.is_none()
            && self.error.is_none()
            && !self.palette
            && self.command_dialog.is_none();
        if editable {
            accessible_text_value(ui.ctx(), Id::new(("path", index)), &mut pane.path_text);
            if accessible_text_value(ui.ctx(), Id::new(("filter", index)), &mut pane.filter) {
                self.active = index;
                pane.rebuild();
            }
        }
        selection_footer(ui, index, pane, active, accent, muted, border);
        let heading = egui::Frame::new()
            .fill(stripe)
            .inner_margin(egui::Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(if index == 0 {
                            "01 / LEFT PANE"
                        } else {
                            "02 / RIGHT PANE"
                        })
                        .monospace()
                        .size(11.0)
                        .color(if active {
                            accent
                        } else {
                            ui.visuals().weak_text_color()
                        }),
                    );
                    let layout = if ui.available_width() < 100.0 {
                        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true)
                    } else {
                        egui::Layout::right_to_left(egui::Align::Center)
                    };
                    ui.with_layout(layout, |ui| {
                        pin = ui.small_button("+ pin").clicked();
                        parent = ui
                            .small_button("..")
                            .on_hover_text("Parent folder / Backspace")
                            .clicked();
                    });
                })
            });
        if active {
            let rect = heading.response.rect;
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
                0.0,
                accent,
            );
        }
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                if !self.path_edits[index] {
                    let (path, edit) =
                        compact_breadcrumbs(ui, &breadcrumb_paths(&pane.path, self.scope.root()));
                    breadcrumb = path;
                    if edit {
                        self.path_edits[index] = true;
                        self.editor_focus = Some(Id::new(("path", index)));
                    }
                } else {
                    let response = trailing_field(
                        ui,
                        egui::TextEdit::singleline(&mut pane.path_text)
                            .id(Id::new(("path", index)))
                            .font(egui::TextStyle::Monospace)
                            .margin(Vec2::new(7.0, 6.0)),
                        |ui| {
                            if ui.small_button("Done").clicked() {
                                self.path_edits[index] = false;
                            }
                            go = ui.button("Go").clicked();
                        },
                    );
                    if response.has_focus() {
                        self.active = index;
                        self.rail_index = None;
                    }
                    if self.editor_focus == Some(response.id) {
                        response.request_focus();
                        self.editor_focus = None;
                    }
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.add_action(egui::accesskit::Action::SetValue)
                    });
                    go |= response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
                }
                let clear = !pane.filter.is_empty();
                let mut clear_clicked = false;
                let response = trailing_field(
                    ui,
                    egui::TextEdit::singleline(&mut pane.filter)
                        .id(Id::new(("filter", index)))
                        .hint_text("Filter this folder...")
                        .font(egui::TextStyle::Monospace)
                        .margin(Vec2::new(7.0, 6.0)),
                    |ui| {
                        clear_clicked = ui
                            .add_enabled(clear, egui::Button::new("x"))
                            .on_hover_text("Clear filter")
                            .clicked();
                    },
                );
                if response.changed() {
                    pane.rebuild();
                }
                ui.ctx().accesskit_node_builder(response.id, |node| {
                    node.set_label(if index == 0 {
                        "Filter left pane"
                    } else {
                        "Filter right pane"
                    });
                    node.add_action(egui::accesskit::Action::SetValue)
                });
                if response.has_focus() {
                    self.active = index;
                    self.rail_index = None;
                }
                if self.editor_focus == Some(response.id) {
                    response.request_focus();
                    self.editor_focus = None;
                }
                if clear_clicked {
                    pane.filter.clear();
                    pane.rebuild();
                    self.file_focus_requested = true;
                }
            });
        let (head, head_response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), egui::Sense::hover());
        head_response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                ui.is_enabled(),
                "Name / Size / Last modified",
            )
        });
        ui.painter().rect_filled(head, 0.0, stripe);
        ui.painter().hline(head.x_range(), head.top(), border);
        ui.painter().hline(head.x_range(), head.bottom(), border);
        let mut head_content = head;
        head_content.max.x -= SCROLL_GUTTER;
        let columns = RowColumns::new(head_content);
        for (rect, text, sort, align) in [
            (columns.name, "Name", Sort::Name, egui::Align2::LEFT_CENTER),
            (columns.size, "Size", Sort::Size, egui::Align2::RIGHT_CENTER),
            (
                columns.age,
                "Last modified",
                Sort::Modified,
                egui::Align2::RIGHT_CENTER,
            ),
        ] {
            if rect.width() == 0.0 {
                continue;
            }
            let label = if pane.sort == sort {
                format!(
                    "{text} {}",
                    if pane.descending {
                        "\u{25bc}"
                    } else {
                        "\u{25b2}"
                    }
                )
            } else {
                text.to_owned()
            };
            let response =
                ui.interact(rect, Id::new(("column", index, text)), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
            });
            if response.clicked() {
                if pane.sort == sort {
                    pane.descending = !pane.descending;
                } else {
                    pane.sort = sort;
                    pane.descending = false;
                }
                self.active = index;
                files::sort_entries_ordered(&mut pane.entries, pane.sort, pane.descending);
                pane.rebuild();
                pane.scroll_to_focus = true;
            }
            ui.painter().with_clip_rect(rect).text(
                if align == egui::Align2::LEFT_CENTER {
                    rect.left_center()
                } else {
                    rect.right_center()
                },
                align,
                label,
                egui::FontId::monospace(10.0),
                muted,
            );
        }
        if let Some(warning) = &pane.warning {
            ui.label(RichText::new("PARTIAL / ERROR").color(accent));
            ui.label(warning);
        }
        let height = ui.available_height().max(0.0);
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("rows", index))
            .auto_shrink([false, false])
            .animated(false)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .max_height(height);
        if pane.scroll_to_focus {
            scroll = scroll.vertical_scroll_offset(reveal_row(
                pane.scroll_offset,
                pane.focus_position(),
                height,
            ));
            pane.scroll_to_focus = false;
        }
        let output = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                scroll.show_rows(ui, ROW_HEIGHT, pane.visible.len() + 1, |ui, range| {
                    for position in range {
                        let navigation = position == 0;
                        let parent_entry = Entry {
                            path: pane.path.clone(),
                            name: "..".into(),
                            directory: true,
                            link: false,
                            size: 0,
                            modified: None,
                            folded: String::new(),
                            aggregate: None,
                        };
                        let entry = if navigation {
                            parent_entry
                        } else {
                            pane.entries[pane.visible[position - 1]].clone()
                        };
                        let path = entry.path.clone();
                        let focused = if navigation {
                            pane.focused.is_none()
                        } else {
                            pane.focused.as_ref() == Some(&path)
                        };
                        let selected = !navigation && pane.selected.contains(&path);
                        let name = entry.name.clone();
                        let icon = if entry.link {
                            "[link]"
                        } else if entry.directory {
                            "[/]"
                        } else {
                            "[.]"
                        };
                        let available_parent = pane.path.parent().is_some()
                            && self.scope.root() != Some(pane.path.as_path());
                        let label = if navigation {
                            if available_parent {
                                "[/] .. / Parent folder"
                            } else {
                                "[/] .. / Parent unavailable"
                            }
                            .to_string()
                        } else {
                            format!("{icon} {name}")
                        };
                        let aggregate = entry.aggregate.as_ref().and_then(|result| result.as_ref().ok());
                        let partial = aggregate.is_some_and(files::DirectoryStats::partial);
                        let size = if navigation {
                            if available_parent { "<UP>" } else { "<ROOT>" }.into()
                        } else if entry.directory {
                            match &entry.aggregate {
                                None => "...".into(),
                                Some(Err(_)) => "!".into(),
                                Some(Ok(stats)) => format!("{}{}", format_size(stats.size), if partial { "+" } else { "" }),
                            }
                        } else {
                            format_size(entry.size)
                        };
                        let age = if navigation {
                            String::new()
                        } else if entry.directory {
                            match &entry.aggregate {
                                None => "...".into(),
                                Some(Err(_)) => "!".into(),
                                Some(Ok(stats)) => format!("{}{}", format_modified(stats.modified), if partial { "*" } else { "" }),
                            }
                        } else {
                            format_modified(entry.modified)
                        };
                        ui.push_id(&path, |ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), ROW_HEIGHT),
                                egui::Sense::hover(),
                            );
                            let row = egui::Rect::from_min_max(
                                egui::pos2(rect.left() + 30.0, rect.top()),
                                rect.max,
                            );
                            let mut response = ui
                                .interact(row, ui.id().with("file"), if navigation || !drag_allowed { egui::Sense::click() } else { egui::Sense::click_and_drag() })
                                .on_hover_text(format!("{}{}", files::display_path(&path),
                                    match &entry.aggregate {
                                        Some(Err(error)) => format!("\n{error}"),
                                        Some(Ok(stats)) => format!("\n{} items; {} links excluded{}{}; modified times are UTC",
                                            stats.examined, stats.skipped_links,
                                            if stats.truncated { "; calculation limit reached" } else { "" },
                                            if stats.warnings.is_empty() { String::new() } else { format!("\n{}", stats.warnings.join("\n")) }),
                                        None if entry.directory && !navigation => "\nCalculating folder size and latest file modification...".into(),
                                        _ => "\nModified times are UTC".into(),
                                    }));
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::Button,
                                    ui.is_enabled(),
                                    selected,
                                    &label,
                                )
                            });
                            if !navigation && drag_allowed && response.drag_started_by(egui::PointerButton::Primary) {
                                self.inline_pending = None;
                                egui::DragAndDrop::set_payload(ui.ctx(), DragItems::Files(pane.drag_targets(&path)));
                                pane.focused = Some(path.clone());
                                self.active = index;
                                self.rail_index = None;
                            }
                            if !navigation && !entry.link && entry.directory {
                                register_drop_zone(ui.ctx(), response.rect.intersect(ui.clip_rect()), DropLocation::Folder(path.clone()), 2);
                            } else if !navigation && !entry.link && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("zip")) {
                                register_drop_zone(ui.ctx(), response.rect.intersect(ui.clip_rect()), DropLocation::Zip { archive: path.clone(), prefix: String::new() }, 2);
                            }
                            if focused && active && editable && self.file_focus_requested {
                                response.request_focus();
                                self.file_focus_requested = false;
                            }
                            ui.ctx().accesskit_node_builder(response.id, |node| {
                                node.set_description(format!(
                                    "{size}, last modified {age}{}",
                                    if focused && active {
                                        ", current file"
                                    } else {
                                        ""
                                    }
                                ));
                            });
                            let fill = if selected {
                                tint(surface, accent, if focused && active { 40 } else { 28 })
                            } else if focused || response.hovered() || position % 2 == 1 {
                                stripe
                            } else {
                                surface
                            };
                            ui.painter().rect_filled(rect, 0.0, fill);
                            if focused {
                                ui.painter().rect_stroke(
                                    rect.shrink(0.5),
                                    0.0,
                                    if active {
                                        Stroke::new(1.0, accent)
                                    } else {
                                        border
                                    },
                                    egui::StrokeKind::Inside,
                                );
                            }
                            let mut checked = selected;
                            if !navigation {
                                let checkbox = ui
                                    .place(
                                        egui::Rect::from_center_size(
                                            egui::pos2(rect.left() + 16.0, rect.center().y),
                                            Vec2::splat(28.0),
                                        ),
                                        egui::Checkbox::without_text(&mut checked),
                                    )
                                    .on_hover_text(format!("Select {name}"));
                                checkbox.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::Checkbox,
                                        ui.is_enabled(),
                                        checked,
                                        format!("Select {name}"),
                                    )
                                });
                                if checkbox.changed() {
                                    if checked {
                                        pane.selected.insert(path.clone());
                                    } else {
                                        pane.selected.remove(&path);
                                    }
                                    self.active = index;
                                    self.rail_index = None;
                                    pane.focused = Some(path.clone());
                                    pane.anchor = position;
                                }
                            }
                            let columns = RowColumns::new(rect);
                            ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).text(
                                egui::pos2(rect.left() + 32.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                if entry.link { "@" } else { icon },
                                egui::FontId::monospace(10.0),
                                if entry.directory { accent } else { muted },
                            );
                            let editing = self.name_dialog.as_mut().filter(|dialog|
                                dialog.inline && dialog.pane == index && dialog.source.as_ref() == Some(&path));
                            if let Some(dialog) = editing {
                                let id = ui.id().with("inline-name");
                                accessible_text_value(ui.ctx(), id, &mut dialog.name);
                                let edit = ui.place(columns.name, egui::TextEdit::singleline(&mut dialog.name)
                                    .id(id).font(egui::TextStyle::Monospace));
                                if dialog.focus { edit.request_focus(); dialog.focus = false; }
                                inline_submit |= inline_name_submitted(ui, &edit);
                                inline_cancel |= ui.input_mut(|input|
                                    input.consume_key(Modifiers::NONE, Key::Escape));
                                inline_cancel |= edit.lost_focus() && ui.input(|input| input.pointer.any_pressed());
                                if let Some(error) = &dialog.error { edit.on_hover_text(error); }
                            } else {
                                let label = file_name_label(ui, columns.name, &name);
                                if !navigation && drag_allowed && label.drag_started_by(egui::PointerButton::Primary) {
                                    self.inline_pending = None;
                                    egui::DragAndDrop::set_payload(ui.ctx(), DragItems::Files(pane.drag_targets(&path)));
                                    pane.focused = Some(path.clone());
                                    self.active = index;
                                    self.rail_index = None;
                                }
                                if !navigation && label.clicked() && ui.input(|input| input.modifiers.is_none()) {
                                    label.request_focus();
                                    pane.focused = Some(path.clone());
                                    self.active = index;
                                    self.inline_pending = Some((index, path.clone(), ui.input(|input| input.time)));
                                    ui.ctx().request_repaint_after(Duration::from_millis(400));
                                }
                                response = response.union(label);
                            }
                            if !navigation {
                                response.context_menu(|ui| {
                                    for (command, label) in [(Command::OpenWith, "Open with / Shift Enter"), (Command::Preview, "Internal preview"), (Command::Rename, "Rename"), (Command::Copy, "Copy"), (Command::Move, "Move"), (Command::Recycle, "Recycle")] {
                                        if ui.button(label).clicked() {
                                            if !pane.selected.contains(&path) {
                                                pane.selected.clear();
                                            }
                                            pane.focused = Some(path.clone()); self.active = index;
                                            row_command = Some(command); ui.close();
                                        }
                                    }
                                });
                            }
                            for (rect, text) in [(columns.size, &size), (columns.age, &age)] {
                                if rect.width() == 0.0 { continue; }
                                ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).text(
                                    rect.right_center(),
                                    egui::Align2::RIGHT_CENTER,
                                    text,
                                    egui::FontId::monospace(11.0),
                                    muted,
                                );
                            }
                            if response.clicked() || response.secondary_clicked() {
                                response.request_focus();
                                self.active = index;
                                self.rail_index = None;
                                pane.focused = if navigation { None } else { Some(path.clone()) };
                                let modifiers = ui.input(|input| input.modifiers);
                                if navigation {
                                    pane.anchor = 0;
                                } else if modifiers.ctrl && response.clicked() {
                                    if !pane.selected.remove(&path) {
                                        pane.selected.insert(path.clone());
                                    }
                                } else if modifiers.shift {
                                    pane.selected = (pane.anchor.min(position)
                                        ..=pane.anchor.max(position))
                                        .filter_map(|position| pane.row_path(position))
                                        .collect();
                                } else {
                                    pane.anchor = position;
                                }
                            }
                            if response.double_clicked() {
                                inline_start = false;
                                self.inline_pending = None;
                                self.active = index;
                                pane.focused = if navigation { None } else { Some(path.clone()) };
                                if let Some(command) = double_click_command(
                                    navigation, ui.input(|input| input.modifiers),
                                ) {
                                    row_command = Some(command);
                                } else {
                                    open = true;
                                }
                            }
                        });
                    }
                })
            })
            .inner;
        pane.scroll_offset = output.state.offset.y;
        if !pane.busy {
            register_drop_zone(
                ui.ctx(),
                ui.max_rect(),
                DropLocation::Folder(pane.path.clone()),
                0,
            );
        }
        if let Some((pending_pane, path, started)) = &self.inline_pending
            && *pending_pane == index
            && ui.input(|input| input.time) - started >= 0.4
        {
            inline_start = self.panes[index].focused.as_ref() == Some(path);
            self.inline_pending = None;
        }
        if self.inline_pending.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
        if inline_cancel {
            self.name_dialog = None;
        }
        if inline_submit {
            self.prepare_name_change();
        }
        if inline_start {
            self.dispatch(Command::Rename);
            if let Some(dialog) = &mut self.name_dialog {
                dialog.inline = true;
            }
            ui.ctx().request_repaint();
        }
        if go {
            self.path_edits[index] = false;
            self.active = index;
            self.navigate(index, PathBuf::from(self.panes[index].path_text.trim()));
        }
        if let Some(path) = breadcrumb {
            self.active = index;
            self.navigate(index, path);
        }
        if let Some(command) = row_command {
            self.dispatch(command);
        }
        if parent {
            self.active = index;
            self.parent(index);
        }
        if pin {
            self.active = index;
            self.dispatch(Command::Pin);
        }
        if open {
            self.open_focused();
        }
    }

    fn prepare_name_change(&mut self) {
        let Some(dialog) = &mut self.name_dialog else {
            return;
        };
        if let Err(error) = tomas_commander::naming::validate_name(&dialog.name) {
            self.name_dialog = None;
            self.fail(error);
            return;
        }
        let dialog = self.name_dialog.take().expect("name dialog present");
        self.cancellation.store(false, Ordering::Release);
        match self.work.try_send(Work::PrepareName {
            id: self.next_id,
            folder: dialog.folder,
            source: dialog.source,
            name: dialog.name,
        }) {
            Ok(()) => {
                self.next_id += 1;
                self.operation_busy = true;
                self.log("Preparing exact name change.");
            }
            Err(error) => self.fail(format!("Name change was not queued: {error}")),
        }
    }

    fn resolve_command_intent(&mut self, result: FileResult<commands::Intent>) {
        let Some(dialog) = &mut self.command_dialog else {
            return;
        };
        match result.and_then(|intent| intent.validate().map(|()| intent)) {
            Err(error) => dialog.error = Some(error),
            Ok(intent) if intent.status != search::IntentStatus::Ready => {
                dialog.error = Some(
                    if intent.status == search::IntentStatus::Ambiguous {
                        "Clarify source concepts, action and destination folder. No scan or effects."
                    } else {
                        "Unsupported command. No scan or effects."
                    }
                    .into(),
                );
            }
            Ok(intent) => {
                self.cancellation.store(false, Ordering::Release);
                match self.work.try_send(Work::ResolveCommand {
                    roots: dialog.roots.clone(),
                    intent,
                }) {
                    Ok(()) => {
                        dialog.resolving = true;
                        self.operation_busy = true;
                    }
                    Err(error) => {
                        dialog.error = Some(format!("Command discovery not queued: {error}"))
                    }
                }
            }
        }
    }

    fn command_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.command_dialog else {
            return;
        };
        let mut start = false;
        let mut close = false;
        let mut configure = false;
        let mut prepare = false;
        let mut reveal = false;
        #[cfg(feature = "native-desktop-tests")]
        let mut injected = None;
        #[cfg(feature = "native-desktop-tests")]
        let mut rejection_probe = false;
        #[cfg(feature = "native-desktop-tests")]
        let rejection_root = dialog.roots[0].clone();
        let busy = dialog.interpreting || dialog.resolving;
        let modal = egui::Modal::new(Id::new("natural-command")).show(ctx, |ui| {
            ui.set_width(680.0);
            ui.heading("Natural-language file command / CZ EN");
            #[cfg(feature = "native-desktop-tests")]
            if self.desktop_metrics.is_some() {
                ui.label("TEST ONLY / injected guard proposals / no model requests or execution approval");
                ui.add_enabled_ui(!busy, |ui| {
                    rejection_probe = ui.button("Probe unapproved / stale snapshots / replay").clicked();
                    for (label, value) in [
                        ("Inject unsupported command", r#"{"status":"unsupported","action":null,"source":null,"destination":[]}"#),
                        ("Inject malformed command", r#"{"status":"ready","action":"shell"}"#),
                        ("Inject missing destination", r#"{"status":"ready","action":"copy","source":{"status":"ready","terms":[["alpha"]],"folders":[],"file_types":[]},"destination":["folder"]}"#),
                        ("Inject copy proposal", r#"{"status":"ready","action":"copy","source":{"status":"ready","terms":[["alpha"]],"folders":[],"file_types":[]},"destination":["Příliš žluťoučký folder"]}"#),
                        ("Inject conflicting copy", r#"{"status":"ready","action":"copy","source":{"status":"ready","terms":[["workspace-proof"]],"folders":[],"file_types":[]},"destination":["Příliš žluťoučký folder"]}"#),
                    ] {
                        if ui.button(label).clicked() {
                            injected = Some(commands::Intent::parse(value));
                        }
                    }
                });
            }
            ui.label(format!("Source root: {}", files::display_path(&dialog.roots[0])));
            ui.label(format!("Destination root: {}", files::display_path(&dialog.roots[1])));
            ui.label("One file: find, copy, move or recycle. Names only; no overwrite, new folders, shell or automatic retry.");
            let prompt_id = Id::new("command-prompt");
            let mut changed = !busy && accessible_text_value(ui.ctx(), prompt_id, &mut dialog.prompt);
            let response = ui.add_enabled(!busy, egui::TextEdit::singleline(&mut dialog.prompt)
                .id(prompt_id).char_limit(2048).desired_width(f32::INFINITY)
                .hint_text("Find the Penta hackathon presentation and copy it into Kosik"));
            changed |= response.changed();
            ui.ctx().accesskit_node_builder(response.id, |node| {
                node.set_label("Natural-language command prompt");
                node.add_action(egui::accesskit::Action::SetValue);
            });
            if dialog.focus_prompt {
                response.request_focus();
                dialog.focus_prompt = false;
            }
            if changed {
                dialog.consent = false;
                dialog.resolution = None;
                dialog.source = None;
                dialog.destination = None;
                dialog.details = None;
                dialog.error = None;
            }
            ui.label("Luna receives only this explicit prompt + fixed schema. Pane roots, filenames, candidates and contents stay local.");
            ui.add_enabled(!busy, egui::Checkbox::new(&mut dialog.consent,
                "Send only this harmless prompt; resolve names within these two roots locally."));
            ui.horizontal(|ui| {
                start = ui.add_enabled(!busy && dialog.consent,
                    egui::Button::new("Interpret command once")).clicked();
                configure = ui.add_enabled(!busy,
                    egui::Button::new("Foundry connection / Check")).clicked();
            });
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(if dialog.interpreting { "Interpreting prompt / Foundry..." } else { "Resolving actual names locally..." });
                });
            }
            if let Some(details) = &dialog.details {
                egui::CollapsingHeader::new("Command interpretation / untrusted proposal").show(ui, |ui| {
                    ui.add(egui::Label::new(details).wrap().selectable(true));
                });
            }
            if let Some(error) = &dialog.error {
                ui.label(RichText::new(error).color(Color32::LIGHT_RED));
            }
            if let Some(resolution) = &dialog.resolution {
                ui.separator();
                ui.label(format!("Action: {:?} / {} source candidates", resolution.action(), resolution.sources().len()));
                if resolution.sources().is_empty() {
                    ui.label("No source match. Clarify the prompt; no plan or effects.");
                } else if resolution.sources().len() > 1 {
                    ui.label("Ambiguous source: explicitly choose one actual file. Ranking never chooses for you.");
                }
                egui::ScrollArea::vertical().id_salt("command-sources").max_height(140.0).show(ui, |ui| {
                    for (index, candidate) in resolution.sources().iter().enumerate() {
                        if ui.selectable_label(dialog.source == Some(index),
                            files::display_path(&candidate.entry.path)).clicked() {
                            dialog.source = Some(index);
                        }
                    }
                });
                let mutation = resolution.action() != commands::Action::Find;
                let needs_destination = matches!(resolution.action(), commands::Action::Copy | commands::Action::Move);
                if needs_destination {
                    if resolution.destinations().is_empty() {
                        ui.label("No existing destination folder matches. Clarify the prompt; folders are never created by interpretation.");
                    } else if resolution.destinations().len() > 1 {
                        ui.label("Ambiguous destination: explicitly choose one actual folder.");
                    }
                    egui::ScrollArea::vertical().id_salt("command-destinations").max_height(100.0).show(ui, |ui| {
                        for (index, candidate) in resolution.destinations().iter().enumerate() {
                            if ui.selectable_label(dialog.destination == Some(index),
                                files::display_path(candidate.path())).clicked() {
                                dialog.destination = Some(index);
                            }
                        }
                    });
                }
                ui.label("Choosing candidates is not approval. The immutable exact-target plan requires separate confirmation.");
                if mutation {
                    prepare = ui.add_enabled(dialog.source.is_some() && (!needs_destination || dialog.destination.is_some()),
                        egui::Button::new("Prepare exact plan / no effects")).clicked();
                } else {
                    reveal = ui.add_enabled(dialog.source.is_some(), egui::Button::new("Reveal chosen file")).clicked();
                }
            }
            close = ui.button(if busy { "Cancel command / Escape" } else { "Close / Escape" }).clicked();
        });
        close |= modal.should_close();
        if start && !close {
            dialog.consent = false;
            dialog.resolution = None;
            dialog.source = None;
            dialog.destination = None;
            dialog.details = None;
            dialog.error = None;
            dialog.interpreting = self.ai.start_command(ctx, &dialog.prompt);
            if !dialog.interpreting {
                dialog.error = Some(self.ai.status().to_owned());
            }
        }
        if (prepare || reveal)
            && !close
            && let (Some(resolution), Some(source)) = (&dialog.resolution, dialog.source)
        {
            self.cancellation.store(false, Ordering::Release);
            let job = if prepare {
                Work::PrepareCommand {
                    id: self.next_id,
                    resolution: resolution.clone(),
                    source,
                    destination: dialog.destination,
                }
            } else {
                Work::LocateCommand {
                    pane: dialog.pane,
                    resolution: resolution.clone(),
                    source,
                }
            };
            match self.work.try_send(job) {
                Ok(()) => {
                    if prepare {
                        self.next_id += 1;
                        self.operation_busy = true;
                    }
                    close = true;
                }
                Err(error) => dialog.error = Some(format!("Command choice not queued: {error}")),
            }
        } else if close && busy {
            self.ai.cancel();
            self.cancellation.store(true, Ordering::Release);
        }
        if configure {
            close = true;
            self.ai.open = true;
        }
        if close {
            if !configure {
                self.ai.disconnect();
            }
            self.command_dialog = None;
            self.file_focus_requested = true;
        }
        #[cfg(feature = "native-desktop-tests")]
        if rejection_probe && !close {
            let dialog = self.command_dialog.as_mut().expect("Open command dialog");
            match self.work.try_send(Work::DesktopRejections(rejection_root)) {
                Ok(()) => {
                    dialog.resolving = true;
                    self.operation_busy = true;
                }
                Err(error) => dialog.error = Some(format!("Rejection probe not queued: {error}")),
            }
        }
        #[cfg(feature = "native-desktop-tests")]
        if let Some(result) = injected
            && !close
        {
            if let Some(dialog) = &mut self.command_dialog {
                dialog.resolution = None;
                dialog.source = None;
                dialog.destination = None;
                dialog.error = None;
                dialog.details = Some("TEST ONLY / injected proposal, not live inference.".into());
            }
            self.resolve_command_intent(result);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some((path, name)) = &mut self.place_edit {
            let mut save = false;
            let mut cancel = false;
            let modal = egui::Modal::new(Id::new("saved-place-name")).show(ctx, |ui| {
                ui.heading("Saved place name");
                ui.label(files::display_path(path));
                let id = Id::new("saved-place-name-input");
                accessible_text_value(ctx, id, name);
                ui.add(egui::TextEdit::singleline(name).id(id));
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                save = ui.button("Save").clicked()
                    || ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
                cancel = ui.button("Cancel / Escape").clicked();
            });
            if cancel || modal.should_close() {
                self.place_edit = None;
                self.error = None;
            } else if save {
                let (path, name) = self.place_edit.clone().expect("place edit present");
                match self.preferences.name_favorite(path, name) {
                    Ok(()) => {
                        self.save_pending = true;
                        self.place_edit = None;
                        self.error = None;
                    }
                    Err(error) => self.fail(error),
                }
            }
            return;
        }
        if self.settings_open {
            let mut connection = false;
            let mut close = false;
            let modal = egui::Modal::new(Id::new("settings")).show(ctx, |ui| {
                ui.set_width(420.0);
                ui.heading("Settings");
                ui.label("Appearance");
                if ui.button(if self.preferences.dark { "Switch to light mode" } else { "Switch to dark mode" }).clicked() { self.dispatch(Command::Theme); }
                if ui.button(format!("Accent: {}", ACCENTS[self.preferences.accent].0)).clicked() { self.dispatch(Command::Accent); }
                ui.horizontal(|ui| {
                    if ui.button("-").clicked() { self.dispatch(Command::ZoomOut); }
                    ui.label(format!("UI zoom: {:.0}%", self.preferences.zoom * 100.0));
                    if ui.button("+").clicked() { self.dispatch(Command::ZoomIn); }
                    if ui.button("Reset").clicked() { self.dispatch(Command::ZoomReset); }
                });
                ui.separator();
                ui.label("Connections");
                connection = ui.button("Foundry connection / advanced pilot").clicked();
                ui.label("Connection configuration and explicit pilot controls stay separate from everyday file work.");
                close = ui.button("Close / Escape").clicked();
            });
            if connection {
                self.settings_open = false;
                self.ai.open = true;
            } else if close || modal.should_close() {
                self.settings_open = false;
            }
            return;
        }
        if self.operations_open {
            let mut close = false;
            let modal = egui::Modal::new(Id::new("operations")).show(ctx, |ui| {
                ui.set_width(620.0);
                ui.heading("Operations");
                ui.label("Exact results, errors and partial paths. No automatic undo.");
                if let Some(outcome) = &self.last_outcome {
                    ui.label(format!("Operation #{}", outcome.id));
                    if let Some(error) = &outcome.error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                    if outcome.cancelled {
                        ui.label(
                            "Cancelled / inspect any created or incomplete paths before retrying.",
                        );
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("operation-paths")
                        .max_height(170.0)
                        .show(ui, |ui| {
                            for (label, paths) in [
                                ("CREATED", &outcome.created),
                                ("COMPLETED", &outcome.completed),
                                ("NOT COMPLETED", &outcome.incomplete),
                            ] {
                                for path in paths {
                                    ui.label(format!("{label}: {}", files::display_path(path)));
                                }
                            }
                        });
                } else {
                    ui.label("No executed operation in this session.");
                }
                egui::CollapsingHeader::new("Diagnostics / recent messages").show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("operation-diagnostics")
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for message in &self.activity {
                                ui.label(message);
                            }
                        });
                });
                close = ui.button("Close / Escape").clicked();
            });
            if close || modal.should_close() {
                self.operations_open = false;
            }
            return;
        }
        if let Some(dialog) = &mut self.open_with {
            let mut chosen = None;
            let mut close = false;
            let folder = dialog.folder.clone();
            let modal = egui::Modal::new(Id::new("open-with")).show(ctx, |ui| {
                ui.set_width(480.0);
                ui.heading("Open with / Shift Enter");
                ui.label(format!("Workspace: {}", files::display_path(&folder)));
                if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                    dialog.index = dialog.index.saturating_sub(1);
                }
                if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                    dialog.index = (dialog.index + 1).min(2);
                }
                let enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                for (index, label) in ["VS Code", "GitHub Copilot CLI", "GitHub Copilot App"]
                    .iter()
                    .enumerate()
                {
                    if ui
                        .add(egui::Button::selectable(dialog.index == index, *label))
                        .clicked()
                        || (enter && dialog.index == index)
                    {
                        chosen = Some(index);
                    }
                }
                close = ui.button("Cancel / Escape").clicked();
            });
            if close || modal.should_close() {
                self.open_with = None;
            } else if let Some(index) = chosen {
                let job = match index {
                    0 => Work::Code(folder),
                    1 => Work::Copilot {
                        path: folder,
                        app: false,
                    },
                    _ => Work::Copilot {
                        path: folder,
                        app: true,
                    },
                };
                self.open_with = None;
                if let Err(error) = self.work.try_send(job) {
                    self.fail(format!("Open with was not queued: {error}"));
                }
            }
            return;
        }
        if self
            .name_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.inline)
        {
            return;
        }
        if let Some(dialog) = &mut self.name_dialog {
            let mut prepare = false;
            let mut close = false;
            let modal = egui::Modal::new(Id::new("name-dialog")).show(ctx, |ui| {
                ui.set_width(500.0);
                ui.heading(if dialog.source.is_some() {
                    "Rename"
                } else {
                    "New folder"
                });
                ui.label(format!("Folder: {}", files::display_path(&dialog.folder)));
                if let Some(source) = &dialog.source {
                    ui.label(format!("Source: {}", files::display_path(source)));
                }
                let id = Id::new("name-input");
                accessible_text_value(ui.ctx(), id, &mut dialog.name);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut dialog.name)
                        .id(id)
                        .hint_text("One name, not a path")
                        .desired_width(f32::INFINITY),
                );
                ui.ctx().accesskit_node_builder(id, |node| {
                    node.set_label("New name");
                    node.add_action(egui::accesskit::Action::SetValue);
                });
                if dialog.focus {
                    response.request_focus();
                    dialog.focus = false;
                }
                if let Some(error) = &dialog.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                prepare = ui.button("Review exact change / Enter").clicked()
                    || ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                close = ui.button("Cancel / Escape").clicked();
            });
            if close || modal.should_close() {
                self.name_dialog = None;
            } else if prepare {
                if let Err(error) = tomas_commander::naming::validate_name(&dialog.name) {
                    dialog.error = Some(error);
                } else {
                    self.prepare_name_change();
                }
            }
            return;
        }
        if let Some(dialog) = &mut self.zip_dialog {
            let mut prepare = false;
            let mut close = false;
            let modal = egui::Modal::new(Id::new("create-zip-name")).show(ctx, |ui| {
                ui.set_width(600.0);
                ui.heading("Create ZIP / choose exact filename");
                ui.label(format!("{} selected roots / destination {}", dialog.sources.len(), files::display_path(&dialog.destination)));
                let id = Id::new("zip-output-name");
                accessible_text_value(ctx, id, &mut dialog.name);
                let response = ui.add(egui::TextEdit::singleline(&mut dialog.name).id(id));
                if dialog.focus { response.request_focus(); dialog.focus = false; }
                ui.label("This only prepares a plan. The archive is created after separate exact-target confirmation.");
                prepare = ui.button("Prepare ZIP plan / Enter").clicked();
                prepare |= ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
                close = ui.button("Cancel / Escape").clicked();
            });
            close |= modal.should_close();
            if close {
                self.zip_dialog = None;
            } else if prepare && let Some(dialog) = self.zip_dialog.take() {
                if dialog.name.is_empty() || dialog.name.contains(['/', '\\', ':']) {
                    self.fail("ZIP output must be one filename inside the displayed destination, not a path.");
                } else {
                    self.prepare_archive(tomas_commander::archive_changes::Request::Create {
                        sources: dialog.sources,
                        target: dialog.destination.join(dialog.name),
                    });
                }
            }
        }
        if self.help_open {
            let mut close = false;
            let modal = egui::Modal::new(Id::new("keyboard-help")).show(ctx, |ui| {
                ui.set_width(600.0);
                ui.heading("Keyboard shortcuts");
                ui.label("File shortcuts apply outside text editors. Escape closes dialogs.");
                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .show(ui, |ui| {
                        egui::Grid::new("help-shortcuts")
                            .striped(true)
                            .show(ui, |ui| {
                                for (key, label) in [
                                    ("Ctrl Shift P", "Command palette"),
                                    ("Tab", "Switch file pane"),
                                    ("Up / Down / Home / End", "Navigate rows"),
                                    (
                                        "Space / Shift arrows / Ctrl A",
                                        "Toggle / range / select all",
                                    ),
                                    ("Enter", "Open file or folder"),
                                    ("Ctrl Shift Left / Right", "Resize file panes"),
                                ] {
                                    ui.label(RichText::new(key).monospace());
                                    ui.label(label);
                                    ui.end_row();
                                }
                                for (_, label, key) in
                                    COMMANDS.iter().filter(|(_, _, key)| !key.is_empty())
                                {
                                    ui.label(RichText::new(*key).monospace());
                                    ui.label(*label);
                                    ui.end_row();
                                }
                                ui.label(RichText::new("Ctrl Enter / Escape").monospace());
                                ui.label("Approve exact plan / Cancel");
                                ui.end_row();
                            });
                    });
                close = ui.button("Close / Escape").clicked();
            });
            if close || modal.should_close() {
                self.help_open = false;
                self.file_focus_requested = true;
            }
            return;
        }
        if let Some(mut index) = self.places_popup {
            let mut selected = None;
            let mut close = false;
            let mut pin = false;
            let mut remove = None;
            let mut edit = None;
            let can_pin =
                self.reader.target != Some(self.active) && self.archive.target != Some(self.active);
            let pinned = self
                .preferences
                .favorites
                .contains(&self.panes[self.active].path);
            let modal = egui::Modal::new(Id::new("saved-places")).show(ctx, |ui| {
                ui.set_width(460.0);
                ui.heading("Saved places");
                if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                    index = index.saturating_sub(1);
                }
                if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                    index = index.saturating_add(1);
                }
                index = index.min(self.preferences.favorites.len());
                if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter)) {
                    if index == self.preferences.favorites.len() {
                        pin = can_pin && !pinned;
                    } else {
                        selected = self.preferences.favorites.get(index).cloned();
                    }
                }
                if self.preferences.favorites.is_empty() {
                    ui.label("No saved places yet.");
                }
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for (row, path) in self.preferences.favorites.iter().enumerate() {
                            let (response, rename, delete) = saved_place_row(
                                ui,
                                &self.preferences.favorite_name(path),
                                path,
                                row == index,
                            );
                            if rename {
                                edit = Some((path.clone(), self.preferences.favorite_name(path)));
                            }
                            if delete {
                                remove = Some(path.clone());
                            }
                            if response.clicked() {
                                selected = Some(path.clone());
                            }
                            if row == index {
                                response.scroll_to_me(None);
                            }
                        }
                    });
                ui.separator();
                let response = ui.add_enabled(
                    can_pin && !pinned,
                    egui::Button::new(if pinned {
                        "Current folder already saved"
                    } else {
                        "+ Add current folder"
                    })
                    .selected(index == self.preferences.favorites.len()),
                );
                pin |= response.clicked();
                response.on_hover_text(files::display_path(&self.panes[self.active].path));
                close = ui.button("Cancel / Escape").clicked();
            });
            self.places_popup = Some(index);
            if let Some(path) = remove {
                self.preferences.remove_favorite(&path);
                self.save_pending = true;
                self.places_popup = Some(index.min(self.preferences.favorites.len()));
            }
            if edit.is_some() {
                self.name_dialog = None;
                self.inline_pending = None;
                self.place_edit = edit;
            }
            if selected.is_some() || close || modal.should_close() {
                self.places_popup = None;
                self.file_focus_requested = true;
            }
            if let Some(path) = selected {
                match self.scope.resolve(&path) {
                    Ok(path) if path.is_dir() => self.navigate(self.active, path),
                    Ok(_) => self.fail("Saved place is no longer a directory."),
                    Err(error) => self.fail(error),
                }
            }
            if pin {
                self.places_popup = None;
                self.dispatch(Command::Pin);
                self.file_focus_requested = true;
            }
            return;
        }
        self.command_dialog_ui(ctx);
        if let Some(dialog) = &mut self.search_dialog {
            let mut start = false;
            let mut cancel = false;
            let mut close = false;
            let mut activate = None;
            let mut configure = false;
            let modal = egui::Modal::new(Id::new("filename-search")).show(ctx, |ui| {
                ui.set_width(680.0);
                ui.heading(if dialog.ai_assisted { "AI-assisted file search" } else { "Search files / local" });
                ui.label(format!("In: {}", files::display_path(&dialog.root)))
                    .on_hover_text(format!("Includes subfolders; matches filenames without case sensitivity. Links are skipped.\nLimits: {} examined items, {} results.", search::MAX_EXAMINED, search::MAX_RESULTS));
                let query_id = Id::new("search-query");
                let query_focused_before = ui.memory(|memory| memory.has_focus(query_id));
                let enter = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
                let down = dialog.results.is_some()
                    && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown));
                let up = dialog.results.is_some()
                    && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp));
                let mut changed = false;
                if !dialog.busy {
                    changed = accessible_text_value(ui.ctx(), query_id, &mut dialog.query);
                }
                #[cfg(feature = "native-search-tests")]
                if self.ai.test_scenario.is_some() {
                    ui.label(RichText::new("TEST ONLY / injected failure states / ZERO provider calls").strong());
                    ui.add_enabled_ui(!dialog.busy, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for (scenario, label) in crate::ai_ui::SearchTestScenario::ALL {
                                if ui.selectable_label(self.ai.test_scenario == Some(scenario), label).clicked() {
                                    self.ai.test_scenario = Some(scenario);
                                    dialog.consent = false;
                                    dialog.results = None;
                                    dialog.error = None;
                                    dialog.interpretation = None;
                                }
                            }
                        });
                    });
                }
                ui.horizontal(|ui| {
                    let response = ui.add_enabled(!dialog.busy, egui::TextEdit::singleline(&mut dialog.query)
                        .id(query_id).char_limit(2048)
                        .hint_text(if dialog.ai_assisted { "Describe names, folders or file types (CZ / EN)..." } else { "Filename contains..." }).desired_width(480.0));
                    changed |= response.changed();
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.set_label(if dialog.ai_assisted { "AI search prompt" } else { "Recursive filename query" });
                        node.add_action(egui::accesskit::Action::SetValue);
                    });
                    if dialog.focus_query {
                        response.request_focus();
                        dialog.focus_query = false;
                    }
                    start = ui.add_enabled(!dialog.busy && (!dialog.ai_assisted || dialog.consent),
                        egui::Button::new(if dialog.ai_assisted { "Interpret once" } else { "Search" })).clicked();
                });
                if dialog.ai_assisted {
                    if changed {
                        dialog.consent = false;
                        dialog.results = None;
                        dialog.interpretation = None;
                        dialog.error = None;
                    }
                    ui.label("Only this prompt + fixed instructions go to Luna. Filenames, paths and contents stay local. No automatic retry.");
                    ui.add_enabled(!dialog.busy, egui::Checkbox::new(&mut dialog.consent,
                        "Search this folder locally; send only this harmless prompt. No private data."));
                    configure = ui.add_enabled(!dialog.busy, egui::Button::new("Foundry connection / Check")).clicked();
                    if let Some(interpretation) = &dialog.interpretation {
                        ui.label(RichText::new(interpretation.lines().next().unwrap_or("")).small())
                            .on_hover_text(interpretation);
                        egui::CollapsingHeader::new("Interpretation / details").show(ui, |ui| {
                            ui.add(egui::Label::new(interpretation).wrap().selectable(true));
                        });
                    }
                }
                if query_focused_before && enter {
                    start = !dialog.busy && (!dialog.ai_assisted || dialog.consent);
                }
                if dialog.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(if dialog.interpreting { "Interpreting prompt / Foundry..." } else { "Searching actual filenames locally..." });
                        cancel = ui.button("Cancel search").clicked();
                    });
                }
                if let Some(error) = &dialog.error { ui.label(RichText::new(error).color(Color32::LIGHT_RED)); }
                if let Some(results) = &dialog.results {
                    ui.label(format!("{} results{}", results.matches.len(),
                        if results.truncated { " / limit reached" } else { "" }))
                        .on_hover_text(format!("{} items examined; {} links skipped", results.examined, results.skipped_links));
                    if results.matches.is_empty() { ui.label(if dialog.ai_assisted { "No matching files in this root. Name matching only; try a more precise prompt." } else { "No results" }); }
                    if !results.errors.is_empty() {
                        ui.label("Partial results").on_hover_text(results.errors.join("\n"));
                    }
                    let query_focused = query_focused_before;
                    let activate_enter = !query_focused && enter;
                    if down || up {
                        ui.memory_mut(|memory| {
                            memory.surrender_focus(query_id);
                            memory.move_focus(egui::FocusDirection::None);
                        });
                        dialog.index = if query_focused { 0 } else if down { dialog.index.saturating_add(1) } else { dialog.index.saturating_sub(1) };
                    }
                    dialog.index = dialog.index.min(results.matches.len().saturating_sub(1));
                    let mut scroll = egui::ScrollArea::vertical().id_salt("search-matches")
                        .max_height(280.0).animated(false);
                    if down || up {
                        scroll = scroll.vertical_scroll_offset(reveal_row(dialog.scroll_offset, dialog.index, 280.0));
                    }
                    let output = scroll.show_rows(ui, ROW_HEIGHT, results.matches.len(), |ui, range| {
                        for index in range {
                            let result = &results.matches[index];
                            let label = format!("{} {}{}", if result.entry.directory { "[/]" } else { "[.]" }, result.relative.display(),
                                if dialog.ai_assisted { format!("  / {}", result.explanation) } else { String::new() });
                            let response = ui.add(egui::Button::new(label).selected(index == dialog.index)
                                .min_size(Vec2::new(ui.available_width(), ROW_HEIGHT)).truncate())
                                .on_hover_text(format!("{}\n{} / score {}", result.relative.display(), result.explanation, result.score));
                            if response.clicked() { dialog.index = index; }
                            if response.double_clicked() { activate = Some(result.clone()); }
                            if (down || up) && index == dialog.index { response.request_focus(); }
                        }
                    });
                    dialog.scroll_offset = output.state.offset.y;
                    if activate_enter {
                        activate = results.matches.get(dialog.index).cloned();
                    }
                    if ui.add_enabled(!results.matches.is_empty(), egui::Button::new("Show result in pane / Enter")).clicked() {
                        activate = results.matches.get(dialog.index).cloned();
                    }
                }
                close = ui.button("Close").on_hover_text("Escape").clicked();
            });
            close |= modal.should_close();
            if cancel || close {
                if dialog.interpreting {
                    self.ai.disconnect();
                    dialog.interpreting = false;
                }
                self.search_generation.fetch_add(1, Ordering::Relaxed);
                dialog.busy = false;
                dialog.results = None;
                dialog.error = Some("Search cancelled.".into());
            }
            if start {
                dialog.index = 0;
                dialog.scroll_offset = 0.0;
                dialog.results = None;
                dialog.error = None;
                dialog.interpretation = None;
                if dialog.ai_assisted {
                    dialog.consent = false;
                    dialog.interpreting = self.ai.start_search(ctx, &dialog.query);
                    dialog.busy = dialog.interpreting;
                    if !dialog.busy {
                        dialog.error = Some(self.ai.status().to_owned());
                    }
                } else {
                    let generation = self.search_generation.fetch_add(1, Ordering::Relaxed) + 1;
                    match self.search_jobs.try_send(SearchRequest {
                        generation,
                        root: dialog.root.clone(),
                        query: dialog.query.clone(),
                        intent: None,
                        #[cfg(feature = "native-search-tests")]
                        slow_test_walk: false,
                    }) {
                        Ok(()) => dialog.busy = true,
                        Err(error) => {
                            dialog.error = Some(format!("Search was not queued: {error}"))
                        }
                    }
                }
            }
            if configure {
                close = true;
                self.ai.open = true;
            }
            let pane = dialog.pane;
            if let Some(result) = activate {
                match self.work.try_send(Work::Locate { pane, result }) {
                    Ok(()) => close = true,
                    Err(error) => {
                        dialog.error = Some(format!("Result activation was not queued: {error}"))
                    }
                }
            }
            if close {
                if dialog.ai_assisted && !configure {
                    self.ai.disconnect();
                }
                self.search_dialog = None;
            }
        }
        if self.palette {
            let mut chosen = None;
            let mut dismiss = false;
            let modal = egui::Modal::new(Id::new("palette")).show(ctx, |ui| {
                ui.set_width(580.0);
                ui.heading("Command palette / Ctrl Shift P");
                let query_id = Id::new("palette-query");
                let keyboard = ui.memory(|memory| memory.has_focus(query_id));
                let enter = keyboard
                    && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
                if keyboard {
                    if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                        self.palette_index += 1;
                    }
                    if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                        self.palette_index = self.palette_index.saturating_sub(1);
                    }
                }
                if accessible_text_value(
                    ui.ctx(),
                    Id::new("palette-query"),
                    &mut self.palette_query,
                ) {
                    self.palette_index = 0;
                }
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.palette_query)
                        .id(query_id)
                        .hint_text("Find a command...")
                        .desired_width(f32::INFINITY),
                );
                if self.palette_focus {
                    response.request_focus();
                    self.palette_focus = false;
                }
                if response.changed() {
                    self.palette_index = 0;
                }
                ui.ctx().accesskit_node_builder(response.id, |node| {
                    node.add_action(egui::accesskit::Action::SetValue)
                });
                let query = self.palette_query.to_lowercase();
                let matches: Vec<_> = COMMANDS
                    .iter()
                    .filter(|(_, name, _)| name.to_lowercase().contains(&query))
                    .collect();
                self.palette_index = self.palette_index.min(matches.len().saturating_sub(1));
                egui::ScrollArea::vertical()
                    .id_salt("palette-commands")
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (index, (command, name, hint)) in matches.iter().enumerate() {
                            let response = ui.add(
                                egui::Button::new(*name)
                                    .shortcut_text(RichText::new(*hint).monospace().size(11.0))
                                    .min_size(Vec2::new(ui.available_width(), 32.0))
                                    .selected(index == self.palette_index),
                            );
                            ui.ctx().accesskit_node_builder(response.id, |node| {
                                node.set_label(format!("{name}    {hint}"));
                            });
                            if index == self.palette_index {
                                response.scroll_to_me(None);
                            }
                            if response.clicked() {
                                chosen = Some(*command);
                            }
                        }
                    });
                if enter && let Some((command, _, _)) = matches.get(self.palette_index) {
                    chosen = Some(*command);
                }
                ui.separator();
                ui.label("Deterministic commands work offline. Natural-language file commands use an explicit prompt-only mode.");
                dismiss = ui.button("Close / Escape").clicked();
            });
            dismiss |= modal.should_close();
            if dismiss {
                self.palette = false;
                self.file_focus_requested = true;
            }
            if let Some(command) = chosen {
                self.dispatch(command);
            }
        }
        if let Some(plan) = &self.change_plan {
            let mut approve = false;
            let mut cancel = false;
            let modal = egui::Modal::new(Id::new(("source-change-approval", plan.id()))).show(ctx, |ui| {
                ui.set_width(620.0);
                ui.heading(plan.label());
                ui.label(plan.summary());
                if let Change::Archive(archive) = plan {
                    egui::ScrollArea::both().id_salt("zip-exact-plan-rows").max_height(220.0)
                        .show_rows(ui, ROW_HEIGHT, archive.row_count(), |ui, range| {
                            for index in range { ui.add(egui::Label::new(archive.row(index)).wrap_mode(egui::TextWrapMode::Extend)); }
                        });
                }
                ui.label("Only this frozen plan is approved. External changes and duplicate replay are refused.");
                let response = ui.button("Cancel / Escape");
                if self.approval_focus { response.request_focus(); self.approval_focus = false; }
                cancel = response.clicked();
                approve = ui.button("Confirm exact change / Ctrl Enter").clicked();
                approve |= ui.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::Enter));
            });
            cancel |= modal.should_close();
            if cancel {
                self.change_plan = None;
                self.log("Source change cancelled; no source write.");
            }
            if approve && let Some(plan) = self.change_plan.take() {
                self.cancellation.store(false, Ordering::Relaxed);
                match self.work.try_send(Work::Change(plan)) {
                    Ok(()) => {
                        self.operation_busy = true;
                        self.log("Executing exactly approved source change...");
                    }
                    Err(error) => self.fail(format!("Source change was not queued: {error}")),
                }
            }
        }
        if let Some(plan) = &self.plan {
            let mut approve = false;
            let mut cancel = false;
            let modal = egui::Modal::new(Id::new(("approval", plan.id()))).show(ctx, |ui| {
                ui.set_width(620.0);
                ui.spacing_mut().item_spacing.y = 10.0;
                let count = plan.sources().len();
                ui.heading(format!(
                    "{} {count} {}",
                    plan.operation().name(),
                    if count == 1 { "item" } else { "items" }
                ));
                egui::Frame::group(ui.style())
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("Destination").strong());
                        let destination = plan
                            .destination()
                            .map(files::display_path)
                            .unwrap_or_else(|| {
                                "Windows Recycle Bin (never permanently delete)".into()
                            });
                        ui.add(
                            egui::Label::new(RichText::new(destination).monospace().size(12.0))
                                .wrap(),
                        );
                    });
                ui.label(RichText::new("Sources").strong());
                egui::ScrollArea::vertical()
                    .id_salt("approval-sources")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for source in plan.sources() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(files::display_path(source))
                                        .monospace()
                                        .size(12.0),
                                )
                                .wrap(),
                            );
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    let cancel_button = action_button(ui, "Cancel", "Escape", true);
                    if self.approval_focus {
                        cancel_button.request_focus();
                        self.approval_focus = false;
                    }
                    cancel = cancel_button.clicked();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let response =
                            action_button(ui, plan.operation().name(), "Ctrl Enter", true);
                        ui.ctx().accesskit_node_builder(response.id, |node| {
                            node.set_label(format!("Confirm {}", plan.operation().name()))
                        });
                        approve = response.clicked();
                    });
                });
                approve |= ui.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::Enter));
            });
            cancel |= modal.should_close();
            if cancel {
                self.plan = None;
                self.log("Plan cancelled. No filesystem changes.");
                self.file_focus_requested = true;
            }
            if approve && let Some(plan) = self.plan.take() {
                self.cancellation.store(false, Ordering::Relaxed);
                match self.work.try_send(Work::Execute(plan)) {
                    Ok(()) => {
                        self.operation_busy = true;
                        self.log("Executing approved plan...");
                    }
                    Err(error) => self.fail(format!("Operation was not queued: {error}")),
                }
            }
        }
        if let Some(error) = &self.error {
            let mut dismiss = false;
            let modal = egui::Modal::new(Id::new("error")).show(ctx, |ui| {
                ui.set_width(620.0);
                ui.heading("Attention / operation or configuration error");
                ui.label(error);
                dismiss = ui.button("Acknowledge / Escape").clicked();
            });
            dismiss |= modal.should_close();
            if dismiss {
                self.error = None;
            }
        }
    }
}

impl eframe::App for Ledger {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        enforce_stable_minimum_window_size(ctx, &mut self.viewport_zoom, self.preferences.zoom);
        self.zoom_input(ctx);
        #[cfg(feature = "native-desktop-tests")]
        if let Some(metrics) = &mut self.desktop_metrics {
            metrics.begin(ctx);
        }
        self.ai.poll();
        self.answers.poll();
        self.reader.poll();
        self.archive.poll();
        self.poll(ctx);
        if std::mem::take(&mut self.reader.edit_requested) {
            self.dispatch(Command::EditSource);
        }
        if std::mem::take(&mut self.reader.save_requested) {
            self.dispatch(Command::SaveSource);
        }
        if let Some(entry) = self.archive.open_entry.take() {
            let (path, _) = self.archive.archive_location();
            if let Err(error) = self.reader.start_virtual(self.archive.source, path, entry) {
                self.fail(error);
            }
        }
        self.shortcuts(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.data_mut(|data| data.remove::<DropHover>(Id::new("file-drop-hover")));
        if self.appearance_dirty {
            self.appearance_dirty = false;
            self.apply_theme(&ctx);
        }
        if self.palette
            || self.settings_open
            || self.operations_open
            || self
                .name_dialog
                .as_ref()
                .is_some_and(|dialog| !dialog.inline)
            || self.place_edit.is_some()
            || self.open_with.is_some()
            || self.change_plan.is_some()
            || self.zip_dialog.is_some()
            || (self.operation_busy && self.reader.dirty())
            || self.plan.is_some()
            || self.error.is_some()
            || self.search_dialog.is_some()
            || self.command_dialog.is_some()
            || self.close_after_save
            || self.ai.open
            || self.answers.open
        {
            ui.disable();
        }
        if self.operation_busy && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.fail("An operation is still running. Cancel work or wait for its exact outcome before closing.");
        } else if self.reader.dirty() && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.reader.close();
        } else if self.answers.busy() && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.answers.cancel();
            self.answers.open = true;
        } else if self.ai.busy() && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.ai.cancel();
            if self.search_dialog.is_none() {
                self.ai.open = true;
            }
        } else if ctx.input(|input| input.viewport().close_requested())
            && ((self.save_pending && self.preferences_writable) || self.saves_in_flight > 0)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_after_save = true;
        }
        if self.first_frame {
            self.first_frame = false;
            self.log(format!(
                "Native Ledger initialized in {:.0} ms / real filesystem",
                self.started.elapsed().as_secs_f64() * 1000.0
            ));
        }
        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(egui::Margin::symmetric(18, 14)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("[t_]")
                            .monospace()
                            .size(23.0)
                            .color(self.accent()),
                    );
                    ui.vertical(|ui| {
                        ui.heading("Tomas Commander");
                    });
                    let layout = if ui.available_width() < 420.0 {
                        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true)
                    } else {
                        egui::Layout::right_to_left(egui::Align::Center)
                    };
                    ui.with_layout(layout, |ui| {
                        if ui.button("Settings").clicked() {
                            self.dispatch(Command::Settings);
                        }
                        #[cfg(feature = "native-desktop-tests")]
                        if self.desktop_metrics.is_some()
                            && ui.button("Command guard tests").clicked()
                        {
                            self.dispatch(Command::AiPlan);
                        }
                        if action_button(ui, "Commands", "Ctrl Shift P", true).clicked() {
                            self.palette = true;
                            self.palette_focus = true;
                            self.palette_query.clear();
                        }
                        if action_button(
                            ui,
                            if self.preferences.dark {
                                "Light mode"
                            } else {
                                "Dark mode"
                            },
                            "",
                            true,
                        )
                        .clicked()
                        {
                            self.dispatch(Command::Theme);
                        }
                        if action_button(
                            ui,
                            &format!("Accent: {}", ACCENTS[self.preferences.accent].0),
                            "",
                            true,
                        )
                        .clicked()
                        {
                            self.dispatch(Command::Accent);
                        }
                    });
                });
            });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let available = self.plan.is_none()
                    && self.error.is_none()
                    && self.search_dialog.is_none()
                    && self.command_dialog.is_none()
                    && !self.ai.open
                    && !self.answers.open
                    && self.places_popup.is_none()
                    && self.place_edit.is_none();
                if ui
                    .add_enabled(available, egui::Button::new("? Help"))
                    .clicked()
                {
                    self.dispatch(Command::Help);
                }
                if ui
                    .button(format!("{:.0}%", self.preferences.zoom * 100.0))
                    .on_hover_text("UI zoom / click to reset / Ctrl +, Ctrl -, Ctrl wheel")
                    .clicked()
                {
                    self.dispatch(Command::ZoomReset);
                }
                if ui
                    .button(if self.operation_busy {
                        "Operations / running"
                    } else if self.last_outcome.as_ref().is_some_and(|o| {
                        o.error.is_some() || o.cancelled || !o.incomplete.is_empty()
                    }) {
                        "Operations / attention"
                    } else {
                        "Operations"
                    })
                    .clicked()
                {
                    self.operations_open = true;
                }
                if self.operation_busy && ui.button("Cancel work").clicked() {
                    self.cancellation.store(true, Ordering::Relaxed);
                }
                ui.add(
                    egui::Label::new(RichText::new(&self.status).monospace().small()).truncate(),
                )
                .on_hover_text(&self.status);
            });
        });
        egui::Panel::bottom("actions").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (command, label, hint) in [
                    (Command::Copy, "Copy", "Ctrl Shift C"),
                    (Command::Move, "Move", "Ctrl Shift M"),
                    (Command::Recycle, "Delete", "Ctrl Shift D"),
                    (Command::Refresh, "Refresh", "Ctrl R"),
                    (Command::Search, "Search", "Ctrl Shift F"),
                    (Command::AiSearch, "AI search", ""),
                    (Command::Rename, "Rename", "Ctrl Shift N"),
                    (Command::NewFolder, "New folder", "Ctrl N"),
                    (Command::OpenWith, "Open with", "Shift Enter"),
                ] {
                    let response = action_button(ui, label, hint, !self.operation_busy && self.plan.is_none());
                    if response.clicked() {
                        self.dispatch(command);
                    }
                    if matches!(command, Command::CopilotCli) {
                        response.on_hover_text("Runs copilot --yolo in this folder. The CLI can act without permission prompts.");
                    }
                }
            });
        });
        egui::Panel::left("context-rail")
            .resizable(true)
            .default_size(175.0)
            .size_range(145.0..=300.0)
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().extreme_bg_color)
                    .inner_margin(egui::Margin::symmetric(12, 16)),
            )
            .show(ui, |ui| {
                ui.label(
                    RichText::new("WORKSPACE / LEDGER")
                        .monospace()
                        .size(10.0)
                        .weak(),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().button_padding.x = 4.0;
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (rail, label) in [(Rail::Favorites, "Places"), (Rail::Recent, "Recent")] {
                        if ui
                            .selectable_value(&mut self.rail, rail, RichText::new(label).size(11.0))
                            .clicked()
                        {
                            self.rail_index = None;
                        }
                    }
                });
                ui.separator();
                match self.rail {
                    Rail::Favorites | Rail::Recent => {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(if self.rail == Rail::Recent {
                                "RECENT DOCUMENTS"
                            } else {
                                "PINNED LOCATIONS"
                            })
                            .monospace()
                            .size(10.0)
                            .weak(),
                        );
                        ui.add_space(6.0);
                        let mut target = None;
                        let mut remove = None;
                        let mut edit = None;
                        egui::ScrollArea::vertical()
                            .id_salt("pinned-locations")
                            .max_height((ui.available_height() - 170.0).max(60.0))
                            .animated(false)
                            .show(ui, |ui| {
                                let paths = if self.rail == Rail::Recent {
                                    &self.preferences.recent_documents
                                } else {
                                    &self.preferences.favorites
                                };
                                for (index, path) in paths.iter().enumerate() {
                                    let name = path
                                        .file_name()
                                        .map(|name| name.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| files::display_path(path));
                                    let response = if self.rail == Rail::Favorites {
                                        let name = self.preferences.favorite_name(path);
                                        let (response, rename, delete) = saved_place_row(
                                            ui,
                                            &name,
                                            path,
                                            self.rail_index == Some(index),
                                        );
                                        if rename {
                                            edit = Some((path.clone(), name));
                                        }
                                        if delete {
                                            remove = Some(path.clone());
                                        }
                                        response
                                    } else {
                                        ui.add(
                                            egui::Button::new(if self.rail == Rail::Recent {
                                                name
                                            } else {
                                                format!("/ {name}")
                                            })
                                            .min_size(Vec2::new(ui.available_width(), 30.0))
                                            .truncate()
                                            .frame(false)
                                            .selected(self.rail_index == Some(index)),
                                        )
                                        .on_hover_text(files::display_path(path))
                                    };
                                    if self.rail_index == Some(index) {
                                        response.request_focus();
                                        response.scroll_to_me(None);
                                    }
                                    if response.clicked() {
                                        target = Some(path.clone());
                                    }
                                }
                            });
                        if let Some(path) = remove {
                            self.preferences.remove_favorite(&path);
                            self.rail_index = None;
                            self.save_pending = true;
                        }
                        if edit.is_some() {
                            self.name_dialog = None;
                            self.inline_pending = None;
                            self.place_edit = edit;
                        }
                        if let Some(target) = target {
                            self.rail_index = None;
                            if self.rail == Rail::Recent {
                                if let Err(error) = self.work.try_send(Work::Open(target)) {
                                    self.fail(format!("Could not open recent document: {error}"));
                                }
                            } else {
                                self.navigate(self.active, target);
                            }
                        }

                        ui.add_space(25.0);
                        if self.rail == Rail::Favorites
                            && ui.button("+ Pin active folder").clicked()
                        {
                            self.dispatch(Command::Pin);
                        }
                        ui.label(
                            RichText::new(if self.rail == Rail::Favorites {
                                "Ctrl Shift B"
                            } else {
                                "Ctrl Shift H"
                            })
                            .small()
                            .weak(),
                        );
                    }
                }
            });
        let remaining = ui.available_width();
        if self.workspace_width > 0.0 && (remaining - self.workspace_width).abs() > 1.0 {
            self.left_width *= remaining / self.workspace_width;
            self.resize_requested = true;
        }
        self.workspace_width = remaining;
        if self.balance_requested {
            self.left_width = remaining / 2.0;
        }
        let pane_min = (remaining * 0.35).clamp(100.0, 230.0);
        self.left_width = self
            .left_width
            .clamp(pane_min, (remaining - pane_min).max(pane_min));
        if self.resize_requested || self.balance_requested {
            ctx.data_mut(|data| {
                data.remove::<egui::containers::panel::PanelState>(Id::new("left-files"))
            });
            self.resize_requested = false;
            self.balance_requested = false;
        }
        let left = egui::Panel::left("left-files")
            .resizable(true)
            .frame(egui::Frame::new().fill(ui.visuals().panel_fill))
            .default_size(self.left_width)
            .size_range(pane_min..=(remaining - pane_min).max(pane_min))
            .show(ui, |ui| {
                self.pane_ui(ui, 0);
            });
        self.left_width = left.response.rect.width();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(ui.visuals().panel_fill))
            .show(ui, |ui| {
                self.pane_ui(ui, 1);
            });
        self.dialogs(&ctx);
        self.ai.show(&ctx);
        self.answers.show(&ctx);
        self.finish_drag(&ctx);
        #[cfg(feature = "native-desktop-tests")]
        if let Some(metrics) = &mut self.desktop_metrics {
            let listing_ready = self.panes.iter().all(|pane| !pane.busy) && self.error.is_none();
            let pending_scan = self.panes.iter().any(|pane| {
                pane.busy
                    || pane
                        .entries
                        .iter()
                        .any(|entry| entry.directory && !entry.link && entry.aggregate.is_none())
            });
            let pending_search = self
                .search_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.busy);
            metrics.finish(listing_ready, pending_scan, pending_search);
        }
    }
}

// egui 0.36 text editors expose ValuePattern but do not process SetValue.
pub(crate) fn accessible_text_value(ctx: &egui::Context, id: Id, text: &mut String) -> bool {
    let mut changed = false;
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            if let egui::Event::AccessKitActionRequest(request) = event
                && request.target_node == id.accesskit_id()
                && request.action == egui::accesskit::Action::SetValue
                && let Some(egui::accesskit::ActionData::Value(value)) = &request.data
            {
                text.clear();
                text.push_str(value);
                changed = true;
                return false;
            }
            true
        })
    });
    if changed && let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        state.cursor.set_char_range(None);
        state.store(ctx, id);
    }
    changed
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1e6)
    } else if bytes >= 1000 {
        format!("{:.1} KB", bytes as f64 / 1000.0)
    } else {
        format!("{bytes} B")
    }
}

fn breadcrumb_paths(path: &std::path::Path, boundary: Option<&std::path::Path>) -> Vec<PathBuf> {
    let mut paths: Vec<_> = path
        .ancestors()
        .filter(|ancestor| boundary.is_none_or(|root| ancestor.starts_with(root)))
        .map(std::path::Path::to_owned)
        .collect();
    paths.reverse();
    paths
}

fn double_click_command(navigation: bool, modifiers: Modifiers) -> Option<Command> {
    if navigation {
        None
    } else if modifiers.ctrl {
        Some(Command::Preview)
    } else if modifiers.shift {
        Some(Command::OpenWith)
    } else {
        None
    }
}

fn inline_name_submitted(ui: &mut egui::Ui, edit: &egui::Response) -> bool {
    (edit.has_focus() || edit.lost_focus())
        && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter))
}

fn file_name_label(ui: &mut egui::Ui, rect: egui::Rect, name: &str) -> egui::Response {
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::monospace(12.0),
            ui.visuals().text_color(),
        );
    let response = ui.interact(
        rect,
        ui.id().with("filename"),
        egui::Sense::click_and_drag(),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), name));
    response.on_hover_cursor(egui::CursorIcon::Text)
}

fn saved_place_row(
    ui: &mut egui::Ui,
    name: &str,
    path: &std::path::Path,
    selected: bool,
) -> (egui::Response, bool, bool) {
    let mut edit = false;
    let mut delete = false;
    if ui.available_width() < 240.0 {
        let response = ui
            .add_sized(
                Vec2::new(ui.available_width(), 24.0),
                egui::Button::new(name).selected(selected).truncate(),
            )
            .on_hover_text(files::display_path(path));
        ui.horizontal(|ui| {
            edit = ui.small_button("Edit").clicked();
            delete = ui
                .small_button("Delete")
                .on_hover_text("Remove bookmark only; keep folder")
                .clicked();
        });
        return (response, edit, delete);
    }
    let response = ui
        .horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                delete = ui
                    .small_button("Delete")
                    .on_hover_text("Remove bookmark only; keep folder")
                    .clicked();
                edit = ui.small_button("Edit").clicked();
                ui.add_sized(
                    Vec2::new(ui.available_width(), 24.0),
                    egui::Button::new(name).selected(selected).truncate(),
                )
                .on_hover_text(files::display_path(path))
            })
            .inner
        })
        .inner;
    (response, edit, delete)
}

fn compact_breadcrumbs(ui: &mut egui::Ui, paths: &[PathBuf]) -> (Option<PathBuf>, bool) {
    if paths.is_empty() {
        return (None, false);
    }
    let width = ui.available_width();
    let gap = 4.0;
    let font = egui::TextStyle::Small.resolve(ui.style());
    let measure = |label: &str| {
        ui.painter()
            .layout_no_wrap(label.to_owned(), font.clone(), ui.visuals().text_color())
            .size()
            .x
            + ui.spacing().button_padding.x * 2.0
    };
    let edit_width = measure("Edit path").min(width * 0.6);
    let component_max = (width - edit_width - gap).max(1.0).min(width * 0.55);
    let labels: Vec<_> = paths
        .iter()
        .map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| files::display_path(path))
        })
        .collect();
    let widths: Vec<_> = labels
        .iter()
        .map(|label| measure(label).min(component_max))
        .collect();
    let ellipsis_width = measure("...").min(component_max);
    let mut chosen = Vec::new();
    for first in 0..paths.len() {
        let mut items = vec![Some(0)];
        if first > 1 {
            items.push(None);
        }
        items.extend((first.max(1)..paths.len()).map(Some));
        items.push(Some(paths.len()));
        let mut row = 0;
        let mut used = 0.0;
        let mut layout = Vec::new();
        for item in items {
            let item_width = match item {
                None => ellipsis_width,
                Some(i) if i == paths.len() => edit_width,
                Some(i) => widths[i],
            };
            if used > 0.0 && used + gap + item_width > width {
                row += 1;
                used = 0.0;
            }
            used += if used == 0.0 {
                item_width
            } else {
                gap + item_width
            };
            layout.push((row, item, item_width));
        }
        if row < 2 {
            chosen = layout;
            break;
        }
    }
    let mut clicked = None;
    let mut edit = false;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(gap, 2.0);
        for row in 0..2 {
            if !chosen.iter().any(|(r, _, _)| *r == row) {
                continue;
            }
            ui.horizontal_top(|ui| {
                for (_, item, item_width) in chosen.iter().filter(|(r, _, _)| *r == row) {
                    let label = match item {
                        None => "...",
                        Some(i) if *i == paths.len() => "Edit path",
                        Some(i) => &labels[*i],
                    };
                    let response = ui.add_sized(
                        Vec2::new(*item_width, 18.0),
                        egui::Button::new(RichText::new(label).small()).truncate(),
                    );
                    match item {
                        Some(i) if *i == paths.len() => edit |= response.clicked(),
                        Some(i) => {
                            if response
                                .on_hover_text(files::display_path(&paths[*i]))
                                .clicked()
                            {
                                clicked = Some(paths[*i].clone());
                            }
                        }
                        None => {
                            response.on_hover_text(files::display_path(paths.last().unwrap()));
                        }
                    }
                }
            });
        }
    });
    (clicked, edit)
}

fn consume_ui_zoom(ctx: &egui::Context, current: f32) -> f32 {
    ctx.input_mut(|input| {
        let mut delta = 0.0;
        let mut reset = false;
        let mut wheel = 1.0;
        input.events.retain(|event| {
            if let egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } = event
                && modifiers.ctrl
                && !modifiers.alt
                && !modifiers.mac_cmd
            {
                let speed = match unit {
                    egui::MouseWheelUnit::Line => 0.1,
                    egui::MouseWheelUnit::Point => 0.002,
                    egui::MouseWheelUnit::Page => 0.2,
                };
                wheel *= ((delta.x + delta.y) * speed).exp();
                input.smooth_scroll_delta = Vec2::ZERO;
                return false;
            }
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
                && modifiers.ctrl
                && !modifiers.alt
                && !modifiers.mac_cmd
            {
                match key {
                    Key::Plus | Key::Equals => delta += 0.1,
                    Key::Minus => delta -= 0.1,
                    Key::Num0 => reset = true,
                    _ => return true,
                }
                return false;
            }
            true
        });
        if reset {
            1.0
        } else {
            (current * wheel + delta).clamp(Preferences::MIN_ZOOM, Preferences::MAX_ZOOM)
        }
    })
}

fn consume_file_copy(ctx: &egui::Context, modifiers: &mut Modifiers, enabled: bool) -> bool {
    // egui-winit converts Ctrl+C to Copy even when Shift is held.
    ctx.input_mut(|input| {
        if !input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::ModifiersChanged(_)))
        {
            *modifiers = input.modifiers;
        }
        let mut copy = false;
        input.events.retain(|event| {
            if let egui::Event::ModifiersChanged(changed) = event {
                *modifiers = *changed;
            }
            if matches!(event, egui::Event::Copy)
                && enabled
                && modifiers.ctrl
                && modifiers.shift
                && !modifiers.alt
                && !modifiers.mac_cmd
            {
                copy = true;
                return false;
            }
            true
        });
        *modifiers = input.modifiers;
        copy
    })
}

fn consume_help(ctx: &egui::Context) -> bool {
    ctx.input_mut(|input| {
        let requested = input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Text(text) if text == "?"));
        input
            .events
            .retain(|event| !matches!(event, egui::Event::Text(text) if text == "?"));
        requested
    })
}

fn consume_navigation_key(ctx: &egui::Context, key: Key) -> Option<Modifiers> {
    ctx.input_mut(|input| {
        let modifiers = input.events.iter().find_map(|event| match event {
            egui::Event::Key {
                key: event_key,
                pressed: true,
                modifiers,
                ..
            } if *event_key == key
                && (*modifiers == Modifiers::NONE || *modifiers == Modifiers::SHIFT) =>
            {
                Some(*modifiers)
            }
            _ => None,
        })?;
        input.consume_key(modifiers, key).then_some(modifiers)
    })
}

fn enforce_minimum_window_size(ctx: &egui::Context) {
    let minimum = Vec2::from(MIN_WINDOW_SIZE) / ctx.zoom_factor();
    if let Some(size) = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.size()))
        && (size.x < minimum.x || size.y < minimum.y)
    {
        // Windows tiling tools can override the native minimum-size hint.
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.max(minimum)));
    }
}

fn enforce_stable_minimum_window_size(
    ctx: &egui::Context,
    previous_zoom: &mut f32,
    requested_zoom: f32,
) {
    let zoom = ctx.zoom_factor();
    // Native viewport rectangles can still use the previous scale on the transition pass.
    if zoom == *previous_zoom && zoom == requested_zoom {
        enforce_minimum_window_size(ctx);
    }
    *previous_zoom = zoom;
}

fn selection_footer(
    ui: &mut egui::Ui,
    index: usize,
    pane: &mut Pane,
    active: bool,
    accent: Color32,
    muted: Color32,
    border: egui::Stroke,
) -> egui::InnerResponse<()> {
    let bounds = ui.available_rect_before_wrap();
    let footer = egui::Panel::bottom(Id::new(("pane-footer", index)))
        .resizable(false)
        .frame(
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(12, 6))
                .stroke(border),
        )
        .show(ui, |ui| {
            pane_footer(ui, pane, active, accent, muted);
        });
    if footer.response.rect.bottom() > bounds.bottom() + 1.0 {
        ui.ctx()
            .request_discard("Selection footer grew after wrapping");
    }
    footer
}

fn pane_footer(ui: &mut egui::Ui, pane: &mut Pane, active: bool, accent: Color32, muted: Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!(
                "{} visible / {} selected",
                pane.visible.len(),
                pane.selected.len()
            ))
            .monospace()
            .small()
            .weak(),
        );
        let hidden = pane
            .selected
            .iter()
            .filter(|path| {
                !pane
                    .visible
                    .iter()
                    .any(|row| &pane.entries[*row].path == *path)
            })
            .count();
        if hidden > 0 {
            ui.label(format!("{hidden} selected hidden by filter"));
        }
        if ui
            .add_enabled(
                !pane.selected.is_empty(),
                egui::Button::new("Clear selection"),
            )
            .clicked()
        {
            pane.selected.clear();
        }
        ui.label(
            RichText::new(if active { "READY / FOCUSED" } else { "READY" })
                .monospace()
                .size(10.0)
                .color(if active { accent } else { muted }),
        );
    });
}

fn trailing_field(
    ui: &mut egui::Ui,
    field: egui::TextEdit<'_>,
    controls: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            controls(ui);
            ui.add_sized(
                Vec2::new(ui.available_width().max(1.0), ui.spacing().interact_size.y),
                field,
            )
        })
        .inner
    })
    .inner
}

fn action_button(ui: &mut egui::Ui, label: &str, hint: &str, enabled: bool) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let label_size = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), ui.visuals().text_color())
        .size();
    let hint_size = ui
        .painter()
        .layout_no_wrap(
            hint.to_owned(),
            egui::FontId::monospace(9.0),
            ui.visuals().weak_text_color(),
        )
        .size();
    let width = (label_size.x + ui.spacing().button_padding.x * 2.0).max(hint_size.x) + 4.0;
    let height = ui
        .spacing()
        .interact_size
        .y
        .max(label_size.y + ui.spacing().button_padding.y * 2.0)
        + 2.0
        + hint_size.y.max(12.0);
    ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let response = ui.add_enabled(
                enabled,
                egui::Button::new(RichText::new(label).font(font))
                    .wrap_mode(egui::TextWrapMode::Extend),
            );
            if hint.is_empty() {
                ui.add_space(12.0);
            } else {
                ui.label(RichText::new(hint).monospace().size(9.0).weak());
            }
            response
        },
    )
    .inner
}

fn format_modified(modified: Option<SystemTime>) -> String {
    let Some(time) = modified else {
        return "-".into();
    };
    let Ok(duration) = time.duration_since(SystemTime::UNIX_EPOCH) else {
        return "pre-1970".into();
    };
    let mut days = duration.as_secs() / 86_400;
    let mut year = 1970;
    loop {
        let leap = year % 400 == 0 || (year % 4 == 0 && year % 100 != 0);
        let length = if leap { 366 } else { 365 };
        if days < length {
            break;
        }
        days -= length;
        year += 1;
        if year > 9999 {
            return ">9999".into();
        }
    }
    let leap = year % 400 == 0 || (year % 4 == 0 && year % 100 != 0);
    let lengths = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0;
    while days >= lengths[month] {
        days -= lengths[month];
        month += 1;
    }
    format!(
        "{year:04}-{:02}-{:02} {:02}:{:02}",
        month + 1,
        days + 1,
        duration.as_secs() % 86_400 / 3600,
        duration.as_secs() % 3600 / 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_wheel_uses_event_modifiers_and_subpercent_deltas_without_rounding_loss() {
        let ctx = egui::Context::default();
        let mut zoom = 1.0;
        for unit in [
            egui::MouseWheelUnit::Point,
            egui::MouseWheelUnit::Line,
            egui::MouseWheelUnit::Page,
        ] {
            ctx.run_ui(
                egui::RawInput {
                    events: vec![
                        egui::Event::MouseWheel {
                            unit,
                            delta: Vec2::new(0.0, 1.0),
                            phase: egui::TouchPhase::Move,
                            modifiers: Modifiers::CTRL,
                        },
                        egui::Event::ModifiersChanged(Modifiers::NONE),
                    ],
                    ..Default::default()
                },
                |_| {
                    zoom = consume_ui_zoom(&ctx, 1.0);
                    assert!(zoom > 1.0);
                    assert!(!ctx.input(|i| {
                        i.events
                            .iter()
                            .any(|e| matches!(e, egui::Event::MouseWheel { .. }))
                    }));
                    assert_eq!(ctx.input(|i| i.smooth_scroll_delta), Vec2::ZERO);
                },
            )
            .textures_delta
            .clear();
        }
        ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: Vec2::new(0.0, -1.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Modifiers::CTRL,
                }],
                ..Default::default()
            },
            |_| assert!(consume_ui_zoom(&ctx, 1.0) < 1.0),
        )
        .textures_delta
        .clear();
        ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: Vec2::new(0.0, 1.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |_| {
                assert_eq!(consume_ui_zoom(&ctx, 1.0), 1.0);
                assert!(ctx.input(|i| {
                    i.events
                        .iter()
                        .any(|e| matches!(e, egui::Event::MouseWheel { .. }))
                }));
            },
        )
        .textures_delta
        .clear();
    }

    #[test]
    fn breadcrumbs_keep_exact_paths_and_never_offer_fixture_ancestors() {
        let root = std::path::Path::new(r"C:\owned");
        let path = root.join("Folder").join("Nested");
        assert_eq!(
            breadcrumb_paths(&path, Some(root)),
            vec![root.to_owned(), root.join("Folder"), path.clone()]
        );
        assert_eq!(breadcrumb_paths(&path, None).last(), Some(&path));
    }

    #[test]
    fn compact_breadcrumbs_render_within_two_rows_even_for_long_components() {
        let path = PathBuf::from(
            r"C:\very long ancestor name\another ancestor\947dfde0-a1eb-40dc-b348-628e8df8e692\files\nested\current folder",
        );
        for width in [140.0, 200.0, 400.0, 800.0] {
            let ctx = egui::Context::default();
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(width, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let bounds = ui.max_rect();
                    compact_breadcrumbs(ui, &breadcrumb_paths(&path, None));
                    assert!(ui.min_rect().height() > 0.0);
                    assert!(
                        ui.min_rect().height()
                            <= 2.0 * ui.spacing().interact_size.y.max(18.0) + 4.0
                    );
                    assert!(ui.min_rect().right() <= bounds.right() + 0.1);
                },
            )
            .textures_delta
            .clear();
        }
    }

    #[test]
    fn inline_filename_enter_submits_when_singleline_editor_loses_focus() {
        let ctx = egui::Context::default();
        let mut name = String::from("renamed-alpha.txt");
        let mut submitted = false;
        for frame in 0..3 {
            let events = if frame == 2 {
                vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: Some(Key::Enter),
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }]
            } else {
                Vec::new()
            };
            ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    let edit =
                        ui.add(egui::TextEdit::singleline(&mut name).id(Id::new("inline-test")));
                    if frame == 0 {
                        edit.request_focus();
                    }
                    submitted |= inline_name_submitted(ui, &edit);
                },
            )
            .textures_delta
            .clear();
        }
        assert!(submitted);
        assert_eq!(name, "renamed-alpha.txt");
    }

    #[test]
    fn filename_click_and_selection_keep_exact_text_position() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(80.0, 60.0), Vec2::new(300.0, 29.0));
        let mut positions = Vec::new();
        let mut clicks = 0;
        for (frame, pressed) in [None, Some(true), Some(false)].into_iter().enumerate() {
            let events = pressed
                .map(|pressed| {
                    vec![
                        egui::Event::PointerMoved(rect.center()),
                        egui::Event::PointerButton {
                            pos: rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Modifiers::NONE,
                        },
                    ]
                })
                .unwrap_or_default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(frame as f64 * 0.05),
                    events,
                    ..Default::default()
                },
                |ui| {
                    if frame > 0 {
                        ui.painter().rect_filled(rect, 0.0, Color32::GREEN);
                    }
                    let row = ui.interact(
                        rect.expand2(Vec2::new(30.0, 0.0)),
                        ui.id().with("file"),
                        egui::Sense::click_and_drag(),
                    );
                    let label = file_name_label(ui, rect, "alpha.txt");
                    let response = row.union(label.clone());
                    assert_eq!(response.clicked(), label.clicked());
                    assert_eq!(label.rect, rect);
                    clicks += usize::from(response.clicked());
                },
            );
            for shape in &output.shapes {
                if let egui::epaint::Shape::Text(text) = &shape.shape
                    && text.galley.job.text == "alpha.txt"
                {
                    positions.push(text.pos);
                }
            }
            output.textures_delta.clear();
        }
        assert_eq!(clicks, 1);
        assert_eq!(positions.len(), 3);
        assert!(positions.iter().all(|position| position == &positions[0]));
        assert_eq!(positions[0].x, rect.left());
    }

    #[test]
    fn filename_accessibility_click_uses_the_inline_rename_hit_target() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(80.0, 60.0), Vec2::new(300.0, 29.0));
        let mut id = Id::NULL;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            id = file_name_label(ui, rect, "alpha.txt").id;
        })
        .textures_delta
        .clear();
        ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::AccessKitActionRequest(
                    egui::accesskit::ActionRequest {
                        action: egui::accesskit::Action::Click,
                        target_tree: egui::accesskit::TreeId::ROOT,
                        target_node: id.accesskit_id(),
                        data: None,
                    },
                )],
                ..Default::default()
            },
            |ui| {
                let label = file_name_label(ui, rect, "alpha.txt");
                assert!(label.clicked());
            },
        )
        .textures_delta
        .clear();
    }

    #[test]
    fn filename_right_click_reaches_the_shared_row_context_menu() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(80.0, 60.0), Vec2::new(300.0, 29.0));
        let mut menu_opened = false;
        for pressed in [None, Some(true), Some(false)] {
            let events = pressed
                .map(|pressed| {
                    vec![
                        egui::Event::PointerMoved(rect.center()),
                        egui::Event::PointerButton {
                            pos: rect.center(),
                            button: egui::PointerButton::Secondary,
                            pressed,
                            modifiers: Modifiers::NONE,
                        },
                    ]
                })
                .unwrap_or_default();
            ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    let row = ui.interact(
                        rect.expand2(Vec2::new(30.0, 0.0)),
                        ui.id().with("file"),
                        egui::Sense::click_and_drag(),
                    );
                    let label = file_name_label(ui, rect, "alpha.txt");
                    row.union(label).context_menu(|ui| {
                        menu_opened = true;
                        ui.label("Open with");
                    });
                },
            )
            .textures_delta
            .clear();
        }
        assert!(menu_opened);
    }

    #[test]
    fn modified_double_click_matches_enter_actions_without_renaming() {
        assert!(double_click_command(false, Modifiers::NONE).is_none());
        assert!(matches!(
            double_click_command(false, Modifiers::CTRL),
            Some(Command::Preview)
        ));
        assert!(matches!(
            double_click_command(false, Modifiers::SHIFT),
            Some(Command::OpenWith)
        ));
        assert!(double_click_command(true, Modifiers::CTRL).is_none());
        assert!(double_click_command(true, Modifiers::SHIFT).is_none());
    }

    #[test]
    fn egui_pointer_double_click_dispatches_modifiers_with_drag_sense() {
        for modifiers in [Modifiers::NONE, Modifiers::CTRL, Modifiers::SHIFT] {
            let ctx = egui::Context::default();
            let mut position = egui::Pos2::ZERO;
            let mut double_clicks = 0;
            for (frame, pressed) in [None, Some(true), Some(false), Some(true), Some(false)]
                .into_iter()
                .enumerate()
            {
                let mut events = vec![egui::Event::ModifiersChanged(modifiers)];
                if let Some(pressed) = pressed {
                    events.push(egui::Event::PointerMoved(position));
                    events.push(egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    });
                }
                ctx.run_ui(
                    egui::RawInput {
                        time: Some(frame as f64 * 0.05),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::new(400.0, ROW_HEIGHT),
                            egui::Sense::hover(),
                        );
                        let row = egui::Rect::from_min_max(
                            egui::pos2(rect.left() + 30.0, rect.top()),
                            rect.max,
                        );
                        let response = file_name_label(ui, row, "alpha.txt");
                        position = response.rect.center();
                        if response.double_clicked() {
                            double_clicks += 1;
                            let command =
                                double_click_command(false, ui.input(|input| input.modifiers));
                            assert!(match modifiers {
                                Modifiers::CTRL => matches!(command, Some(Command::Preview)),
                                Modifiers::SHIFT => matches!(command, Some(Command::OpenWith)),
                                _ => command.is_none(),
                            });
                        }
                    },
                )
                .textures_delta
                .clear();
            }
            assert_eq!(double_clicks, 1);
        }
    }

    #[test]
    fn pane_text_controls_never_expand_the_resizable_panel() {
        for width in [200.0, 400.0, 800.0] {
            for path_edit in [false, true] {
                let ctx = egui::Context::default();
                let mut text = r"C:\long folder\another folder".to_owned();
                for _ in 0..12 {
                    ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                Vec2::new(1400.0, 900.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            let panel = egui::Panel::left("field-panel")
                                .default_size(width)
                                .frame(egui::Frame::NONE)
                                .show(ui, |ui| {
                                    egui::Frame::new()
                                        .inner_margin(egui::Margin::symmetric(12, 10))
                                        .show(ui, |ui| {
                                            let bounds = ui.max_rect();
                                            let response = trailing_field(
                                                ui,
                                                egui::TextEdit::singleline(&mut text)
                                                    .margin(Vec2::new(7.0, 6.0)),
                                                |ui| {
                                                    if path_edit {
                                                        let _ = ui.small_button("Done");
                                                        let _ = ui.button("Go");
                                                    } else {
                                                        let _ = ui.button("x");
                                                    }
                                                },
                                            );
                                            assert!(response.rect.left() >= bounds.left() - 0.1);
                                            assert!(response.rect.right() <= bounds.right() + 0.1);
                                            assert!(response.rect.height() < 40.0);
                                            assert!(ui.min_rect().height() < 60.0);
                                        });
                                });
                            assert!(
                                (panel.response.rect.width() - width).abs() < 1.0,
                                "panel grew from {width} to {}",
                                panel.response.rect.width()
                            );
                        },
                    )
                    .textures_delta
                    .clear();
                }
            }
        }
    }

    #[test]
    fn wrapped_selection_footer_reserves_space_above_the_action_bar() {
        for width in [180.0, 300.0, 800.0] {
            let ctx = egui::Context::default();
            let mut pane = Pane::new(PathBuf::from("fixture"));
            pane.selected.insert(PathBuf::from("hidden.txt"));
            let mut checked = 0;
            for _ in 0..3 {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(width, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let bounds = ui.max_rect();
                        let footer = selection_footer(
                            ui,
                            0,
                            &mut pane,
                            true,
                            Color32::BLUE,
                            Color32::GRAY,
                            egui::Stroke::NONE,
                        );
                        if ctx.will_discard() {
                            return;
                        }
                        checked += 1;
                        assert!(
                            footer.response.rect.bottom() <= bounds.bottom() + 0.1,
                            "footer {:?} outside {bounds:?}",
                            footer.response.rect
                        );
                        assert!(footer.response.rect.width() <= bounds.width() + 0.1);
                        assert!(
                            ui.available_rect_before_wrap().bottom()
                                <= footer.response.rect.top() + 0.1
                        );
                        if width < 400.0 {
                            assert!(footer.response.rect.height() > 42.0);
                        }
                    },
                )
                .textures_delta
                .clear();
            }
            assert!(checked >= 3, "footer never reached a publishable layout");
        }
    }

    #[test]
    fn drops_prepare_copy_move_and_zip_requests_without_execution() {
        use tomas_commander::archive_changes::Request;
        let sources = vec![
            PathBuf::from(r"C:\root\alpha.txt"),
            PathBuf::from(r"C:\root\beta.txt"),
        ];
        let destination = PathBuf::from(r"C:\destination");
        let archive = PathBuf::from(r"C:\root\sample.zip");
        for copy in [false, true] {
            let job = drop_work(
                7,
                DragItems::Files(sources.clone()),
                DropLocation::Folder(destination.clone()),
                copy,
            )
            .unwrap()
            .unwrap();
            assert!(
                matches!(job, Work::Prepare { id: 7, operation, sources: paths, destination: target }
                if operation == if copy { Operation::Copy } else { Operation::Move } && paths == sources && target == destination)
            );
            let job = drop_work(
                8,
                DragItems::Files(sources.clone()),
                DropLocation::Zip {
                    archive: archive.clone(),
                    prefix: "nested/".into(),
                },
                copy,
            )
            .unwrap()
            .unwrap();
            assert!(
                matches!(job, Work::PrepareArchive { id: 8, request: Request::Insert { archive: target, prefix, sources: paths, move_sources } }
                if target == archive && prefix == "nested/" && paths == sources && move_sources == !copy)
            );
            let job = drop_work(
                9,
                DragItems::Zip {
                    archive: archive.clone(),
                    prefix: "nested/".into(),
                    selected: vec!["nested/member.txt".into()],
                },
                DropLocation::Folder(destination.clone()),
                copy,
            )
            .unwrap()
            .unwrap();
            assert!(
                matches!(job, Work::PrepareArchive { id: 9, request: Request::Extract { archive: source, prefix, selected, destination: target, move_entries } }
                if source == archive && prefix == "nested/" && selected == ["nested/member.txt"] && target == destination && move_entries == !copy)
            );
        }
        assert!(
            drop_work(
                10,
                DragItems::Files(sources.clone()),
                DropLocation::Folder(PathBuf::from(r"C:\root")),
                false
            )
            .unwrap()
            .is_none()
        );
        assert!(
            drop_work(
                11,
                DragItems::Files(vec![destination.clone()]),
                DropLocation::Folder(destination),
                false
            )
            .unwrap()
            .is_none()
        );
        assert!(
            drop_work(
                12,
                DragItems::Files(Vec::new()),
                DropLocation::Folder(PathBuf::from(r"C:\other")),
                true
            )
            .is_err()
        );
        assert!(
            drop_work(
                13,
                DragItems::Zip {
                    archive: archive.clone(),
                    prefix: String::new(),
                    selected: vec!["member.txt".into()]
                },
                DropLocation::Zip {
                    archive,
                    prefix: String::new()
                },
                true
            )
            .is_err()
        );
    }

    #[test]
    fn drag_uses_selected_group_only_when_grabbed_item_is_selected() {
        let mut pane = Pane::new(PathBuf::from("root"));
        let a = PathBuf::from("a");
        let b = PathBuf::from("b");
        let c = PathBuf::from("c");
        pane.selected = BTreeSet::from([a.clone(), b.clone()]);
        assert_eq!(pane.drag_targets(&a), vec![a.clone(), b]);
        assert_eq!(pane.drag_targets(&c), vec![c]);
        assert_eq!(pane.selected.len(), 2);
    }

    #[test]
    fn zoom_shortcuts_are_global_bounded_and_leave_unrelated_text_alone() {
        let ctx = egui::Context::default();
        let input = |key, modifiers| egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        };
        let mut zoom = 1.0;
        ctx.run_ui(input(Key::Plus, Modifiers::CTRL | Modifiers::SHIFT), |_| {
            zoom = consume_ui_zoom(&ctx, zoom);
            assert!(ctx.input(|input| input.events.is_empty()));
        })
        .textures_delta
        .clear();
        assert!((zoom - 1.1).abs() < 0.001);
        ctx.run_ui(input(Key::Equals, Modifiers::CTRL), |_| {
            zoom = consume_ui_zoom(&ctx, Preferences::MAX_ZOOM);
        })
        .textures_delta
        .clear();
        assert_eq!(zoom, Preferences::MAX_ZOOM);
        ctx.run_ui(input(Key::Minus, Modifiers::CTRL), |_| {
            zoom = consume_ui_zoom(&ctx, Preferences::MIN_ZOOM);
        })
        .textures_delta
        .clear();
        assert_eq!(zoom, Preferences::MIN_ZOOM);
        ctx.run_ui(input(Key::Num0, Modifiers::CTRL), |_| {
            zoom = consume_ui_zoom(&ctx, zoom);
        })
        .textures_delta
        .clear();
        assert_eq!(zoom, 1.0);
        ctx.run_ui(input(Key::Plus, Modifiers::NONE), |_| {
            assert_eq!(consume_ui_zoom(&ctx, 1.0), 1.0);
            assert_eq!(ctx.input(|input| input.events.len()), 1);
        })
        .textures_delta
        .clear();
    }

    #[test]
    fn ctrl_wheel_scales_entire_ui_instead_of_scrolling() {
        let ctx = egui::Context::default();
        let mut zoom = 1.0;
        ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(Modifiers::CTRL),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Line,
                        delta: Vec2::new(0.0, 1.0),
                        phase: egui::TouchPhase::Move,
                        modifiers: Modifiers::CTRL,
                    },
                ],
                ..Default::default()
            },
            |_| {
                zoom = consume_ui_zoom(&ctx, 1.0);
                ctx.set_zoom_factor(zoom);
                assert_eq!(ctx.input(|input| input.smooth_scroll_delta), Vec2::ZERO);
            },
        )
        .textures_delta
        .clear();
        ctx.run_ui(egui::RawInput::default(), |_| {
            assert!((ctx.zoom_factor() - zoom).abs() < 0.001)
        })
        .textures_delta
        .clear();
        assert!(zoom > 1.0);
    }

    #[test]
    fn drop_folder_row_wins_over_pane_background_in_either_order() {
        let ctx = egui::Context::default();
        let root = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::splat(400.0));
        let row = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), Vec2::splat(100.0));
        for reverse in [false, true] {
            ctx.run_ui(egui::RawInput {
                events: vec![egui::Event::PointerMoved(egui::pos2(20.0, 20.0))],
                ..Default::default()
            }, |_| {
                ctx.data_mut(|data| data.remove::<DropHover>(Id::new("file-drop-hover")));
                egui::DragAndDrop::set_payload(&ctx, DragItems::Files(vec![PathBuf::from("source")]));
                let parent = || register_drop_zone(&ctx, root, DropLocation::Folder(PathBuf::from("parent")), 0);
                let child = || register_drop_zone(&ctx, row, DropLocation::Folder(PathBuf::from("child")), 2);
                if reverse { child(); parent(); } else { parent(); child(); }
                let hover = ctx.data(|data| data.get_temp::<DropHover>(Id::new("file-drop-hover"))).unwrap();
                assert!(matches!(hover.location, DropLocation::Folder(path) if path == std::path::Path::new("child")));
            }).textures_delta.clear();
        }
    }

    #[test]
    fn palette_matching_is_offline_bounded_and_measured() {
        for trial in 0..3 {
            let started = Instant::now();
            let query = "natural-language".to_lowercase();
            let matches: Vec<_> = COMMANDS
                .iter()
                .filter(|(_, name, _)| name.to_lowercase().contains(&query))
                .collect();
            assert_eq!(matches.len(), 1);
            assert!(matches!(matches[0].0, Command::AiPlan));
            println!(
                "CMD-01 PALETTE matching/typed dispatch selection trial {trial}: {} us",
                started.elapsed().as_micros()
            );
        }
        assert!(
            !COMMANDS
                .iter()
                .any(|(_, name, _)| name.contains("nonexistent command"))
        );
    }

    fn entry(name: &str) -> Entry {
        Entry {
            path: PathBuf::from(name),
            name: name.into(),
            directory: false,
            link: false,
            size: 1,
            modified: None,
            folded: name.to_lowercase(),
            aggregate: None,
        }
    }

    #[test]
    fn ten_row_jumps_clamp_and_preserve_selection() {
        let mut pane = Pane::new(PathBuf::from("fixture"));
        pane.entries = (0..25)
            .map(|row| entry(&format!("file-{row:02}")))
            .collect();
        pane.rebuild();
        pane.selected.insert(PathBuf::from("file-03"));
        pane.jump(true);
        assert_eq!(pane.focus_position(), 10);
        pane.jump(true);
        assert_eq!(pane.focus_position(), 20);
        pane.jump(true);
        assert_eq!(pane.focus_position(), 25);
        pane.jump(false);
        assert_eq!(pane.focus_position(), 15);
        pane.jump(false);
        pane.jump(false);
        assert_eq!(pane.focus_position(), 0);
        assert_eq!(pane.targets(), vec![PathBuf::from("file-03")]);
        pane.filter = "missing".into();
        pane.rebuild();
        pane.jump(true);
        assert_eq!(pane.focus_position(), 0);
        assert!(pane.scroll_to_focus);
    }

    #[test]
    fn help_uses_text_character_instead_of_a_layout_specific_physical_key() {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            events: vec![
                egui::Event::Text("?".into()),
                egui::Event::Text("unchanged".into()),
            ],
            ..Default::default()
        };
        ctx.run_ui(raw, |_| {
            assert!(consume_help(&ctx));
            assert!(!consume_help(&ctx));
            assert!(ctx.input(|input| {
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Text(text) if text == "unchanged"))
            }));
        })
        .textures_delta
        .clear();
    }

    #[test]
    fn parent_returns_are_per_pane_bounded_and_invalidated_only_by_new_navigation() {
        let mut left = Pane::new(PathBuf::from(r"C:\root\child\grandchild"));
        let right = Pane::new(PathBuf::from(r"C:\other"));
        left.filter = "alpha".into();
        left.focused = Some(left.path.join("alpha.txt"));
        left.scroll_offset = 42.0;
        let history = left.remember_parent_return();
        left.begin_navigation(PathBuf::from(r"C:\root\child"));
        left.parent_returns = history;
        let history = left.remember_parent_return();
        left.begin_navigation(PathBuf::from(r"C:\root"));
        left.parent_returns = history;
        assert_eq!(left.parent_returns.len(), 2);
        assert!(right.parent_returns.is_empty());
        left.begin_navigation(left.path.clone());
        assert_eq!(left.parent_returns.len(), 2);
        let child = left.parent_returns.pop().unwrap();
        assert_eq!(child.path, PathBuf::from(r"C:\root\child"));
        let grandchild = left.parent_returns.pop().unwrap();
        assert_eq!(grandchild.filter, "alpha");
        assert_eq!(grandchild.scroll_offset, 42.0);
        assert_eq!(grandchild.focused, Some(grandchild.path.join("alpha.txt")));
        for _ in 0..70 {
            left.parent_returns = left.remember_parent_return();
        }
        assert_eq!(left.parent_returns.len(), 64);
        left.begin_navigation(PathBuf::from(r"C:\unrelated"));
        assert!(left.parent_returns.is_empty());
    }

    #[test]
    fn filtering_preserves_path_selection_and_resets_range_anchor() {
        let mut pane = Pane::new(PathBuf::from("fixture"));
        pane.entries = vec![entry("Alpha"), entry("Beta"), entry("Gamma")];
        pane.rebuild();
        pane.focused = Some(PathBuf::from("Gamma"));
        pane.anchor = 2;
        pane.selected.insert(PathBuf::from("Beta"));
        pane.filter = "ALPHA".into();
        pane.rebuild();
        assert_eq!(pane.focused, Some(PathBuf::from("Alpha")));
        assert_eq!(pane.anchor, 1);
        assert_eq!(pane.targets(), vec![PathBuf::from("Beta")]);
        pane.filter = "missing".into();
        pane.rebuild();
        assert!(pane.focused.is_none());
        assert!(pane.visible.is_empty());
    }

    #[test]
    fn parent_row_remains_first_and_never_becomes_an_operation_target() {
        let mut pane = Pane::new(PathBuf::from("fixture"));
        pane.rebuild();
        assert_eq!(pane.focus_position(), 0);
        assert_eq!(pane.row_path(0), None);
        assert!(pane.targets().is_empty());
        pane.entries = vec![entry("Zulu"), entry("Alpha")];
        pane.rebuild();
        files::sort_entries(&mut pane.entries, Sort::Size);
        pane.focused = pane.row_path(0);
        assert!(pane.targets().is_empty());
        pane.filter = "absent".into();
        pane.rebuild();
        assert_eq!(pane.focus_position(), 0);
        assert!(pane.targets().is_empty());
        pane.filter.clear();
        pane.rebuild();
        let range: Vec<_> = (0..=2)
            .filter_map(|position| pane.row_path(position))
            .collect();
        assert_eq!(range.len(), 2);
        assert!(!range.contains(&pane.path));
    }

    #[test]
    fn accessibility_editor_replaces_only_the_target_value_once() {
        let ctx = egui::Context::default();
        let id = Id::new(("path", 0));
        let request = egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::SetValue,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: id.accesskit_id(),
            data: Some(egui::accesskit::ActionData::Value(
                "C:\\Czech folder".into(),
            )),
        };
        let raw = egui::RawInput {
            events: vec![egui::Event::AccessKitActionRequest(request)],
            ..Default::default()
        };
        let mut value = "old".to_string();
        let mut unrelated = "preserve".to_string();
        let mut output = ctx.run_ui(raw, |_| {
            assert!(!accessible_text_value(
                &ctx,
                Id::new(("filter", 0)),
                &mut unrelated
            ));
            assert!(accessible_text_value(&ctx, id, &mut value));
            assert!(!accessible_text_value(&ctx, id, &mut value));
        });
        output.textures_delta.clear();
        assert_eq!(value, "C:\\Czech folder");
        assert_eq!(unrelated, "preserve");
    }

    #[test]
    fn sorting_retains_focused_item_identity() {
        let mut pane = Pane::new(PathBuf::from("fixture"));
        pane.entries = vec![entry("Zulu"), entry("Alpha")];
        pane.rebuild();
        pane.focused = Some(PathBuf::from("Zulu"));
        files::sort_entries(&mut pane.entries, Sort::Name);
        pane.rebuild();
        assert_eq!(pane.focus_position(), 2);
        assert_eq!(pane.targets(), vec![PathBuf::from("Zulu")]);
    }

    #[test]
    fn visible_keyboard_navigation_does_not_scroll() {
        for position in 0..10 {
            assert_eq!(reveal_row(0.0, position, ROW_HEIGHT * 10.0), 0.0);
        }
        assert_eq!(
            reveal_row(ROW_HEIGHT * 5.0, 8, ROW_HEIGHT * 10.0),
            ROW_HEIGHT * 5.0
        );
    }

    #[test]
    fn keyboard_navigation_reveals_only_the_hidden_row() {
        assert_eq!(reveal_row(0.0, 10, ROW_HEIGHT * 10.0), ROW_HEIGHT);
        assert_eq!(
            reveal_row(ROW_HEIGHT * 5.0, 4, ROW_HEIGHT * 10.0),
            ROW_HEIGHT * 4.0
        );
        assert_eq!(reveal_row(0.0, 239, ROW_HEIGHT * 10.0), ROW_HEIGHT * 230.0);
        assert_eq!(reveal_row(ROW_HEIGHT * 230.0, 0, ROW_HEIGHT * 10.0), 0.0);
    }

    #[test]
    fn file_columns_fit_narrow_panes_without_overlapping() {
        for width in [230.0, 350.0, 600.0] {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, ROW_HEIGHT));
            let columns = RowColumns::new(rect);
            assert_eq!(columns.name.left(), 56.0);
            assert!(columns.name.width() >= 0.0);
            assert!(columns.name.right() < columns.size.left());
            assert!(columns.size.right() < columns.age.left());
            assert_eq!(columns.age.right(), width - 10.0);
        }
        for width in [100.0, 150.0, 200.0] {
            let columns = RowColumns::new(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, ROW_HEIGHT),
            ));
            assert!(columns.name.width() > 0.0);
            assert_eq!(columns.name.right(), width - 10.0);
            assert_eq!(columns.size.width(), 0.0);
            assert_eq!(columns.age.width(), 0.0);
        }
    }

    #[test]
    fn modified_dates_are_utc_and_handle_gregorian_leap_days() {
        assert_eq!(format_modified(None), "-");
        assert_eq!(
            format_modified(Some(SystemTime::UNIX_EPOCH)),
            "1970-01-01 00:00"
        );
        let leap = SystemTime::UNIX_EPOCH + Duration::from_secs(951_827_696);
        assert_eq!(format_modified(Some(leap)), "2000-02-29 12:34");
        let century = SystemTime::UNIX_EPOCH + Duration::from_secs(4_107_542_400);
        assert_eq!(format_modified(Some(century)), "2100-03-01 00:00");
    }

    #[test]
    fn clipboard_copy_translation_routes_only_the_file_chord() {
        let chord = Modifiers {
            ctrl: true,
            command: true,
            shift: true,
            ..Modifiers::NONE
        };
        for (enabled, shift, expected) in [
            (true, true, true),
            (true, false, false),
            (false, true, false),
        ] {
            let ctx = egui::Context::default();
            let modifiers = Modifiers { shift, ..chord };
            let raw = egui::RawInput {
                events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Copy],
                ..Default::default()
            };
            let mut previous = modifiers;
            let mut output = ctx.run_ui(raw, |_| {
                assert_eq!(consume_file_copy(&ctx, &mut previous, enabled), expected);
                assert_eq!(
                    ctx.input(|input| input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Copy))),
                    !expected
                );
            });
            output.textures_delta.clear();
        }
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            events: vec![
                egui::Event::ModifiersChanged(chord),
                egui::Event::Copy,
                egui::Event::ModifiersChanged(Modifiers::NONE),
            ],
            ..Default::default()
        };
        let mut previous = Modifiers::NONE;
        let mut output = ctx.run_ui(raw, |_| {
            assert!(consume_file_copy(&ctx, &mut previous, true));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn range_navigation_uses_key_event_modifiers_after_shift_release() {
        for key in [Key::ArrowUp, Key::ArrowDown, Key::Home, Key::End] {
            let ctx = egui::Context::default();
            let raw = egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(Modifiers::SHIFT),
                    egui::Event::Key {
                        key,
                        physical_key: Some(key),
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::SHIFT,
                    },
                    egui::Event::ModifiersChanged(Modifiers::NONE),
                ],
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |_| {
                assert_eq!(ctx.input(|input| input.modifiers), Modifiers::NONE);
                assert_eq!(consume_navigation_key(&ctx, key), Some(Modifiers::SHIFT));
                assert_eq!(consume_navigation_key(&ctx, key), None);
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn navigation_does_not_inherit_later_shift_or_consume_other_chords() {
        for modifiers in [Modifiers::NONE, Modifiers::CTRL, Modifiers::ALT] {
            let ctx = egui::Context::default();
            let raw = egui::RawInput {
                events: vec![
                    egui::Event::Key {
                        key: Key::ArrowDown,
                        physical_key: Some(Key::ArrowDown),
                        pressed: true,
                        repeat: false,
                        modifiers,
                    },
                    egui::Event::ModifiersChanged(Modifiers::SHIFT),
                ],
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |_| {
                assert_eq!(ctx.input(|input| input.modifiers), Modifiers::SHIFT);
                let expected = (modifiers == Modifiers::NONE).then_some(Modifiers::NONE);
                assert_eq!(consume_navigation_key(&ctx, Key::ArrowDown), expected);
                assert_eq!(
                    ctx.input(|input| input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))),
                    expected.is_none()
                );
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn undersized_native_viewports_recover_without_resizing_valid_windows() {
        for (size, expected) in [
            (Vec2::new(381.0, 970.0), Some(Vec2::new(880.0, 970.0))),
            (Vec2::new(400.0, 300.0), Some(Vec2::new(880.0, 560.0))),
            (Vec2::new(880.0, 560.0), None),
            (Vec2::new(1240.0, 800.0), None),
        ] {
            let ctx = egui::Context::default();
            let mut raw = egui::RawInput::default();
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .inner_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size));
            let mut output = ctx.run_ui(raw, |_| enforce_minimum_window_size(&ctx));
            let requested = output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .find_map(|command| match command {
                    egui::ViewportCommand::InnerSize(size) => Some(*size),
                    _ => None,
                });
            assert_eq!(requested, expected);
            output.textures_delta.clear();
        }
    }

    #[test]
    fn global_zoom_does_not_enlarge_an_already_valid_native_window() {
        let ctx = egui::Context::default();
        let mut previous_zoom = 1.0;
        for (requested, size, next_zoom) in [
            (2.0, Vec2::new(1240.0, 800.0), Some(2.0)),
            (2.0, Vec2::new(1240.0, 800.0), None),
            (2.0, Vec2::new(620.0, 400.0), None),
            (1.0, Vec2::new(620.0, 400.0), Some(1.0)),
            (1.0, Vec2::new(620.0, 400.0), None),
            (1.0, Vec2::new(1240.0, 800.0), None),
        ] {
            let mut raw = egui::RawInput::default();
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .inner_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size));
            let mut output = ctx.run_ui(raw, |_| {
                enforce_stable_minimum_window_size(&ctx, &mut previous_zoom, requested);
                if let Some(zoom) = next_zoom {
                    ctx.set_zoom_factor(zoom);
                }
            });
            assert!(
                !output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .iter()
                    .any(|command| matches!(command, egui::ViewportCommand::InnerSize(_)))
            );
            output.textures_delta.clear();
        }
    }

    #[test]
    fn header_controls_stay_right_aligned_at_supported_widths() {
        for width in [880.0, 1000.0, 1240.0, 1600.0] {
            let ctx = egui::Context::default();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 900.0),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                egui::Panel::top("test-header")
                    .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(18, 14)))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("[t_]").monospace().size(23.0));
                            let title = ui.heading("Tomas Commander");
                            let right = ui.max_rect().right();
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let commands =
                                        action_button(ui, "Commands", "Ctrl Shift P", true);
                                    let theme = action_button(ui, "Light mode", "", true);
                                    let accent = action_button(ui, "Accent: Green", "", true);
                                    assert!(commands.rect.right() <= right);
                                    assert!(theme.rect.right() <= commands.rect.left());
                                    assert!(accent.rect.right() <= theme.rect.left());
                                    assert!(accent.rect.left() > title.rect.right());
                                    assert!((commands.rect.top() - theme.rect.top()).abs() < 1.0);
                                    assert!((theme.rect.top() - accent.rect.top()).abs() < 1.0);
                                },
                            );
                        });
                    });
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn action_columns_share_a_baseline_and_do_not_wrap_labels() {
        for width in [400.0, 880.0, 1000.0, 1240.0, 1600.0] {
            let ctx = egui::Context::default();
            ctx.style_mut_of(egui::Theme::Dark, |style| {
                style.spacing.item_spacing = Vec2::new(8.0, 6.0);
                style.spacing.button_padding = Vec2::new(10.0, 5.0);
                style
                    .text_styles
                    .insert(egui::TextStyle::Button, egui::FontId::proportional(12.0));
            });
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 900.0),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                let mut previous: Option<egui::Rect> = None;
                ui.horizontal_wrapped(|ui| {
                    let bounds = ui.max_rect();
                    for (label, hint) in [
                        ("Copy", "Ctrl Shift C"),
                        ("Move", "Ctrl Shift M"),
                        ("Delete", "Ctrl Shift D"),
                        ("Refresh", "Ctrl R"),
                        ("Search", "Ctrl Shift F"),
                        ("Run VS Code", ""),
                        ("Run GitHub Copilot CLI", ""),
                        ("Run GitHub Copilot App", ""),
                    ] {
                        let response = action_button(ui, label, hint, true);
                        assert!(
                            response.rect.left() >= bounds.left()
                                && response.rect.right() <= bounds.right(),
                            "{width}: {label} outside strip: {:?} in {:?}",
                            response.rect,
                            bounds
                        );
                        if let Some(previous) = previous {
                            if response.rect.left() < previous.right() {
                                assert!(
                                    response.rect.top() >= previous.bottom() + 12.0,
                                    "{width}: {label} overlaps the previous row's hints"
                                );
                            } else {
                                assert!(
                                    (response.rect.top() - previous.top()).abs() < 1.0,
                                    "{width}: {label} has a different baseline"
                                );
                            }
                        }
                        assert!(
                            response.rect.height() <= 28.0,
                            "{label} unexpectedly wrapped"
                        );
                        previous = Some(response.rect);
                    }
                });
            });
            output.textures_delta.clear();
        }
    }
}
