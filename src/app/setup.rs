use super::*;

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
        let aggregate_cache = Arc::new(tomas_commander::directory_cache::DirectoryCache::default());
        let (cache_sender, cache_changes) = mpsc::sync_channel(8);
        let cache_weak = Arc::downgrade(&aggregate_cache);
        let cache_context = creation.egui_ctx.clone();
        spawn_cache_watcher(cache_weak, cache_sender, cache_context);
        let mut scans = Vec::new();
        for pane in 0..2 {
            let (sender, receiver) = mpsc::sync_channel::<ScanRequest>(1);
            let generation = Arc::new(AtomicU64::new(0));
            let current = generation.clone();
            let results = scan_tx.clone();
            let scope = scope.clone();
            let context = creation.egui_ctx.clone();
            let cache = aggregate_cache.clone();
            spawn_scan_worker(pane, receiver, current, results, scope, context, cache);
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
        spawn_search_worker(
            search_requests,
            search_tx,
            generation,
            search_scope,
            search_context,
        );
        let (results, work_results) = mpsc::sync_channel(8);
        let cancellation = Arc::new(AtomicBool::new(false));
        let cancel = cancellation.clone();
        let worker_scope = scope.clone();
        let context = creation.egui_ctx.clone();
        let recent_context = creation.egui_ctx.clone();
        let recent = recent::Monitor::start(
            recent::Config::for_app(
                &settings,
                scope.root(),
                preferences.recent_documents.clone(),
                preferences.recent_roots.clone(),
            ),
            move || recent_context.request_repaint(),
        );
        spawn_file_worker(jobs, results, cancel, worker_scope, context, settings);
        let mut app = Self {
            reader: crate::viewer_ui::ReaderPanel::new(scope.clone(), creation.egui_ctx.clone()),
            change_plan: None,
            archive: crate::viewer_ui::ReaderPanel::new(scope.clone(), creation.egui_ctx.clone()),
            zip_dialog: None,
            zip_drag: None,
            ai: crate::ai_ui::AiPanel::new(),
            file_agent: crate::file_agent_ui::FileAgentPanel::new(),
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
            settings_tab: SettingsTab::Appearance,
            recent_roots_edit: String::new(),
            recent_config_pending: false,
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
            review: None,
            next_id: 1,
            last_outcome: None,
            rail: Rail::Recent,
            rail_index: None,
            recent,
            recent_snapshot: Default::default(),
            recent_pending: None,
            activity: Vec::new(),
            status: "Native Rust / loading folders".into(),
            error,
            palette: false,
            palette_query: String::new(),
            palette_focus: false,
            palette_index: 0,
            palette_command_selection: false,
            rail_open: false,
            places_popup: None,
            drives_popup: None,
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
        app.load_windows_fonts(&creation.egui_ctx);
        app.apply_theme(&creation.egui_ctx);
        creation
            .egui_ctx
            .options_mut(|options| options.zoom_with_keyboard = false);
        creation.egui_ctx.set_zoom_factor(app.preferences.zoom);
        app.navigate(0, initial.clone());
        app.navigate(1, initial);
        app
    }

    #[cfg(windows)]
    fn load_windows_fonts(&mut self, ctx: &egui::Context) {
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
                    Err(error) => self.log(format!(
                        "Appearance: could not read {}: {error}. Using bundled fallback fonts.",
                        files::display_path(&path),
                    )),
                }
            }
            ctx.set_fonts(fonts);
        } else {
            self.log("Appearance: WINDIR is not defined. Using bundled fallback fonts.");
        }
    }
}

fn spawn_cache_watcher(
    cache_weak: std::sync::Weak<tomas_commander::directory_cache::DirectoryCache>,
    cache_sender: SyncSender<FileResult<tomas_commander::directory_cache::Changes>>,
    cache_context: egui::Context,
) {
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
}

fn spawn_scan_worker(
    pane: usize,
    receiver: Receiver<ScanRequest>,
    current: Arc<AtomicU64>,
    results: SyncSender<ScanResult>,
    scope: Scope,
    context: egui::Context,
    cache: Arc<tomas_commander::directory_cache::DirectoryCache>,
) {
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
}

fn spawn_search_worker(
    search_requests: Receiver<SearchRequest>,
    search_tx: SyncSender<(u64, FileResult<search::Results>)>,
    generation: Arc<AtomicU64>,
    search_scope: Scope,
    search_context: egui::Context,
) {
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
                None => search::discover(&search_scope, &request.root, &request.query, cancelled),
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
}

fn spawn_file_worker(
    jobs: Receiver<Work>,
    results: SyncSender<WorkResult>,
    cancel: Arc<AtomicBool>,
    worker_scope: Scope,
    context: egui::Context,
    settings: PathBuf,
) {
    thread::spawn(move || {
        if let Err(error) =
            tomas_commander::archive_export::cleanup_expired(&worker_scope, SystemTime::now())
        {
            if results.send(WorkResult::Recovery(error)).is_err() {
                return;
            }
            context.request_repaint();
        }
        let journal = tomas_commander::journal::Journal::new(settings.with_extension("operations"));
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
                } => prepare_name_work(
                    &worker_scope,
                    &journal,
                    id,
                    &folder,
                    source.as_deref(),
                    &name,
                    &cancel,
                ),
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
                Work::PrepareExtraction {
                    id,
                    archive,
                    destination,
                } => WorkResult::ArchivePrepared(journal.initialize().and_then(|()| {
                    let inventory =
                        tomas_commander::archives::inventory(&worker_scope, &archive, || {
                            cancel.load(Ordering::Acquire)
                        })?;
                    tomas_commander::archive_changes::prepare(
                        &worker_scope,
                        id,
                        tomas_commander::archive_changes::Request::Extract {
                            archive,
                            prefix: String::new(),
                            selected: inventory
                                .entries
                                .into_iter()
                                .map(|entry| entry.path)
                                .collect(),
                            destination,
                            move_entries: false,
                        },
                        &cancel,
                    )
                })),
                Work::OpenArchiveEntry { archive, entry } => WorkResult::ArchiveOpened((|| {
                    let exported = tomas_commander::archive_export::prepare(
                        &worker_scope,
                        &archive,
                        &[entry],
                        &AtomicBool::new(false),
                    )?;
                    if let Err(error) = platform::open_file(&exported.paths[0]) {
                        return match exported.cleanup() {
                            Ok(()) => Err(error),
                            Err(cleanup) => Err(format!("{error}\n{cleanup}")),
                        };
                    }
                    exported.retain();
                    Ok(archive)
                })(
                )),
                Work::ExportDrag {
                    id,
                    archive,
                    selected,
                    cancel,
                } => WorkResult::DragExported {
                    id,
                    result: tomas_commander::archive_export::prepare(
                        &worker_scope,
                        &archive,
                        &selected,
                        &cancel,
                    ),
                },
                #[cfg(feature = "native-desktop-tests")]
                Work::DesktopRejections(root) => WorkResult::DesktopRejections(
                    tomas_commander::journal::Journal::desktop_rejection_probe(
                        &worker_scope,
                        settings.with_extension("rejections"),
                        &root.join("alpha.txt"),
                        &root.join("Příliš žluťoučký folder"),
                    ),
                ),
                Work::ResolveCommand { roots, intent } => WorkResult::CommandResolved(
                    journal
                        .initialize()
                        .and_then(|()| commands::resolve(&worker_scope, &roots, &intent, &cancel)),
                ),
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
                    policy,
                } => WorkResult::Prepared(journal.initialize().and_then(|()| {
                    files::plan_with_policy(
                        &worker_scope,
                        id,
                        operation,
                        &sources,
                        Some(&destination),
                        policy,
                        &cancel,
                    )
                })),
                Work::Replan {
                    id,
                    operation,
                    sources,
                    destination,
                    policy,
                } => WorkResult::Replanned {
                    id,
                    result: journal.initialize().and_then(|()| {
                        files::plan_with_policy(
                            &worker_scope,
                            id,
                            operation,
                            &sources,
                            Some(&destination),
                            policy,
                            &cancel,
                        )
                    }),
                },
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
                Work::OpenFileWith(path) => WorkResult::OpenFileWith(
                    worker_scope
                        .resolve(&path)
                        .and_then(|path| platform::open_file_with(&path)),
                ),
                Work::LocateRecent { pane, path } => WorkResult::Located {
                    pane,
                    result: worker_scope.resolve(&path),
                },
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
                Work::LocateAgent {
                    pane,
                    settings,
                    record,
                    preview,
                } => WorkResult::AgentLocated {
                    pane,
                    preview,
                    result: tomas_commander::file_agent::locate(&settings, &record)
                        .and_then(|path| worker_scope.resolve(&path)),
                },
            };
            if results.send(result).is_err() {
                break;
            }
            context.request_repaint();
        }
    });
}
