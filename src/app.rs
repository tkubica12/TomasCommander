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
};

const ROW_HEIGHT: f32 = 29.0;
const SCROLL_GUTTER: f32 = 12.0;

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
        let age_width = 48.0;
        let size_width = 68.0;
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
            path_text: path.display().to_string(),
            path,
            entries: Vec::new(),
            visible: Vec::new(),
            filter: String::new(),
            sort: Sort::Name,
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
        if !self
            .visible
            .iter()
            .any(|index| Some(&self.entries[*index].path) == self.focused.as_ref())
        {
            self.focused = self
                .visible
                .first()
                .map(|index| self.entries[*index].path.clone());
            self.anchor = 0;
            self.scroll_to_focus = true;
        }
        self.anchor = self.anchor.min(self.visible.len().saturating_sub(1));
    }

    fn focus_position(&self) -> usize {
        self.visible
            .iter()
            .position(|index| Some(&self.entries[*index].path) == self.focused.as_ref())
            .unwrap_or(0)
    }

    fn targets(&self) -> Vec<PathBuf> {
        if self.selected.is_empty() {
            self.focused.iter().cloned().collect()
        } else {
            self.selected.iter().cloned().collect()
        }
    }
}

struct ScanRequest {
    path: PathBuf,
    generation: u64,
}
struct ScanResult {
    pane: usize,
    generation: u64,
    listing: FileResult<Listing>,
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
}

enum WorkResult {
    Prepared(FileResult<Plan>),
    Executed(Outcome),
    Saved(FileResult<()>),
    Code(FileResult<()>),
    Open(FileResult<()>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Rail {
    Favorites,
    Activity,
}

#[derive(Clone, Copy)]
enum Command {
    Copy,
    Move,
    Recycle,
    Refresh,
    Favorites,
    Pin,
    Sort,
    Filter,
    Balance,
    Code,
    Activity,
    Theme,
    Accent,
}

const COMMANDS: [(Command, &str, &str); 13] = [
    (Command::Copy, "Copy to opposite pane", "Ctrl Shift C"),
    (Command::Move, "Move to opposite pane", "Ctrl Shift M"),
    (Command::Recycle, "Recycle selected items", "Delete"),
    (Command::Refresh, "Refresh folder", "Ctrl R"),
    (Command::Favorites, "Show favorite locations", ""),
    (Command::Pin, "Pin current folder", ""),
    (Command::Sort, "Cycle sorting", "Ctrl Shift S"),
    (Command::Filter, "Filter active folder", "Ctrl F"),
    (Command::Balance, "Balance file panes", ""),
    (Command::Code, "Open folder in VS Code", ""),
    (
        Command::Activity,
        "Show activity and exact operation results",
        "",
    ),
    (Command::Theme, "Toggle light and dark theme", ""),
    (Command::Accent, "Cycle accent color", ""),
];

pub struct Ledger {
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
                    if results
                        .send(ScanResult {
                            pane,
                            generation: request.generation,
                            listing,
                        })
                        .is_err()
                    {
                        break;
                    }
                    context.request_repaint();
                }
            });
            scans.push(ScanWorker {
                sender,
                generation,
                pending: None,
            });
        }
        let (work, jobs) = mpsc::sync_channel::<Work>(4);
        let (results, work_results) = mpsc::sync_channel(8);
        let cancellation = Arc::new(AtomicBool::new(false));
        let cancel = cancellation.clone();
        let worker_scope = scope.clone();
        let context = creation.egui_ctx.clone();
        thread::spawn(move || {
            let mut executed_ids = BTreeSet::new();
            while let Ok(job) = jobs.recv() {
                let result = match job {
                    Work::Prepare {
                        id,
                        operation,
                        sources,
                        destination,
                    } => WorkResult::Prepared(files::plan_cancellable(
                        &worker_scope,
                        id,
                        operation,
                        &sources,
                        Some(&destination),
                        &cancel,
                    )),
                    Work::Execute(plan) => {
                        if !executed_ids.insert(plan.id) {
                            WorkResult::Executed(Outcome { id: plan.id, completed: Vec::new(), created: Vec::new(),
                                error: Some("This operation ID has already executed; inspect state before retrying.".into()), cancelled: false })
                        } else {
                            WorkResult::Executed(files::execute(
                                &worker_scope,
                                &plan,
                                true,
                                &cancel,
                            ))
                        }
                    }
                    Work::Save(preferences) => WorkResult::Saved(preferences.save(&settings)),
                    Work::Code(path) => WorkResult::Code(platform::open_vscode(&path)),
                    Work::Open(path) => WorkResult::Open(
                        worker_scope
                            .resolve(&path)
                            .and_then(|path| platform::open_file(&path)),
                    ),
                };
                if results.send(result).is_err() {
                    break;
                }
                context.request_repaint();
            }
        });
        let mut app = Self {
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
                            path.display(),
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
        pane.path_text = path.display().to_string();
        pane.path = path;
        if !refresh {
            pane.filter.clear();
            pane.entries.clear();
            pane.visible.clear();
            pane.selected.clear();
            pane.focused = None;
            pane.scroll_offset = 0.0;
            pane.scroll_to_focus = true;
        }
        pane.busy = true;
        pane.warning = None;
    }

    fn dispatch(&mut self, command: Command) {
        self.palette = false;
        match command {
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
                pane.sort = match pane.sort {
                    Sort::Name => Sort::Size,
                    Sort::Size => Sort::Modified,
                    Sort::Modified => Sort::Name,
                };
                files::sort_entries(&mut pane.entries, pane.sort);
                pane.rebuild();
            }
            Command::Filter => self.editor_focus = Some(Id::new(("filter", self.active))),
            Command::Balance => self.balance_requested = true,
            Command::Code => {
                if let Err(error) = self
                    .work
                    .try_send(Work::Code(self.panes[self.active].path.clone()))
                {
                    self.fail(format!("Could not queue VS Code launch: {error}"));
                }
            }
        }
    }

    fn open_focused(&mut self) {
        let pane = &self.panes[self.active];
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
        if self.plan.is_some() || self.error.is_some() {
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
        let editing = ctx.memory(|memory| {
            (0..2).any(|pane| {
                memory.has_focus(Id::new(("path", pane)))
                    || memory.has_focus(Id::new(("filter", pane)))
            })
        });
        if editing {
            return;
        }
        if let Some(index) = self.rail_index {
            let mut next = index;
            if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                next = next.saturating_sub(1);
            }
            if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                next += 1;
            }
            next = next.min(self.preferences.favorites.len().saturating_sub(1));
            self.rail_index = Some(next);
            if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter)) {
                let path = self.preferences.favorites.get(next).cloned();
                self.rail_index = None;
                if let Some(path) = path {
                    self.navigate(self.active, path);
                }
            }
            if ctx.input_mut(|input| {
                input.consume_key(Modifiers::NONE, Key::Escape)
                    || input.consume_key(Modifiers::NONE, Key::Tab)
            }) {
                self.rail_index = None;
            }
            return;
        }
        for (key, command) in [
            (Key::C, Command::Copy),
            (Key::M, Command::Move),
            (Key::S, Command::Sort),
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
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Delete)) {
            self.dispatch(Command::Recycle);
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
            if let Some(parent) = self.panes[self.active].path.parent() {
                self.navigate(self.active, parent.to_owned());
            }
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
            let shift = ctx.input(|input| input.modifiers.shift);
            let modifiers = if shift {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            };
            if !ctx.input_mut(|input| input.consume_key(modifiers, key)) || pane.visible.is_empty()
            {
                continue;
            }
            // egui resolves focus traversal from raw input before consume_key.
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            let previous = pane.focus_position();
            let position = match key {
                Key::ArrowUp => previous.saturating_sub(1),
                Key::ArrowDown => (previous + 1).min(pane.visible.len() - 1),
                Key::Home => 0,
                _ => pane.visible.len() - 1,
            };
            pane.focused = Some(pane.entries[pane.visible[position]].path.clone());
            if shift {
                pane.selected = (pane.anchor.min(position)..=pane.anchor.max(position))
                    .filter_map(|position| pane.visible.get(position))
                    .map(|index| pane.entries[*index].path.clone())
                    .collect();
            } else {
                pane.anchor = position;
            }
            pane.scroll_to_focus = true;
            self.file_focus_requested = true;
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
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
        while let Ok(result) = self.scan_results.try_recv() {
            if self.scans[result.pane].generation.load(Ordering::Relaxed) != result.generation {
                continue;
            }
            let pane = &mut self.panes[result.pane];
            pane.busy = false;
            match result.listing {
                Ok(mut listing) => {
                    files::sort_entries(&mut listing.entries, pane.sort);
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
                }
                Err(error) => {
                    pane.warning = Some(error.clone());
                    self.fail(error);
                }
            }
        }
        while let Ok(result) = self.work_results.try_recv() {
            match result {
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
                        "Operation #{}: {} completed nodes, {} created paths{}",
                        outcome.id,
                        outcome.completed.len(),
                        outcome.created.len(),
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
                    Ok(()) => self.log("Windows associated-application opening requested."),
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
                            .desired_width((ui.available_width() - 105.0).max(40.0)),
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
                    let previous = pane.sort;
                    let combo = egui::ComboBox::from_id_salt(("sort", index))
                        .selected_text(match pane.sort {
                            Sort::Name => "Name",
                            Sort::Size => "Size",
                            Sort::Modified => "Modified",
                        })
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut pane.sort, Sort::Name, "Name");
                            ui.selectable_value(&mut pane.sort, Sort::Size, "Size");
                            ui.selectable_value(&mut pane.sort, Sort::Modified, "Modified");
                        });
                    if self.editor_focus == Some(Id::new(("sort", index))) {
                        combo.response.request_focus();
                        self.editor_focus = None;
                    }
                    if previous != pane.sort {
                        files::sort_entries(&mut pane.entries, pane.sort);
                        pane.rebuild();
                    }
                });
            });
        let (head, head_response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), egui::Sense::hover());
        head_response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                ui.is_enabled(),
                "NAME / SIZE / AGE",
            )
        });
        ui.painter().rect_filled(head, 0.0, stripe);
        ui.painter().hline(head.x_range(), head.top(), border);
        ui.painter().hline(head.x_range(), head.bottom(), border);
        let mut head_content = head;
        head_content.max.x -= SCROLL_GUTTER;
        let columns = RowColumns::new(head_content);
        for (rect, text, align) in [
            (columns.name, "NAME", egui::Align2::LEFT_CENTER),
            (columns.size, "SIZE", egui::Align2::RIGHT_CENTER),
            (columns.age, "AGE", egui::Align2::RIGHT_CENTER),
        ] {
            ui.painter().text(
                if align == egui::Align2::LEFT_CENTER {
                    rect.left_center()
                } else {
                    rect.right_center()
                },
                align,
                text,
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
                scroll.show_rows(ui, ROW_HEIGHT, pane.visible.len(), |ui, range| {
                    for position in range {
                        let entry_index = pane.visible[position];
                        let entry = &pane.entries[entry_index];
                        let path = entry.path.clone();
                        let focused = pane.focused.as_ref() == Some(&path);
                        let selected = pane.selected.contains(&path);
                        let name = entry.name.clone();
                        let icon = if entry.link {
                            "[link]"
                        } else if entry.directory {
                            "[/]"
                        } else {
                            "[.]"
                        };
                        let label = format!("{icon} {name}");
                        let size = if entry.directory {
                            "<DIR>".into()
                        } else {
                            format_size(entry.size)
                        };
                        let age = entry
                            .modified
                            .and_then(|time| SystemTime::now().duration_since(time).ok())
                            .map(|age| format!("{}d", age.as_secs() / 86400))
                            .unwrap_or_else(|| "unknown".into());
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
                                .on_hover_text(path.display().to_string());
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
                                    "{size}, age {age}{}",
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
                            let columns = RowColumns::new(rect);
                            ui.painter().text(
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
                            if response.clicked() {
                                self.active = index;
                                self.rail_index = None;
                                pane.focused = Some(path.clone());
                                let modifiers = ui.input(|input| input.modifiers);
                                if modifiers.ctrl {
                                    if !pane.selected.remove(&path) {
                                        pane.selected.insert(path.clone());
                                    }
                                } else if modifiers.shift {
                                    pane.selected = (pane.anchor.min(position)
                                        ..=pane.anchor.max(position))
                                        .filter_map(|position| pane.visible.get(position))
                                        .map(|index| pane.entries[*index].path.clone())
                                        .collect();
                                } else {
                                    pane.anchor = position;
                                }
                            }
                            if response.double_clicked() {
                                self.active = index;
                                pane.focused = Some(path.clone());
                                open = true;
                            }
                        });
                    }
                    if pane.visible.is_empty() && !pane.busy {
                        ui.label(if pane.filter.is_empty() {
                            "Empty folder"
                        } else {
                            "No matching names"
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
            if let Some(path) = self.panes[index].path.parent() {
                self.navigate(index, path.to_owned());
            }
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
                ui.label("Real local commands only. Foundry/MCP and voice are not connected.");
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
            let modal=egui::Modal::new(Id::new(("approval",plan.id))).show(ctx,|ui| {
                    ui.set_width(620.0);
                    ui.heading(format!("{} / approval required",plan.operation.name()));
                    ui.label("This is a REAL filesystem operation. Review the exact targets.");
                    if let Some(destination)=&plan.destination {ui.label(format!("TO {}",destination.display()));}
                    else {ui.label("TO Windows Recycle Bin / no permanent-delete fallback");}
                    egui::ScrollArea::vertical().max_height(250.0).show(ui,|ui| {
                        for source in &plan.sources {ui.label(RichText::new(source.display().to_string()).monospace());}
                    });
                    ui.label("Conflicts are refused. Cancelling mid-copy may leave explicitly reported partial destinations.");
                    ui.horizontal(|ui| {
                        let cancel_button=ui.button("Cancel / Escape");
                        if self.approval_focus {cancel_button.request_focus();self.approval_focus=false;}
                        cancel=cancel_button.clicked();
                        approve=ui.button(format!("Approve {} / Ctrl Enter",plan.operation.name())).clicked();
                    });
                    approve |=ui.input_mut(|input|input.consume_key(Modifiers::CTRL,Key::Enter));
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
        self.poll(ctx);
        self.shortcuts(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.appearance_dirty {
            self.appearance_dirty = false;
            self.apply_theme(&ctx);
        }
        if self.palette || self.plan.is_some() || self.error.is_some() || self.close_after_save {
            ui.disable();
        }
        if self.operation_busy && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.fail("An operation is still running. Cancel work or wait for its exact outcome before closing.");
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
                        ui.heading("TomasCommander");
                        ui.label(
                            RichText::new("A little more command. A little less friction.")
                                .small()
                                .weak(),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Commands  Ctrl Shift P").clicked() {
                            self.palette = true;
                            self.palette_focus = true;
                            self.palette_query.clear();
                        }
                        if ui
                            .button(if self.preferences.dark {
                                "Light mode"
                            } else {
                                "Dark mode"
                            })
                            .clicked()
                        {
                            self.dispatch(Command::Theme);
                        }
                        if ui
                            .button(format!("Accent: {}", ACCENTS[self.preferences.accent].0))
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
            ui.horizontal(|ui| {
                for (command, label) in [
                    (Command::Copy, "Copy  Ctrl Shift C"),
                    (Command::Move, "Move  Ctrl Shift M"),
                    (Command::Recycle, "Recycle  Delete"),
                    (Command::Refresh, "Refresh"),
                    (Command::Code, "VS Code"),
                ] {
                    if ui
                        .add_enabled(
                            !self.operation_busy && self.plan.is_none(),
                            egui::Button::new(label),
                        )
                        .clicked()
                    {
                        self.dispatch(command);
                    }
                }
                if ui.available_width() > 220.0 {
                    ui.label(
                        RichText::new("Tab pane / Space select / Enter folder")
                            .small()
                            .weak(),
                    );
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
                    ui.selectable_value(&mut self.rail, Rail::Favorites, "Places");
                    ui.selectable_value(&mut self.rail, Rail::Activity, "Activity");
                });
                ui.separator();
                match self.rail {
                    Rail::Favorites => {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("PINNED LOCATIONS")
                                .monospace()
                                .size(10.0)
                                .weak(),
                        );
                        ui.add_space(6.0);
                        let mut target = None;
                        for (index, path) in self.preferences.favorites.iter().enumerate() {
                            let name = path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.display().to_string());
                            if ui
                                .add(
                                    egui::Button::new(format!("/ {name}"))
                                        .right_text("")
                                        .min_size(Vec2::new(ui.available_width(), 30.0))
                                        .truncate()
                                        .frame(false)
                                        .selected(self.rail_index == Some(index)),
                                )
                                .on_hover_text(path.display().to_string())
                                .clicked()
                            {
                                target = Some(path.clone());
                            }
                        }
                        if let Some(target) = target {
                            self.rail_index = None;
                            self.navigate(self.active, target);
                        }

                        ui.add_space(25.0);
                        if ui.button("+ Pin active folder").clicked() {
                            self.dispatch(Command::Pin);
                        }
                        ui.label(
                            RichText::new("Places for navigation.\nActivity for exact outcomes.")
                                .small()
                                .weak(),
                        );
                    }
                    Rail::Activity => {
                        if let Some(outcome) = &self.last_outcome {
                            ui.label(format!("Exact outcome #{}", outcome.id));
                            let count = outcome.created.len() + outcome.completed.len();
                            egui::ScrollArea::vertical()
                                .id_salt("outcome-paths")
                                .max_height(200.0)
                                .show_rows(ui, ROW_HEIGHT, count, |ui, range| {
                                    for index in range {
                                        let (label, path) = if index < outcome.created.len() {
                                            ("CREATED", &outcome.created[index])
                                        } else {
                                            (
                                                "COMPLETED",
                                                &outcome.completed[index - outcome.created.len()],
                                            )
                                        };
                                        ui.add(
                                            egui::Label::new(format!("{label} {}", path.display()))
                                                .truncate(),
                                        )
                                        .on_hover_text(path.display().to_string());
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
    }
}

// egui 0.36 text editors expose ValuePattern but do not process SetValue.
fn accessible_text_value(ctx: &egui::Context, id: Id, text: &mut String) -> bool {
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
        assert_eq!(pane.anchor, 0);
        assert_eq!(pane.targets(), vec![PathBuf::from("Beta")]);
        pane.filter = "missing".into();
        pane.rebuild();
        assert!(pane.focused.is_none());
        assert!(pane.visible.is_empty());
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
        assert_eq!(pane.focus_position(), 1);
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
}
