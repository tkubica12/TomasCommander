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
    files::{self, Entry, FileResult, Listing, Operation, Outcome, Plan, Scope, Sort},
    platform,
    preferences::Preferences,
    search,
};

const ROW_HEIGHT: f32 = 29.0;
const SCROLL_GUTTER: f32 = 12.0;
pub const MIN_WINDOW_SIZE: [f32; 2] = [880.0, 560.0];

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

    fn row_path(&self, position: usize) -> Option<PathBuf> {
        position
            .checked_sub(1)
            .and_then(|position| self.visible.get(position))
            .map(|index| self.entries[*index].path.clone())
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

enum Work {
    Prepare {
        id: u64,
        operation: Operation,
        sources: Vec<PathBuf>,
        destination: PathBuf,
    },
    Execute(Plan),
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

enum WorkResult {
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Rail {
    Favorites,
    Recent,
    Activity,
}

#[derive(Clone, Copy)]
enum Command {
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
    Balance,
    Code,
    CopilotCli,
    CopilotApp,
    Activity,
    Theme,
    Accent,
    Foundry,
}

const COMMANDS: [(Command, &str, &str); 18] = [
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
];

pub struct Ledger {
    ai: crate::ai_ui::AiPanel,
    panes: [Pane; 2],
    active: usize,
    scope: Scope,
    preferences: Preferences,
    preferences_writable: bool,
    save_pending: bool,
    saves_in_flight: usize,
    close_after_save: bool,
    scans: Vec<ScanWorker>,
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
}

impl Ledger {
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
        let mut scans = Vec::new();
        for pane in 0..2 {
            let (sender, receiver) = mpsc::sync_channel::<ScanRequest>(1);
            let generation = Arc::new(AtomicU64::new(0));
            let current = generation.clone();
            let results = scan_tx.clone();
            let scope = scope.clone();
            let context = creation.egui_ctx.clone();
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
                        let result = files::directory_stats(&scope, &path, stale);
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
                let cancelled = || generation.load(Ordering::Relaxed) != request.generation;
                if cancelled() {
                    continue;
                }
                let result =
                    search::discover(&search_scope, &request.root, &request.query, cancelled);
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
            ai: crate::ai_ui::AiPanel::new(),
            panes: [Pane::new(initial.clone()), Pane::new(initial.clone())],
            active: 0,
            scope,
            preferences,
            preferences_writable,
            save_pending: false,
            saves_in_flight: 0,
            close_after_save: false,
            scans,
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

    fn fail(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.log(format!("ERROR / {message}"));
        self.error = Some(message);
    }

    fn navigate(&mut self, index: usize, path: PathBuf) {
        let worker = &mut self.scans[index];
        let generation = worker.generation.fetch_add(1, Ordering::Relaxed) + 1;
        worker.pending = Some(ScanRequest {
            path: path.clone(),
            generation,
        });
        let pane = &mut self.panes[index];
        let refresh = pane.path == path;
        let child = if pane.path.parent() == Some(path.as_path()) {
            Some(pane.path.clone())
        } else {
            None
        };
        pane.path_text = files::display_path(&path);
        pane.path = path;
        if !refresh {
            pane.filter.clear();
            pane.entries.clear();
            pane.visible.clear();
            pane.selected.clear();
            pane.focused = child;
            pane.scroll_offset = 0.0;
            pane.scroll_to_focus = true;
        }

        pane.busy = true;
        pane.warning = None;
        self.file_focus_requested = true;
    }

    fn parent(&mut self, index: usize) {
        let path = &self.panes[index].path;
        if self.scope.root() == Some(path.as_path()) || path.parent().is_none() {
            self.log("Parent unavailable at filesystem root or fixture boundary.");
        } else if let Some(parent) = path.parent() {
            self.navigate(index, parent.to_owned());
        }
    }

    fn dispatch(&mut self, command: Command) {
        self.palette = false;
        match command {
            Command::Foundry => self.ai.open = true,
            Command::Copy | Command::Move | Command::Recycle => {
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
            Command::Refresh => self.navigate(self.active, self.panes[self.active].path.clone()),
            Command::Favorites => {
                self.rail = Rail::Favorites;
                self.rail_index = Some(0);
            }
            Command::Recent => {
                self.rail = Rail::Recent;
                self.rail_index = Some(0);
            }
            Command::Activity => {
                self.rail = Rail::Activity;
                self.rail_index = None;
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
            Command::Pin => {
                let path = self.panes[self.active].path.clone();
                if !self.preferences.favorites.contains(&path) {
                    self.preferences.favorites.push(path);
                    self.save_pending = true;
                }
                self.log("Current folder pinned.");
            }
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
            Command::Search => {
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
                } else {
                    if let Err(error) = self.work.try_send(Work::Open(path.clone())) {
                        self.fail(format!("Could not queue file opening: {error}"));
                    }
                }
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.ai.open {
            return;
        }
        let editing = ctx.memory(|memory| {
            (0..2).any(|pane| {
                memory.has_focus(Id::new(("path", pane)))
                    || memory.has_focus(Id::new(("filter", pane)))
            })
        });
        let copy = consume_file_copy(
            ctx,
            &mut self.copy_modifiers,
            !editing
                && self.rail_index.is_none()
                && !self.palette
                && self.plan.is_none()
                && self.error.is_none()
                && self.search_dialog.is_none(),
        );
        if self.plan.is_some() || self.error.is_some() || self.search_dialog.is_some() {
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
        if editing {
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
                WorkResult::Recovery(error) => self.fail(error),
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
                WorkResult::Executed(outcome) => {
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
                    for index in 0..2 {
                        self.navigate(index, self.panes[index].path.clone());
                    }
                    self.last_outcome = Some(outcome);
                }
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
        let mut open = false;
        ui.spacing_mut().item_spacing.y = 0.0;
        let pane = &mut self.panes[index];
        let editable = self.plan.is_none() && self.error.is_none() && !self.palette;
        if editable {
            accessible_text_value(ui.ctx(), Id::new(("path", index)), &mut pane.path_text);
            if accessible_text_value(ui.ctx(), Id::new(("filter", index)), &mut pane.filter) {
                self.active = index;
                pane.rebuild();
            }
        }
        let heading = egui::Frame::new()
            .fill(stripe)
            .inner_margin(egui::Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut pane.path_text)
                            .id(Id::new(("path", index)))
                            .font(egui::TextStyle::Monospace)
                            .margin(Vec2::new(7.0, 6.0))
                            .desired_width((ui.available_width() - 68.0).max(40.0)),
                    );
                    if response.has_focus() {
                        self.active = index;
                        self.rail_index = None;
                    }
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.add_action(egui::accesskit::Action::SetValue)
                    });
                    go = ui.button("Go").clicked()
                        || (response.lost_focus()
                            && ui.input(|input| input.key_pressed(Key::Enter)));
                });
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut pane.filter)
                            .id(Id::new(("filter", index)))
                            .hint_text("Filter this folder...")
                            .font(egui::TextStyle::Monospace)
                            .margin(Vec2::new(7.0, 6.0))
                            .desired_width(f32::INFINITY),
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
                });
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
        if pane.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading actual folder...");
            });
        }
        if let Some(warning) = &pane.warning {
            ui.label(RichText::new("PARTIAL / ERROR").color(accent));
            ui.label(warning);
        }
        let height = (ui.available_height() - 32.0).max(50.0);
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
                            let response = ui
                                .interact(row, ui.id().with("file"), egui::Sense::click())
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
                            ui.painter().with_clip_rect(rect).text(
                                egui::pos2(rect.left() + 32.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                if entry.link { "@" } else { icon },
                                egui::FontId::monospace(10.0),
                                if entry.directory { accent } else { muted },
                            );
                            ui.scope_builder(
                                egui::UiBuilder::new()
                                    .max_rect(columns.name)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&name).monospace().size(12.0),
                                        )
                                        .truncate(),
                                    );
                                },
                            );
                            for (rect, text) in [(columns.size, &size), (columns.age, &age)] {
                                ui.painter().text(
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
                                } else if modifiers.ctrl || response.secondary_clicked() {
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
                                self.active = index;
                                pane.focused = if navigation { None } else { Some(path.clone()) };
                                open = true;
                            }
                        });
                    }
                })
            })
            .inner;
        pane.scroll_offset = output.state.offset.y;
        ui.painter()
            .hline(ui.max_rect().x_range(), ui.cursor().top(), border);
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(12, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(if active { "READY / FOCUSED" } else { "READY" })
                                .monospace()
                                .size(10.0)
                                .color(if active { accent } else { muted }),
                        );
                    });
                })
            });
        if go {
            self.active = index;
            self.navigate(index, PathBuf::from(self.panes[index].path_text.trim()));
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

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(dialog) = &mut self.search_dialog {
            let mut start = false;
            let mut cancel = false;
            let mut close = false;
            let mut activate = None;
            let modal = egui::Modal::new(Id::new("filename-search")).show(ctx, |ui| {
                ui.set_width(680.0);
                ui.heading("Search files");
                ui.label(format!("In: {}", files::display_path(&dialog.root)))
                    .on_hover_text(format!("Includes subfolders; matches filenames without case sensitivity. Links are skipped.\nLimits: {} examined items, {} results.", search::MAX_EXAMINED, search::MAX_RESULTS));
                let query_id = Id::new("search-query");
                let query_focused_before = ui.memory(|memory| memory.has_focus(query_id));
                let enter = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
                let down = dialog.results.is_some()
                    && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown));
                let up = dialog.results.is_some()
                    && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp));
                accessible_text_value(ui.ctx(), query_id, &mut dialog.query);
                ui.horizontal(|ui| {
                    let response = ui.add(egui::TextEdit::singleline(&mut dialog.query)
                        .id(query_id).hint_text("Filename contains...").desired_width(480.0));
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.set_label("Recursive filename query");
                        node.add_action(egui::accesskit::Action::SetValue);
                    });
                    if dialog.focus_query {
                        response.request_focus();
                        dialog.focus_query = false;
                    }
                    start = ui.add_enabled(!dialog.busy, egui::Button::new("Search")).clicked();
                });
                if query_focused_before && enter {
                    start = !dialog.busy;
                }
                if dialog.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Searching actual filenames...");
                        cancel = ui.button("Cancel search").clicked();
                    });
                }
                if let Some(error) = &dialog.error { ui.label(RichText::new(error).color(Color32::LIGHT_RED)); }
                if let Some(results) = &dialog.results {
                    ui.label(format!("{} results{}", results.matches.len(),
                        if results.truncated { " / limit reached" } else { "" }))
                        .on_hover_text(format!("{} items examined; {} links skipped", results.examined, results.skipped_links));
                    if results.matches.is_empty() { ui.label("No results"); }
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
                            let label = format!("{} {}", if result.entry.directory { "[/]" } else { "[.]" }, result.relative.display());
                            let response = ui.add(egui::Button::new(label).selected(index == dialog.index)
                                .min_size(Vec2::new(ui.available_width(), ROW_HEIGHT)).truncate());
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
                self.search_generation.fetch_add(1, Ordering::Relaxed);
                dialog.busy = false;
                dialog.results = None;
                dialog.error = Some("Search cancelled.".into());
            }
            if start {
                let generation = self.search_generation.fetch_add(1, Ordering::Relaxed) + 1;
                match self.search_jobs.try_send(SearchRequest {
                    generation,
                    root: dialog.root.clone(),
                    query: dialog.query.clone(),
                }) {
                    Ok(()) => {
                        dialog.busy = true;
                        dialog.index = 0;
                        dialog.scroll_offset = 0.0;
                        dialog.results = None;
                        dialog.error = None;
                    }
                    Err(error) => dialog.error = Some(format!("Search was not queued: {error}")),
                }
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
                ui.label("Real local commands and bounded Foundry text. MCP and voice are not connected.");
                dismiss = ui.button("Close / Escape").clicked();
            });
            dismiss |= modal.should_close();
            if dismiss {
                self.palette = false;
            }
            if let Some(command) = chosen {
                self.dispatch(command);
            }
        }
        if let Some(plan) = &self.plan {
            let mut approve = false;
            let mut cancel = false;
            let modal = egui::Modal::new(Id::new(("approval", plan.id()))).show(ctx, |ui| {
                ui.set_width(620.0);
                ui.spacing_mut().item_spacing.y = 10.0;
                let count = plan.sources().len();
                ui.heading(format!("{} {count} {}", plan.operation().name(), if count == 1 { "item" } else { "items" }));
                egui::Frame::group(ui.style()).inner_margin(12.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Destination").strong());
                    let destination = plan.destination().map(files::display_path)
                        .unwrap_or_else(|| "Windows Recycle Bin (never permanently delete)".into());
                    ui.add(egui::Label::new(RichText::new(destination).monospace().size(12.0)).wrap());
                });
                ui.label(RichText::new("Sources").strong());
                egui::ScrollArea::vertical().id_salt("approval-sources").max_height(220.0).show(ui, |ui| {
                    for source in plan.sources() {
                        ui.add(egui::Label::new(RichText::new(files::display_path(source)).monospace().size(12.0)).wrap());
                    }
                });
                ui.separator();
                ui.label(RichText::new("Existing destinations are never overwritten. Cancellation can leave partial copies; Activity lists their exact paths.").small().weak());
                ui.horizontal(|ui| {
                    let cancel_button = action_button(ui, "Cancel", "Escape", true);
                    if self.approval_focus {
                        cancel_button.request_focus();
                        self.approval_focus = false;
                    }
                    cancel = cancel_button.clicked();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let response = action_button(ui, plan.operation().name(), "Ctrl Enter", true);
                        ui.ctx().accesskit_node_builder(response.id, |node| node.set_label(format!("Confirm {}", plan.operation().name())));
                        approve = response.clicked();
                    });
                });
                approve |= ui.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::Enter));
            });
            cancel |= modal.should_close();
            if cancel {
                self.plan = None;
                self.log("Plan cancelled. No filesystem changes.");
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
        self.ai.poll();
        self.poll(ctx);
        self.shortcuts(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        enforce_minimum_window_size(&ctx);
        if self.appearance_dirty {
            self.appearance_dirty = false;
            self.apply_theme(&ctx);
        }
        if self.palette
            || self.plan.is_some()
            || self.error.is_some()
            || self.search_dialog.is_some()
            || self.close_after_save
            || self.ai.open
        {
            ui.disable();
        }
        if self.operation_busy && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.fail("An operation is still running. Cancel work or wait for its exact outcome before closing.");
        } else if self.ai.busy() && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.ai.cancel();
            self.ai.open = true;
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
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("[t_]")
                            .monospace()
                            .size(23.0)
                            .color(self.accent()),
                    );
                    ui.vertical(|ui| {
                        ui.heading("Tomas Commander");
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Foundry").clicked() {
                            self.dispatch(Command::Foundry);
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
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.status).monospace().small());
                if self.operation_busy && ui.button("Cancel work").clicked() {
                    self.cancellation.store(true, Ordering::Relaxed);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(if self.scope.root().is_some() {
                            "FIXTURE BOUNDARY ENFORCED"
                        } else {
                            "REAL FILESYSTEM / APPROVAL REQUIRED"
                        })
                        .monospace()
                        .small()
                        .weak(),
                    );
                });
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
                    (Command::Code, "Run VS Code", ""),
                    (Command::CopilotCli, "Run GitHub Copilot CLI", ""),
                    (Command::CopilotApp, "Run GitHub Copilot App", ""),
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
                ui.horizontal(|ui| {
                    ui.spacing_mut().button_padding.x = 4.0;
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (rail, label) in [
                        (Rail::Favorites, "Places"),
                        (Rail::Recent, "Recent"),
                        (Rail::Activity, "Activity"),
                    ] {
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
                                    let response = ui
                                        .add(
                                            egui::Button::new(if self.rail == Rail::Recent {
                                                name
                                            } else {
                                                format!("/ {name}")
                                            })
                                            .right_text("")
                                            .min_size(Vec2::new(ui.available_width(), 30.0))
                                            .truncate()
                                            .frame(false)
                                            .selected(self.rail_index == Some(index)),
                                        )
                                        .on_hover_text(files::display_path(path));
                                    if self.rail_index == Some(index) {
                                        response.request_focus();
                                        response.scroll_to_me(None);
                                    }
                                    if response.clicked() {
                                        target = Some(path.clone());
                                    }
                                }
                            });
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
                    Rail::Activity => {
                        if let Some(outcome) = &self.last_outcome {
                            ui.label(format!("Exact outcome #{}", outcome.id));
                            let count = outcome.created.len()
                                + outcome.completed.len()
                                + outcome.incomplete.len();
                            egui::ScrollArea::vertical()
                                .id_salt("outcome-paths")
                                .max_height(200.0)
                                .show_rows(ui, ROW_HEIGHT, count, |ui, range| {
                                    for index in range {
                                        let (label, path) = if index < outcome.created.len() {
                                            ("CREATED", &outcome.created[index])
                                        } else if index
                                            < outcome.created.len() + outcome.completed.len()
                                        {
                                            (
                                                "COMPLETED",
                                                &outcome.completed[index - outcome.created.len()],
                                            )
                                        } else {
                                            (
                                                "NOT COMPLETED",
                                                &outcome.incomplete[index
                                                    - outcome.created.len()
                                                    - outcome.completed.len()],
                                            )
                                        };
                                        ui.add(
                                            egui::Label::new(format!(
                                                "{label} {}",
                                                files::display_path(path)
                                            ))
                                            .truncate(),
                                        )
                                        .on_hover_text(files::display_path(path));
                                    }
                                });
                            ui.separator();
                        }
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for message in &self.activity {
                                ui.label(RichText::new(message).small());
                                ui.separator();
                            }
                        });
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
        self.left_width = self.left_width.clamp(240.0, (remaining - 250.0).max(240.0));
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
            .size_range(230.0..=(remaining - 230.0).max(230.0))
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
    let minimum = Vec2::from(MIN_WINDOW_SIZE);
    if let Some(size) = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.size()))
        && (size.x < minimum.x || size.y < minimum.y)
    {
        // Windows tiling tools can override the native minimum-size hint.
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.max(minimum)));
    }
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
