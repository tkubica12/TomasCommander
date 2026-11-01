use super::*;

pub(super) struct ScanRequest {
    pub(super) path: PathBuf,
    pub(super) generation: u64,
    /// Reopening a remembered folder (tab activation, Back/Forward, launch): the worker falls
    /// back to the nearest existing ancestor when the folder is missing or unreachable.
    pub(super) fallback: bool,
}
pub(super) struct ScanResult {
    pub(super) pane: usize,
    pub(super) generation: u64,
    pub(super) content: ScanContent,
    /// Time the worker spent listing the folder (zero for aggregates).
    pub(super) elapsed: Duration,
}
pub(super) enum ScanContent {
    /// The listing and, after a fallback, the requested folder that was not available.
    Listing(FileResult<(Listing, Option<PathBuf>)>),
    Aggregate {
        path: PathBuf,
        result: FileResult<files::DirectoryStats>,
    },
}
pub(super) struct ScanWorker {
    pub(super) sender: SyncSender<ScanRequest>,
    pub(super) generation: Arc<AtomicU64>,
    pub(super) pending: Option<ScanRequest>,
    /// Whether the latest request falls back to an existing ancestor.
    pub(super) fallback: bool,
}

impl ScanWorker {
    /// Queues a scan of `path`; results of earlier requests become stale.
    pub(super) fn request(&mut self, path: PathBuf, fallback: bool) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.fallback = fallback;
        self.pending = Some(ScanRequest {
            path,
            generation,
            fallback,
        });
    }

    pub(super) fn current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Relaxed) == generation
    }

    /// Whether reloading `path` in `pane` keeps the fallback: the pane is still loading that
    /// remembered folder (for example when an operation refreshes both panes meanwhile).
    pub(super) fn reload_falls_back(&self, pane: &Pane, path: &std::path::Path) -> bool {
        self.fallback && pane.busy && pane.path == path
    }
}

pub(super) enum Change {
    Save(tomas_commander::changes::Plan),
    Archive(tomas_commander::archive_changes::Plan),
    Name(tomas_commander::naming::Plan),
    Undo(tomas_commander::undo::Plan),
    UndoBatch(tomas_commander::undo::BatchPlan),
}
impl Change {
    pub(super) fn id(&self) -> u64 {
        match self {
            Self::Save(plan) => plan.id,
            Self::Archive(plan) => plan.id,
            Self::Name(plan) => plan.id,
            Self::Undo(plan) => plan.id,
            Self::UndoBatch(plan) => plan.id,
        }
    }
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Save(plan) => plan.label,
            Self::Archive(plan) => plan.label,
            Self::Name(plan) => plan.label(),
            Self::Undo(plan) => plan.label(),
            Self::UndoBatch(plan) => plan.label(),
        }
    }
    /// Review heading, e.g. "Undo Move: 3 items back to D:\x".
    pub(super) fn heading(&self) -> String {
        match self {
            Self::Undo(plan) => plan.heading(),
            Self::UndoBatch(plan) => plan.heading(),
            _ => self.label().to_owned(),
        }
    }
    fn summary(&self) -> String {
        match self {
            Self::Save(plan) => plan.summary(),
            Self::Archive(plan) => plan.summary(),
            Self::Name(plan) => plan.summary(),
            Self::Undo(plan) => format!(
                "{}\nUndoes recorded operation {}. Each step rechecks the current state; execution stops at the first change, leaving later steps undone.",
                plan.heading(),
                plan.undoes()
            ),
            Self::UndoBatch(plan) => format!(
                "{}\nUndoes the operations of batch {} newest first. Each operation's inverse is prepared and rechecked just before it runs; the batch stops at the first change, leaving earlier operations as they are.",
                plan.heading(),
                plan.batch()
            ),
        }
    }
    pub(super) fn confirm_label(&self) -> &'static str {
        match self {
            Self::Save(_) => "Save",
            Self::Archive(_) => "Confirm",
            Self::Name(_) => "Rename",
            Self::Undo(_) => "Undo",
            Self::UndoBatch(_) => "Undo batch",
        }
    }

    pub(super) fn show_summary(&self, ui: &mut egui::Ui) {
        match self {
            Self::Save(plan) => {
                confirmation_path(ui, "File", plan.source());
                ui.label("Replace this file with your edited contents.");
            }
            Self::Name(plan) => {
                if let Some(source) = plan.source() {
                    confirmation_path(ui, "From", source);
                }
                confirmation_path(ui, "To", plan.target());
                ui.label("No overwrite.");
            }
            Self::Undo(plan) => {
                for line in plan.effect_lines() {
                    ui.add(egui::Label::new(line).wrap());
                }
            }
            Self::UndoBatch(plan) => {
                for line in plan.effect_lines() {
                    ui.add(egui::Label::new(line).wrap());
                }
            }
            Self::Archive(plan) => {
                confirmation_path(ui, "To", plan.destination());
                let count = plan.member_count();
                ui.label(format!(
                    "{count} ZIP {}",
                    if count == 1 { "entry" } else { "entries" }
                ));
                ui.label(if plan.removes_sources() {
                    "Move: sources are removed after the destination is verified."
                } else {
                    "Copy: sources are kept."
                });
            }
        }
        ui.collapsing("Details", |ui| {
            ui.add(egui::Label::new(self.summary()).wrap());
            if let Self::Archive(archive) = self {
                egui::ScrollArea::both()
                    .id_salt("zip-exact-plan-rows")
                    .max_height(180.0)
                    .show_rows(ui, ROW_HEIGHT, archive.row_count(), |ui, range| {
                        for index in range {
                            ui.add(
                                egui::Label::new(archive.row(index))
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        }
                    });
            }
            if let Self::Undo(plan) = self {
                let rows = plan.rows();
                egui::ScrollArea::both()
                    .id_salt("undo-exact-plan-rows")
                    .max_height(180.0)
                    .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                        for row in &rows[range] {
                            ui.add(
                                egui::Label::new(row.as_str())
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        }
                    });
            }
            if let Self::UndoBatch(plan) = self {
                let rows = plan.rows();
                egui::ScrollArea::both()
                    .id_salt("undo-batch-rows")
                    .max_height(180.0)
                    .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                        for row in &rows[range] {
                            ui.add(
                                egui::Label::new(row.as_str())
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        }
                    });
            }
        });
    }
}

pub(super) enum Work {
    Change(Change),
    PrepareUndo {
        id: u64,
        key: Option<String>,
    },
    PrepareUndoBatch {
        id: u64,
        batch: String,
    },
    /// Checks every selected AI step, then runs them one by one if all checks pass.
    Batch(Box<tomas_commander::proposal::Batch>),
    History,
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
    PrepareExtraction {
        id: u64,
        archive: PathBuf,
        destination: PathBuf,
    },
    OpenArchiveEntry {
        archive: PathBuf,
        entry: String,
    },
    ExportDrag {
        id: u64,
        archive: PathBuf,
        selected: Vec<String>,
        cancel: Arc<AtomicBool>,
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
        policy: ConflictPolicy,
    },
    Replan {
        id: u64,
        operation: Operation,
        sources: Vec<PathBuf>,
        destination: PathBuf,
        policy: ConflictPolicy,
    },
    Execute(Plan),
    #[cfg(feature = "native-desktop-tests")]
    DesktopRejections(PathBuf),
    Save(Preferences),
    Code(PathBuf),
    Open(PathBuf),
    OpenFileWith(PathBuf),
    LocateRecent {
        pane: usize,
        path: PathBuf,
    },
    OpenUsageFolder(PathBuf),
    Copilot {
        path: PathBuf,
        app: bool,
    },
    Locate {
        pane: usize,
        result: search::Match,
    },
    LocateAgent {
        pane: usize,
        settings: Box<tomas_commander::file_tools::Settings>,
        record: tomas_commander::file_tools::Observed,
        preview: bool,
    },
}

pub(super) enum WorkResult {
    NamePrepared(FileResult<tomas_commander::naming::Plan>),
    UndoPrepared(FileResult<tomas_commander::undo::Plan>),
    UndoBatchPrepared(FileResult<tomas_commander::undo::BatchPlan>),
    /// The dry run of an AI batch: Ok means its steps now run; Err lists every problem.
    BatchChecked(Result<(), Vec<String>>),
    BatchStep {
        index: usize,
        total: usize,
        outcome: Outcome,
    },
    BatchFinished(tomas_commander::proposal::Report),
    History(FileResult<Vec<tomas_commander::undo::Entry>>),
    Changed {
        outcome: Outcome,
        editor: bool,
    },
    ArchivePrepared(FileResult<tomas_commander::archive_changes::Plan>),
    ArchiveOpened(FileResult<PathBuf>),
    DragExported {
        id: u64,
        result: FileResult<tomas_commander::archive_export::Export>,
    },
    #[cfg(feature = "native-desktop-tests")]
    DesktopRejections(FileResult<String>),
    CommandResolved(FileResult<commands::Resolution>),
    Recovery(String),
    Prepared(FileResult<Plan>),
    Replanned {
        id: u64,
        result: FileResult<Plan>,
    },
    Executed(Outcome),
    Saved(FileResult<()>),
    Code(FileResult<()>),
    Open(FileResult<PathBuf>),
    OpenFileWith(FileResult<()>),
    OpenedUsageFolder(FileResult<()>),
    Copilot {
        app: bool,
        result: FileResult<()>,
    },
    Located {
        pane: usize,
        result: FileResult<PathBuf>,
        via: NavVia,
    },
    AgentLocated {
        pane: usize,
        preview: bool,
        result: FileResult<PathBuf>,
    },
}

pub(super) fn prepare_name_work(
    scope: &Scope,
    journal: &tomas_commander::journal::Journal,
    id: u64,
    folder: &std::path::Path,
    source: Option<&std::path::Path>,
    name: &str,
    cancel: &AtomicBool,
) -> WorkResult {
    let result = journal
        .initialize()
        .and_then(|()| tomas_commander::naming::Plan::prepare(scope, id, folder, source, name));
    match result {
        Ok(plan) if source.is_none() => WorkResult::Changed {
            outcome: plan.execute(scope, journal, true, cancel),
            editor: false,
        },
        result => WorkResult::NamePrepared(result),
    }
}

/// Runs an approved AI batch on the file worker: a dry run of every step first (nothing runs
/// if any check fails), then each step in order. Progress goes to `send`; the final result is
/// returned.
pub(super) fn batch_work(
    scope: &Scope,
    journal: &tomas_commander::journal::Journal,
    batch: &tomas_commander::proposal::Batch,
    cancel: &AtomicBool,
    hooks: &files::Hooks,
    send: &mut dyn FnMut(WorkResult),
) -> WorkResult {
    use tomas_commander::proposal;
    if let Err(problems) = journal
        .initialize()
        .map_err(|error| vec![error])
        .and_then(|()| proposal::dry_run(scope, batch, cancel))
    {
        return WorkResult::BatchChecked(Err(problems));
    }
    send(WorkResult::BatchChecked(Ok(())));
    let total = batch.steps.len();
    WorkResult::BatchFinished(proposal::execute(
        scope,
        journal,
        batch,
        cancel,
        hooks,
        &mut |index, outcome| {
            send(WorkResult::BatchStep {
                index,
                total,
                outcome,
            })
        },
    ))
}

/// Prepares (never executes) an inverse plan for review.
pub(super) fn prepare_undo_work(
    scope: &Scope,
    journal: &tomas_commander::journal::Journal,
    id: u64,
    key: Option<&str>,
    cancel: &AtomicBool,
) -> WorkResult {
    WorkResult::UndoPrepared(
        journal
            .initialize()
            .and_then(|()| tomas_commander::undo::prepare(scope, journal, id, key, cancel)),
    )
}

/// Reads Operations rows. A cancel already set when the read starts targeted earlier work, so
/// only a cancel raised during the read stops it.
pub(super) fn history_work(
    scope: &Scope,
    journal: &tomas_commander::journal::Journal,
    cancel: &AtomicBool,
) -> WorkResult {
    let idle = AtomicBool::new(false);
    let cancel = if cancel.load(Ordering::Acquire) {
        &idle
    } else {
        cancel
    };
    WorkResult::History(tomas_commander::undo::history(
        scope,
        journal,
        tomas_commander::undo::HISTORY_LIMIT,
        cancel,
    ))
}

/// Queues one history read; requests coalesce while one is pending, so history reads never pile
/// up ahead of user work on the serial file worker.
pub(super) fn queue_history(work: &SyncSender<Work>, pending: &mut bool) -> Result<(), String> {
    if *pending {
        return Ok(());
    }
    work.try_send(Work::History)
        .map_err(|error| format!("Operation history was not queued: {error}"))?;
    *pending = true;
    Ok(())
}

impl Ledger {
    pub(super) fn request_history(&mut self) {
        if let Err(error) = queue_history(&self.work, &mut self.history_pending) {
            self.history = Some(Err(error));
        }
    }

    fn finish_operation(&mut self, outcome: Outcome, activity: RecentActivity) {
        self.usage_operation_finished(&outcome);
        self.operation_busy = false;
        self.history = None;
        self.last_batch = None;
        if let Err(error) = self.recent.record_many(
            outcome.created.iter().chain(&outcome.completed).cloned(),
            activity,
        ) {
            self.fail(error);
        }
        self.log(match outcome.summary() {
            Some(summary) => format!(
                "Operation #{}: {summary}, {} not completed{}",
                outcome.id,
                outcome.incomplete.len(),
                if outcome.cancelled {
                    " / cancelled"
                } else {
                    ""
                }
            ),
            None => format!(
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
            ),
        });
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
            self.navigate(index, self.panes[index].path.clone(), NavVia::Other);
        }
        self.last_outcome = Some(outcome);
        if self.operations_open {
            self.request_history();
        }
    }

    pub(super) fn prepare_archive(&mut self, request: tomas_commander::archive_changes::Request) {
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

    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        self.poll_cache_changes();
        self.poll_command_interpretation();
        self.poll_search();
        self.poll_scans(ctx);
        self.poll_work_results(ctx);
        self.poll_preferences_save(ctx);
    }

    fn poll_cache_changes(&mut self) {
        while let Ok(result) = self.cache_changes.try_recv() {
            match result {
                Ok(changes) => {
                    for warning in changes.warnings {
                        self.log(warning);
                    }
                    for index in 0..2 {
                        if changes.all
                            || self.panes[index]
                                .entries
                                .iter()
                                .any(|entry| changes.paths.contains(&entry.path))
                        {
                            self.navigate(index, self.panes[index].path.clone(), NavVia::Other);
                        }
                    }
                }
                Err(error) => self.fail(error),
            }
        }
    }

    fn poll_scans(&mut self, ctx: &egui::Context) {
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
            if !self.scans[result.pane].current(result.generation) {
                continue;
            }
            let elapsed = result.elapsed;
            let pane = &mut self.panes[result.pane];
            let listing = match result.content {
                ScanContent::Listing(listing) => listing,
                ScanContent::Aggregate {
                    path,
                    result: stats,
                } => {
                    if pane.apply_aggregate(&path, stats) {
                        aggregate_changed[result.pane] = true;
                    }
                    continue;
                }
            };
            pane.busy = false;
            match listing {
                Ok((listing, missing)) => {
                    let note = pane.show_listing(listing, missing);
                    self.status = format!("{} items / ready", pane.entries.len());
                    if result.pane == self.active {
                        self.file_focus_requested = true;
                    }
                    self.usage_scan_finished(result.pane, elapsed);
                    if let Some(note) = note {
                        self.log(note);
                    }
                }
                Err(error) => {
                    pane.pending_aggregates.clear();
                    pane.restore_scroll = None;
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
    }

    fn poll_work_results(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.work_results.try_recv() {
            match result {
                WorkResult::DragExported { id, result } => match result {
                    Ok(export) => {
                        if let Some(drag) = &mut self.zip_drag
                            && drag.id == id
                        {
                            drag.export = Some(export);
                        } else if let Err(error) = export.cleanup() {
                            self.fail(error);
                        }
                    }
                    Err(error) => {
                        if self.zip_drag.as_ref().is_some_and(|drag| drag.id == id) {
                            self.zip_drag = None;
                            egui::DragAndDrop::clear_payload(ctx);
                            self.fail(error);
                        } else if error != "Temporary ZIP extraction cancelled." {
                            self.fail(error);
                        }
                    }
                },
                WorkResult::ArchiveOpened(result) => {
                    let ok = result.is_ok();
                    self.usage.log.record(|| {
                        Event::new("zip")
                            .with("action", "member_open")
                            .with("ok", ok)
                    });
                    match result {
                        Ok(path) => {
                            if let Err(error) = self.recent.record(path, RecentActivity::Opened) {
                                self.fail(error);
                            }
                            self.log("Temporary ZIP copy opened; edits in the external app do not update the archive.");
                        }
                        Err(error) => self.fail(error),
                    }
                }
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
                            self.review = Some(PlanReview::new(&plan));
                            self.plan = Some(plan);
                            self.approval_focus = true;
                        }
                        Err(error) => self.fail(error),
                    }
                }
                WorkResult::Replanned { id, result } => {
                    self.operation_busy = false;
                    if self.cancellation.load(Ordering::Relaxed) {
                        self.log("Planning cancelled. No changes made.");
                        continue;
                    }
                    let next = match (&mut self.plan, &mut self.review) {
                        (Some(plan), Some(review)) => review.arrived(id, result, plan),
                        _ => None,
                    };
                    if let Some(policy) = next {
                        self.queue_replan(policy);
                    }
                }
                WorkResult::NamePrepared(result) => {
                    self.operation_busy = false;
                    // A new folder executes straight from preparation, so this result means its
                    // pending operation failed or was cancelled; renames start timing on approval.
                    self.usage.operation = None;
                    if self.cancellation.load(Ordering::Acquire) {
                        self.log("Name change cancelled; no effects.");
                        continue;
                    }
                    match result {
                        Ok(plan) => {
                            self.change_plan = Some(Change::Name(plan));
                            self.approval_focus = true;
                        }
                        Err(error) => self.fail(error),
                    }
                }
                WorkResult::UndoPrepared(result) => {
                    self.operation_busy = false;
                    if self.cancellation.load(Ordering::Acquire) {
                        self.log("Undo planning cancelled; no effects.");
                        continue;
                    }
                    match result {
                        Ok(plan) => {
                            self.change_plan = Some(Change::Undo(plan));
                            self.approval_focus = true;
                        }
                        Err(error) => self.fail(error),
                    }
                }
                WorkResult::UndoBatchPrepared(result) => {
                    self.operation_busy = false;
                    if self.cancellation.load(Ordering::Acquire) {
                        self.log("Batch undo planning cancelled; no effects.");
                        continue;
                    }
                    match result {
                        Ok(plan) => {
                            self.change_plan = Some(Change::UndoBatch(plan));
                            self.approval_focus = true;
                        }
                        Err(error) => self.fail(error),
                    }
                }
                WorkResult::BatchChecked(result) => self.batch_checked(result),
                WorkResult::BatchStep {
                    index,
                    total,
                    outcome,
                } => self.batch_step(index, total, outcome),
                WorkResult::BatchFinished(report) => self.finish_batch(report),
                WorkResult::History(result) => {
                    self.history_pending = false;
                    match result {
                        // Cancelled for queued work; read again once the dialog needs it.
                        Err(error) if error == tomas_commander::undo::HISTORY_CANCELLED => {
                            self.history = None;
                            if self.operations_open {
                                self.request_history();
                            }
                        }
                        result => self.history = Some(result),
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
                    self.finish_operation(
                        outcome,
                        if editor {
                            RecentActivity::Saved
                        } else {
                            RecentActivity::FileAction
                        },
                    );
                }
                WorkResult::Executed(outcome) => {
                    self.finish_operation(outcome, RecentActivity::FileAction)
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
                        if let Err(error) = self.recent.record(path, RecentActivity::Opened) {
                            self.fail(error);
                        }
                        self.log("Document opening requested.");
                    }
                    Err(error) => self.fail(error),
                },
                WorkResult::OpenFileWith(result) => {
                    if let Err(error) = result {
                        self.fail(error);
                    }
                }
                WorkResult::OpenedUsageFolder(result) => {
                    if let Err(error) = result {
                        self.fail(error);
                    }
                }
                WorkResult::Copilot { app, result } => match result {
                    Ok(()) => self.log(if app {
                        "GitHub Copilot App launch requested."
                    } else {
                        "GitHub Copilot CLI terminal requested (--yolo)."
                    }),
                    Err(error) => self.fail(error),
                },
                WorkResult::Located { pane, result, via } => match result {
                    Ok(path) => {
                        if let Some(parent) = path.parent() {
                            self.active = pane;
                            self.navigate(pane, parent.to_owned(), via);
                            self.panes[pane].focused = Some(path);
                            self.file_focus_requested = true;
                        }
                    }
                    Err(error) => self.fail(error),
                },
                WorkResult::AgentLocated {
                    pane,
                    preview,
                    result,
                } => match result {
                    Ok(path) if preview => {
                        if let Err(error) = self.reader.start(pane, path) {
                            self.fail(error);
                        }
                    }
                    Ok(path) => {
                        let directory = path.is_dir();
                        if directory {
                            self.active = pane;
                            self.navigate(pane, path, NavVia::AiResult);
                        } else if let Some(parent) = path.parent() {
                            self.active = pane;
                            self.navigate(pane, parent.to_owned(), NavVia::AiResult);
                            self.panes[pane].focused = Some(path);
                        }
                    }
                    Err(error) => self.fail(error),
                },
            }
        }
    }

    fn poll_preferences_save(&mut self, ctx: &egui::Context) {
        if self.save_pending && self.preferences_writable {
            // Every save carries the current tab folders.
            self.sync_tab_preferences();
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
        }
        // A close deferred for saving resumes once the last save has finished.
        if self.close_after_save
            && !(self.save_pending && self.preferences_writable)
            && self.saves_in_flight == 0
        {
            self.close_after_save = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

pub(super) fn queue_archive_open(
    work: &SyncSender<Work>,
    panel: &mut crate::viewer_ui::ReaderPanel,
    allowed: bool,
) -> FileResult<()> {
    let Some(entry) = panel.open_entry.take() else {
        return Ok(());
    };
    if !allowed {
        return Err("Finish pending work before opening a ZIP member.".into());
    }
    let (archive, _) = panel.archive_location();
    work.try_send(Work::OpenArchiveEntry { archive, entry })
        .map_err(|error| format!("Could not queue ZIP member opening: {error}"))
}
