use super::*;

impl Ledger {
    pub(super) fn zip_extract_target(&self) -> Option<PathBuf> {
        let targets = self.panes[self.active].targets();
        if targets.len() == 1
            && targets[0]
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
            && self.panes[self.active]
                .entries
                .iter()
                .any(|entry| entry.path == targets[0] && !entry.directory && !entry.link)
        {
            Some(targets[0].clone())
        } else {
            None
        }
    }

    fn zip_command(&mut self) {
        if !self.file_actions_allowed() || self.panes.iter().any(|pane| pane.busy) {
            self.fail("Finish pending work before a ZIP operation.");
            return;
        }
        if self.archive.target == Some(self.active) {
            match self.archive.extraction_request(
                self.panes[1 - self.active].path.clone(),
                self.archive.archive_targets().is_empty(),
                false,
            ) {
                Ok(request) => self.prepare_archive(request),
                Err(error) => self.fail(error),
            }
        } else if self.reader.target == Some(self.active) {
            self.fail("Tab back to the file or ZIP pane before using the ZIP command.");
        } else if let Some(archive) = self.zip_extract_target() {
            self.cancellation.store(false, Ordering::Release);
            match self.work.try_send(Work::PrepareExtraction {
                id: self.next_id,
                archive,
                destination: self.panes[1 - self.active].path.clone(),
            }) {
                Ok(()) => {
                    self.next_id += 1;
                    self.operation_busy = true;
                    self.log("Preparing ZIP extraction / no effects before approval.");
                }
                Err(error) => self.fail(format!("Could not queue ZIP extraction: {error}")),
            }
        } else {
            let sources = self.panes[self.active].targets();
            if sources.is_empty() {
                self.fail("Select files or folders to archive.");
                return;
            }
            let name = if sources.len() == 1 {
                sources[0]
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Archive")
            } else {
                "Archive"
            };
            self.zip_dialog = Some(ZipDialog {
                name: format!("{name}.zip"),
                sources,
                destination: self.panes[1 - self.active].path.clone(),
                focus: true,
            });
        }
    }

    /// Queues an inverse plan for review; `None` picks the most recent undoable operation.
    pub(super) fn prepare_undo(&mut self, key: Option<String>) {
        if self.operation_busy || self.plan.is_some() || self.change_plan.is_some() {
            self.fail("Finish or cancel the current operation first.");
            return;
        }
        if self.panes.iter().any(|pane| pane.busy) {
            self.fail("Wait for folder loading before planning an operation.");
            return;
        }
        self.cancellation.store(false, Ordering::Release);
        match self.work.try_send(Work::PrepareUndo {
            id: self.next_id,
            key,
        }) {
            Ok(()) => {
                self.next_id += 1;
                self.operation_busy = true;
                self.log("Preparing undo / no effects before approval.");
            }
            Err(error) => self.fail(format!("Undo was not queued: {error}")),
        }
    }

    /// Runs `command` and records how it was invoked (`via`) in the usage log.
    pub(super) fn dispatch(&mut self, via: Via, command: Command) {
        self.usage_dispatch(via, command, |app| app.dispatch_command(command));
    }

    fn dispatch_command(&mut self, command: Command) {
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
        if matches!(command, Command::CreateZip) {
            self.zip_command();
            return;
        }
        if self.archive.is_archive() && matches!(command, Command::Copy | Command::Move) {
            if self.operation_busy || self.plan.is_some() || self.reader.dirty() {
                self.fail("Finish pending work and Save/discard edits before a ZIP operation.");
                return;
            }
            let (archive, prefix) = self.archive.archive_location();
            let request = if self.archive.target == Some(self.active) {
                match self.archive.extraction_request(
                    self.panes[1 - self.active].path.clone(),
                    false,
                    matches!(command, Command::Move),
                ) {
                    Ok(request) => request,
                    Err(error) => {
                        self.fail(error);
                        return;
                    }
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
                    | Command::Recent
                    | Command::Favorites
                    | Command::Theme
                    | Command::Accent
                    | Command::Foundry
                    | Command::Answers
                    | Command::Activity
                    | Command::Balance
                    | Command::Usage
                    // Tab commands close the ZIP view first (see `leave_tab`).
                    | Command::NewTab
                    | Command::CloseTab
                    | Command::NextTab
                    | Command::PreviousTab
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
                    | Command::Recent
                    | Command::Favorites
                    | Command::Theme
                    | Command::Accent
                    | Command::Foundry
                    | Command::Answers
                    | Command::Activity
                    | Command::Balance
                    | Command::Usage
                    | Command::RotateLeft
                    | Command::RotateRight
                    | Command::EditSource
                    | Command::SaveSource
            )
        {
            self.fail("Tab back to the file pane before using a file command.");
            return;
        }
        self.run_command(command);
    }

    fn run_command(&mut self, command: Command) {
        match command {
            Command::Settings => {
                if !self.settings_open {
                    self.recent_roots_edit = self
                        .recent_snapshot
                        .roots
                        .as_ref()
                        .or(self.preferences.recent_roots.as_ref())
                        .map(|roots| {
                            roots
                                .iter()
                                .map(|path| files::display_path(path))
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                }
                self.settings_open = true;
            }
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
            Command::Clone | Command::Rename | Command::NewFolder => {
                if self.operation_busy
                    || self.plan.is_some()
                    || self.reader.dirty()
                    || self.panes[self.active].busy
                {
                    self.fail("Finish pending work and edits before changing names.");
                    return;
                }
                let pane = &self.panes[self.active];
                let clone = matches!(command, Command::Clone);
                let source = if matches!(command, Command::Clone | Command::Rename) {
                    let Some(path) = pane.focused.clone() else {
                        self.fail(if clone {
                            "Focus one regular file to clone."
                        } else {
                            "Focus one file or folder to rename."
                        });
                        return;
                    };
                    if clone
                        && !pane
                            .entries
                            .iter()
                            .any(|entry| entry.path == path && !entry.directory && !entry.link)
                    {
                        self.fail("Focus one regular file to clone.");
                        return;
                    }
                    Some(path)
                } else {
                    None
                };
                let name = if clone {
                    match tomas_commander::naming::suggest_clone_name(
                        source.as_deref().expect("Clone source"),
                    ) {
                        Ok(name) => name,
                        Err(error) => {
                            self.fail(error);
                            return;
                        }
                    }
                } else {
                    source
                        .as_ref()
                        .and_then(|path| path.file_name())
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default()
                };
                self.name_dialog = Some(NameDialog {
                    clone,
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
            Command::CreateZip => unreachable!("contextual ZIP command handled above"),
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
                        self.usage_filter_opened(self.active);
                        if let Err(error) = self.reader.toggle(self.active, path) {
                            self.fail(error);
                        }
                    }
                    None => self.fail("Choose a regular file, not the parent row, for preview."),
                }
            }
            Command::Parent => self.parent(self.active, NavVia::Parent),
            Command::ReturnInside => self.return_inside(self.active),
            Command::Usage => self.open_usage_insights(),
            Command::Back | Command::Forward => {
                self.step_history(self.active, matches!(command, Command::Forward));
            }
            Command::NewTab => self.new_tab(self.active),
            Command::CloseTab => self.close_tab(self.active, self.panes[self.active].tab),
            Command::NextTab | Command::PreviousTab => {
                self.cycle_tab(self.active, matches!(command, Command::NextTab));
            }
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
                    // Conflicts start as Skip; the review offers the other policies.
                    policy: ConflictPolicy::Skip,
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
            Command::Undo => {
                if self.reader.dirty() {
                    self.fail("Save or explicitly discard edits before planning a file mutation.");
                    return;
                }
                self.reader.close();
                self.prepare_undo(None);
            }
            Command::Refresh => {
                if let Err(error) = self.aggregate_cache.clear() {
                    self.fail(error);
                    return;
                }
                self.navigate(
                    self.active,
                    self.panes[self.active].path.clone(),
                    NavVia::Other,
                );
            }
            Command::Favorites => {
                self.rail_open = true;
                self.rail = Rail::Favorites;
                self.rail_index = Some(0);
            }
            Command::Recent => {
                toggle_recent_rail(&mut self.rail_open, &mut self.rail, &mut self.rail_index);
                self.file_focus_requested = !self.rail_open;
            }
            Command::Activity => {
                self.operations_open = true;
                self.request_history();
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
                self.panes[self.active].cycle_sort();
            }
            Command::Filter => self.editor_focus = Some(Id::new(("filter", self.active))),
            Command::AiSearch => {
                self.palette = false;
                self.file_agent.prepare(
                    String::new(),
                    self.panes[self.active].path.clone(),
                    self.active,
                    self.scope.clone(),
                );
            }
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
                    ai_assisted: matches!(command, Command::AiSearch),
                    interpreting: false,
                    interpretation: None,
                    started: None,
                    finished: None,
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
}
