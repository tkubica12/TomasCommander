use super::*;
use tomas_commander::files::MoveStrategy;

pub(super) struct ZipDialog {
    pub(super) sources: Vec<PathBuf>,
    pub(super) destination: PathBuf,
    pub(super) name: String,
    pub(super) focus: bool,
}
pub(super) struct NameDialog {
    pub(super) folder: PathBuf,
    pub(super) source: Option<PathBuf>,
    pub(super) name: String,
    pub(super) focus: bool,
    pub(super) error: Option<String>,
    pub(super) inline: bool,
    pub(super) pane: usize,
}
pub(super) struct OpenWith {
    pub(super) folder: PathBuf,
    pub(super) index: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsTab {
    Appearance,
    Recent,
    Connections,
    Usage,
}

impl Ledger {
    pub(super) fn prepare_name_change(&mut self) {
        let Some(dialog) = &mut self.name_dialog else {
            return;
        };
        if let Err(error) = tomas_commander::naming::validate_name(&dialog.name) {
            self.name_dialog = None;
            self.fail(error);
            return;
        }
        let dialog = self.name_dialog.take().expect("name dialog present");
        if dialog.source.is_none() {
            // New folder executes without a review; its timing starts here.
            self.usage.operation = Some(PendingOperation::named(
                "new_folder",
                1,
                Some(&dialog.folder),
            ));
        }
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
            Err(error) => {
                self.usage.operation = None;
                self.fail(format!("Name change was not queued: {error}"))
            }
        }
    }

    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        // A modal returning true owns this frame; later dialogs are skipped.
        if self.drive_picker_dialog(ctx)
            || self.place_edit_dialog(ctx)
            || self.settings_dialog(ctx)
            || self.operations_dialog(ctx)
            || self.usage_insights_dialog(ctx)
            || self.open_with_dialog(ctx)
            || self.proposal_dialog(ctx)
        {
            return;
        }
        if self
            .name_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.inline)
        {
            return;
        }
        if self.name_change_dialog(ctx) {
            return;
        }
        self.zip_name_dialog(ctx);
        if self.help_dialog(ctx) || self.saved_places_dialog(ctx) {
            return;
        }
        self.command_dialog_ui(ctx);
        self.search_dialog_ui(ctx);
        if self.palette_ui(ctx) {
            return;
        }
        self.change_approval_dialog(ctx);
        self.plan_approval_dialog(ctx);
        self.error_dialog(ctx);
    }

    fn drive_picker_dialog(&mut self, ctx: &egui::Context) -> bool {
        if let Some(picker) = &mut self.drives_popup {
            let pane = picker.pane;
            let (selected, close) = picker.show(ctx);
            if close || selected.is_some() {
                self.drives_popup = None;
                self.file_focus_requested = true;
                self.usage.outcome(
                    "drive_picker",
                    if selected.is_some() {
                        "confirmed"
                    } else {
                        "cancelled"
                    },
                );
            }
            if let Some(path) = selected {
                self.active = pane;
                self.navigate(pane, path, NavVia::Drive);
            }
            return true;
        }
        false
    }

    fn place_edit_dialog(&mut self, ctx: &egui::Context) -> bool {
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
                (save, cancel) = confirmation_buttons(ui, "Save", &mut false);
            });
            if cancel || modal.should_close() {
                self.place_edit = None;
                self.error = None;
                self.usage.outcome("place_name", "cancelled");
            } else if save {
                self.usage.outcome("place_name", "confirmed");
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
            return true;
        }
        false
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) -> bool {
        if self.settings_open {
            let mut connection = false;
            let mut close = false;
            let modal = egui::Modal::new(Id::new("settings")).show(ctx, |ui| {
                ui.set_width(660.0);
                ui.heading("Settings");
                ui.horizontal(|ui| {
                    for (tab, label) in [(SettingsTab::Appearance, "Appearance"), (SettingsTab::Recent, "Recent activity"), (SettingsTab::Connections, "AI connections"), (SettingsTab::Usage, "Usage log")] {
                        ui.selectable_value(&mut self.settings_tab, tab, label);
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().id_salt("settings-content").max_height(360.0).show(ui, |ui| {
                match self.settings_tab {
                SettingsTab::Appearance => {
                ui.heading("Appearance");
                ui.label("Theme, accent and scale for the entire application.");
                if ui.button(if self.preferences.dark { "Switch to light mode" } else { "Switch to dark mode" }).clicked() { self.dispatch(Via::Button, Command::Theme); }
                if ui.button(format!("Accent: {}", ACCENTS[self.preferences.accent].0)).clicked() { self.dispatch(Via::Button, Command::Accent); }
                ui.horizontal(|ui| {
                    if ui.button("-").clicked() { self.dispatch(Via::Button, Command::ZoomOut); }
                    ui.label(format!("UI zoom: {:.0}%", self.preferences.zoom * 100.0));
                    if ui.button("+").clicked() { self.dispatch(Via::Button, Command::ZoomIn); }
                    if ui.button("Reset").clicked() { self.dispatch(Via::Button, Command::ZoomReset); }
                });
                },
                SettingsTab::Recent => {
                    ui.heading("Recent activity");
                    ui.label("Watch these document folders while Tomas Commander is running.");
                    ui.label("One absolute path per line. Subfolders are included; an empty list disables external change tracking.");
                    ui.add_enabled_ui(self.scope.root().is_none(), |ui| {
                        let id = Id::new("recent-roots-input");
                        accessible_text_value(ctx, id, &mut self.recent_roots_edit);
                        ui.add(egui::TextEdit::multiline(&mut self.recent_roots_edit).id(id)
                            .desired_rows(5).desired_width(f32::INFINITY).hint_text("D:\\\nC:\\Users\\you\\OneDrive"));
                        if ui.button("Add active folder").clicked() {
                            if !self.recent_roots_edit.is_empty() { self.recent_roots_edit.push('\n'); }
                            self.recent_roots_edit.push_str(&files::display_path(&self.panes[self.active].path));
                        }
                        if ui.add_enabled(!self.recent_config_pending, egui::Button::new("Apply watched folders")).clicked() {
                            let roots = self.recent_roots_edit.lines().map(str::trim).filter(|line| !line.is_empty()).map(PathBuf::from).collect();
                            match self.recent.configure(roots) {
                                Ok(()) => self.recent_config_pending = true,
                                Err(error) => self.fail(error),
                            }
                        }
                    });
                    if self.recent_config_pending { ui.label("Validating and applying folders in the background..."); }
                    if self.scope.root().is_some() { ui.label("Test mode: tracking is restricted to the fixture root."); }
                    ui.label("Defaults: D: and detected personal/work OneDrive folders. Commander opens/views/saves are also recorded. Windows Recent is best effort, limited to these folders.");
                    ui.label("100 files, newest activity first. Startup inspects up to 30 days of modification times, with a bounded scan. No content upload or background service.");
                    for warning in &self.recent_snapshot.warnings { ui.colored_label(ui.visuals().warn_fg_color, warning); }
                },
                SettingsTab::Connections => {
                ui.heading("AI connections");
                connection = ui.button("Foundry connection / advanced pilot").clicked();
                ui.label("Connection configuration and explicit pilot controls stay separate from everyday file work.");
                },
                SettingsTab::Usage => self.usage_settings_ui(ui),
                }
                });
                if let Some(error) = &self.error { ui.colored_label(ui.visuals().error_fg_color, error); }
                ui.separator();
                close = ui.button("Close").on_hover_text("Escape").clicked();
            });
            if connection {
                self.settings_open = false;
                self.ai.open = true;
            } else if close || modal.should_close() {
                self.settings_open = false;
                self.usage.delete_confirm = false;
            }
            return true;
        }
        false
    }

    fn operations_dialog(&mut self, ctx: &egui::Context) -> bool {
        if self.operations_open {
            let mut close = false;
            let mut undo = None;
            let can_undo = !self.operation_busy
                && self.plan.is_none()
                && self.change_plan.is_none()
                && !self.reader.dirty();
            let modal = egui::Modal::new(Id::new("operations")).show(ctx, |ui| {
                ui.set_width(620.0);
                ui.heading("Operations");
                ui.label("Exact results, errors and partial paths. Undo prepares a reviewed plan; nothing is undone automatically.");
                if let Some(report) = &self.last_batch {
                    ui.label(RichText::new(report.summary()).strong());
                    egui::ScrollArea::vertical()
                        .id_salt("batch-steps")
                        .max_height(170.0)
                        .show(ui, |ui| {
                            for row in report.rows() {
                                let failed = row.contains(". FAILED: ")
                                    || row.starts_with("   ERROR: ");
                                let label = egui::Label::new(if failed {
                                    RichText::new(row).color(ui.visuals().error_fg_color)
                                } else {
                                    RichText::new(row)
                                })
                                .wrap();
                                ui.add(label);
                            }
                        });
                } else if let Some(outcome) = &self.last_outcome {
                    ui.label(format!("Operation #{}", outcome.id));
                    if let Some(summary) = outcome.summary() {
                        ui.label(summary);
                    }
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
                                ("REPLACED (original in Recycle Bin)", &outcome.replaced),
                                ("MERGED", &outcome.merged),
                                ("SKIPPED", &outcome.skipped),
                                (
                                    "RECYCLED (verified original in Recycle Bin)",
                                    &outcome.recycled,
                                ),
                                (
                                    "KEPT (original retained beside its verified copy)",
                                    &outcome.kept,
                                ),
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
                ui.separator();
                ui.label(RichText::new("Recent operations").strong());
                match &self.history {
                    None => {
                        ui.label("Reading the operation journal...");
                    }
                    Some(Err(error)) => {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                    Some(Ok(entries)) => undo = history_rows(ui, entries, can_undo),
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
                close = ui.button("Close").on_hover_text("Escape").clicked();
            });
            if close || modal.should_close() {
                self.operations_open = false;
            } else if let Some(choice) = undo {
                self.usage.outcome("operations", "confirmed");
                self.usage.last_command = "/undo";
                self.operations_open = false;
                match choice {
                    HistoryChoice::Undo(key) => self.prepare_undo(Some(key)),
                    HistoryChoice::UndoBatch(batch) => self.prepare_undo_batch(batch),
                }
            }
            return true;
        }
        false
    }

    fn open_with_dialog(&mut self, ctx: &egui::Context) -> bool {
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
                close = ui.button("Cancel").on_hover_text("Escape").clicked();
            });
            if close || modal.should_close() {
                self.open_with = None;
                self.usage.outcome("open_with", "cancelled");
            } else if let Some(index) = chosen {
                self.usage.outcome("open_with", "confirmed");
                let choice = ["vscode", "copilot_cli", "copilot_app"][index.min(2)];
                self.usage
                    .log
                    .record(|| Event::new("open_with").with("choice", choice));
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
            return true;
        }
        false
    }

    fn name_change_dialog(&mut self, ctx: &egui::Context) -> bool {
        if let Some(dialog) = &mut self.name_dialog {
            let usage_name = if dialog.source.is_some() {
                "rename"
            } else {
                "new_folder"
            };
            let mut prepare = false;
            let mut close = false;
            let modal = egui::Modal::new(Id::new("name-dialog")).show(ctx, |ui| {
                ui.set_width(500.0);
                ui.heading(if dialog.source.is_some() {
                    "Rename"
                } else {
                    "New folder"
                });
                confirmation_path(ui, "Folder", &dialog.folder);
                if let Some(source) = &dialog.source {
                    confirmation_path(ui, "File", source);
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
                (prepare, close) = confirmation_buttons(
                    ui,
                    if dialog.source.is_some() {
                        "Review"
                    } else {
                        "Create"
                    },
                    &mut false,
                );
            });
            if close || modal.should_close() {
                self.name_dialog = None;
                self.usage.outcome(usage_name, "cancelled");
            } else if prepare {
                if let Err(error) = tomas_commander::naming::validate_name(&dialog.name) {
                    dialog.error = Some(error);
                } else {
                    self.usage.outcome(usage_name, "confirmed");
                    self.prepare_name_change();
                }
            }
            return true;
        }
        false
    }

    fn zip_name_dialog(&mut self, ctx: &egui::Context) {
        if let Some(dialog) = &mut self.zip_dialog {
            let mut prepare = false;
            let mut close = false;
            let modal = egui::Modal::new(Id::new("create-zip-name")).show(ctx, |ui| {
                ui.set_width(600.0);
                ui.heading("Create ZIP");
                confirmation_path(ui, "Folder", &dialog.destination);
                ui.label(format!("{} selected items", dialog.sources.len()));
                let id = Id::new("zip-output-name");
                accessible_text_value(ctx, id, &mut dialog.name);
                let response = ui.add(egui::TextEdit::singleline(&mut dialog.name).id(id));
                if dialog.focus {
                    response.request_focus();
                    dialog.focus = false;
                }
                (prepare, close) = confirmation_buttons(ui, "Review", &mut false);
            });
            close |= modal.should_close();
            if close {
                self.zip_dialog = None;
                self.usage.outcome("zip_name", "cancelled");
            } else if prepare && let Some(dialog) = self.zip_dialog.take() {
                self.usage.outcome("zip_name", "confirmed");
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
    }

    fn help_dialog(&mut self, ctx: &egui::Context) -> bool {
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
                                    ("Ctrl Space", "Ask / AI and slash commands"),
                                    ("Tab", "Switch file pane"),
                                    ("Type in file pane", "Filter immediately"),
                                    ("Down from filter", "Focus first matching file"),
                                    ("Palette Enter / Shift Enter", "Run / insert newline"),
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
                                ui.label(RichText::new("Ask: sort, rename, move...").monospace());
                                ui.label("AI proposes changes; the bell opens the review. Arrows/Tab move, Space toggles, Enter on Run runs the checked ones. Deletes start unchecked.");
                                ui.end_row();
                            });
                    });
                close = ui.button("Close").on_hover_text("Escape").clicked();
            });
            if close || modal.should_close() {
                self.help_open = false;
                self.file_focus_requested = true;
            }
            return true;
        }
        false
    }

    fn saved_places_dialog(&mut self, ctx: &egui::Context) -> bool {
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
                close = ui.button("Cancel").on_hover_text("Escape").clicked();
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
            if selected.is_some() || pin || close || modal.should_close() {
                self.usage.outcome(
                    "places",
                    if selected.is_some() || pin {
                        "confirmed"
                    } else {
                        "cancelled"
                    },
                );
            }
            if selected.is_some() || close || modal.should_close() {
                self.places_popup = None;
                self.file_focus_requested = true;
            }
            if let Some(path) = selected {
                match self.scope.resolve(&path) {
                    Ok(path) if path.is_dir() => self.navigate(self.active, path, NavVia::Place),
                    Ok(_) => self.fail("Saved place is no longer a directory."),
                    Err(error) => self.fail(error),
                }
            }
            if pin {
                self.places_popup = None;
                self.dispatch(Via::Button, Command::Pin);
                self.file_focus_requested = true;
            }
            return true;
        }
        false
    }

    fn change_approval_dialog(&mut self, ctx: &egui::Context) {
        if let Some(plan) = &self.change_plan {
            let usage_name = change_name(plan);
            let mut approve = false;
            let mut cancel = false;
            let undo = matches!(plan, Change::Undo(_));
            let modal =
                egui::Modal::new(Id::new(("source-change-approval", plan.id()))).show(ctx, |ui| {
                    ui.set_width(520.0);
                    ui.add(egui::Label::new(RichText::new(plan.heading()).heading()).wrap());
                    plan.show_summary(ui);
                    ui.separator();
                    (approve, cancel) =
                        confirmation_buttons(ui, plan.confirm_label(), &mut self.approval_focus);
                });
            cancel |= modal.should_close();
            if cancel {
                self.change_plan = None;
                self.usage.outcome(usage_name, "cancelled");
                self.log(if undo {
                    "Undo cancelled; no changes."
                } else {
                    "Source change cancelled; no source write."
                });
            }
            if approve && let Some(plan) = self.change_plan.take() {
                self.usage.outcome(usage_name, "confirmed");
                if let Change::Save(save) = &plan {
                    let ext = save
                        .source()
                        .extension()
                        .map(|ext| ext.to_string_lossy().to_lowercase());
                    self.usage
                        .log
                        .record(|| Event::new("editor_save").with("ext", ext));
                }
                self.usage.operation = Some(PendingOperation::change(&plan));
                self.cancellation.store(false, Ordering::Relaxed);
                match self.work.try_send(Work::Change(plan)) {
                    Ok(()) => {
                        self.operation_busy = true;
                        self.log(if undo {
                            "Executing exactly approved undo..."
                        } else {
                            "Executing exactly approved source change..."
                        });
                    }
                    Err(error) => {
                        self.usage.operation = None;
                        self.fail(format!("Source change was not queued: {error}"))
                    }
                }
            }
        }
    }

    fn plan_approval_dialog(&mut self, ctx: &egui::Context) {
        let Some(plan) = &self.plan else {
            return;
        };
        let usage_name = review_name(plan);
        let review = self.review.get_or_insert_with(|| PlanReview::new(plan));
        let mut action = None;
        // The first plan's id keeps the dialog and its focus stable while re-planning.
        let modal = egui::Modal::new(Id::new(("approval", review.modal))).show(ctx, |ui| {
            action = plan_review(ui, plan, review, &mut self.approval_focus);
        });
        if modal.should_close() {
            action = Some(ReviewAction::Cancel);
        }
        match action {
            Some(ReviewAction::Cancel) => {
                if self
                    .review
                    .as_ref()
                    .is_some_and(|review| review.pending.is_some())
                {
                    self.cancellation.store(true, Ordering::Relaxed);
                }
                self.plan = None;
                self.review = None;
                self.usage.outcome(usage_name, "cancelled");
                self.log("Plan cancelled. No filesystem changes.");
                self.file_focus_requested = true;
            }
            Some(ReviewAction::Approve) => {
                if let Some(plan) = self.plan.take() {
                    self.review = None;
                    let pending = PendingOperation::plan(&plan);
                    match pending.policy {
                        Some(policy) => self.usage.outcome_with(
                            usage_name,
                            "confirmed",
                            format!("policy={policy} changes={}", self.usage.policy_changes),
                        ),
                        None => self.usage.outcome(usage_name, "confirmed"),
                    }
                    self.usage.operation = Some(pending);
                    self.cancellation.store(false, Ordering::Relaxed);
                    match self.work.try_send(Work::Execute(plan)) {
                        Ok(()) => {
                            self.operation_busy = true;
                            self.log("Executing approved plan...");
                        }
                        Err(error) => {
                            self.usage.operation = None;
                            self.fail(format!("Operation was not queued: {error}"))
                        }
                    }
                }
            }
            Some(ReviewAction::Policy(policy)) => {
                self.usage.policy_changes += 1;
                if let (Some(plan), Some(review)) = (&self.plan, &mut self.review)
                    && let Some(policy) = review.select(policy, plan)
                {
                    self.queue_replan(policy);
                }
            }
            None => (),
        }
    }

    pub(super) fn queue_replan(&mut self, policy: ConflictPolicy) {
        let Some(job) = self
            .plan
            .as_ref()
            .and_then(|plan| replan_work(plan, policy, self.next_id))
        else {
            return;
        };
        self.cancellation.store(false, Ordering::Relaxed);
        self.usage.last_command = "/conflict-policy";
        let result = self.work.try_send(job);
        let Some(review) = &mut self.review else {
            return;
        };
        match result {
            Ok(()) => {
                review.pending = Some((self.next_id, policy));
                self.next_id += 1;
                self.operation_busy = true;
                self.log(format!(
                    "Updating plan for '{}' / no effects before approval.",
                    policy.label()
                ));
            }
            Err(error) => review.error = Some(format!("Plan update was not queued: {error}")),
        }
    }

    fn error_dialog(&mut self, ctx: &egui::Context) {
        if let Some(error) = &self.error {
            let mut dismiss = false;
            let modal = egui::Modal::new(Id::new("error")).show(ctx, |ui| {
                ui.set_width(620.0);
                ui.heading("Error");
                ui.label(error);
                let button = ui.button("OK").on_hover_text("Enter / Escape");
                button.request_focus();
                dismiss = button.clicked() || consume_prompt_key(ctx, Modifiers::NONE, Key::Enter);
            });
            dismiss |= modal.should_close();
            if dismiss {
                self.error = None;
            }
        }
    }
}

/// What an Operations row asks to undo.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum HistoryChoice {
    Undo(String),
    UndoBatch(String),
}

/// Recent operations with Undo or the short reason it is unavailable; steps of one batch are
/// grouped under a row offering Undo batch.
pub(super) fn history_rows(
    ui: &mut egui::Ui,
    entries: &[tomas_commander::undo::Entry],
    enabled: bool,
) -> Option<HistoryChoice> {
    use tomas_commander::undo::{Entry, Status};
    if entries.is_empty() {
        ui.label("No recorded operations.");
        return None;
    }
    let mut chosen = None;
    let row = |ui: &mut egui::Ui, entry: &Entry, chosen: &mut Option<HistoryChoice>| {
        // Wrapping keeps long paths readable without a fixed column width.
        ui.horizontal_wrapped(|ui| {
            ui.label(entry.title());
            match &entry.status {
                Status::Undoable => {
                    if ui
                        .add_enabled(enabled, egui::Button::new("Undo"))
                        .on_hover_text("Prepare a reviewed inverse plan")
                        .on_disabled_hover_text("Finish pending work first")
                        .clicked()
                    {
                        *chosen = Some(HistoryChoice::Undo(entry.key.clone()));
                    }
                }
                Status::Undone => {
                    ui.label(RichText::new("Undone").weak());
                }
                Status::Refused(reason) => {
                    ui.label(RichText::new(reason).weak());
                }
            }
        });
    };
    egui::ScrollArea::vertical()
        .id_salt("operation-history")
        .max_height(ROW_HEIGHT * 7.0)
        .show(ui, |ui| {
            let mut index = 0;
            while index < entries.len() {
                let Some(batch) = &entries[index].batch else {
                    row(ui, &entries[index], &mut chosen);
                    index += 1;
                    continue;
                };
                let group: Vec<&Entry> = entries[index..]
                    .iter()
                    .take_while(|entry| entry.batch.as_ref() == Some(batch))
                    .collect();
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "AI changes: {} {}",
                            group.len(),
                            if group.len() == 1 { "step" } else { "steps" }
                        ))
                        .strong(),
                    );
                    if group.iter().any(|entry| entry.status == Status::Undoable)
                        && ui
                            .add_enabled(enabled, egui::Button::new("Undo batch"))
                            .on_hover_text(
                                "Prepare a reviewed undo of every step of this batch, newest first",
                            )
                            .on_disabled_hover_text("Finish pending work first")
                            .clicked()
                    {
                        chosen = Some(HistoryChoice::UndoBatch(batch.clone()));
                    }
                });
                ui.indent(("history-batch", index), |ui| {
                    for entry in &group {
                        row(ui, entry, &mut chosen);
                    }
                });
                index += group.len();
            }
        });
    chosen
}

pub(crate) fn confirmation_buttons(
    ui: &mut egui::Ui,
    label: &str,
    focus: &mut bool,
) -> (bool, bool) {
    confirmation_buttons_enabled(ui, label, focus, true)
}

/// Like `confirmation_buttons`; a disabled confirm still consumes Enter without approving.
pub(crate) fn confirmation_buttons_enabled(
    ui: &mut egui::Ui,
    label: &str,
    focus: &mut bool,
    enabled: bool,
) -> (bool, bool) {
    let (mut enter, mut escape) = (false, false);
    let mut key_event = false;
    ui.input_mut(|input| {
        input.events.retain(|event| {
            if let egui::Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } = event
                && matches!(key, Key::Enter | Key::Escape)
            {
                key_event = true;
                if modifiers.is_none() {
                    enter |= *key == Key::Enter && !repeat;
                    escape |= *key == Key::Escape;
                }
                false
            } else {
                true
            }
        });
    });
    ui.horizontal(|ui| {
        let cancel = ui.button("Cancel").on_hover_text("Escape");
        let confirm = ui
            .add_enabled(enabled, egui::Button::new(label))
            .on_hover_text("Enter");
        if *focus && enabled {
            confirm.request_focus();
            *focus = false;
        }
        // egui can synthesize button clicks before the key event is consumed here.
        let cancel_chosen =
            (cancel.clicked() && !key_event) || escape || (enter && cancel.has_focus());
        (
            enabled
                && !cancel_chosen
                && ((confirm.clicked() && !key_event) || (enter && !cancel.has_focus())),
            cancel_chosen,
        )
    })
    .inner
}

/// Conflict-policy state of the plan review; an approved plan is never mutated.
pub(super) struct PlanReview {
    pub(super) modal: u64,
    pub(super) policy: ConflictPolicy,
    pub(super) pending: Option<(u64, ConflictPolicy)>,
    pub(super) error: Option<String>,
    pub(super) option_ids: [Id; 4],
}

impl PlanReview {
    pub(super) fn new(plan: &Plan) -> Self {
        Self {
            modal: plan.id(),
            policy: plan.policy(),
            pending: None,
            error: None,
            option_ids: [Id::NULL; 4],
        }
    }

    pub(super) fn ready(&self, plan: &Plan) -> bool {
        self.pending.is_none() && self.error.is_none() && plan.policy() == self.policy
    }

    /// Records a choice; returns a policy to plan now when no plan is in flight.
    pub(super) fn select(&mut self, policy: ConflictPolicy, plan: &Plan) -> Option<ConflictPolicy> {
        if policy == self.policy {
            return None;
        }
        self.policy = policy;
        self.error = None;
        (self.pending.is_none() && plan.policy() != policy).then_some(policy)
    }

    /// Applies a re-plan result; returns a newer choice that still needs planning.
    pub(super) fn arrived(
        &mut self,
        id: u64,
        result: FileResult<Plan>,
        plan: &mut Plan,
    ) -> Option<ConflictPolicy> {
        let (_, requested) = self.pending.filter(|(pending, _)| *pending == id)?;
        self.pending = None;
        if requested != self.policy {
            return (plan.policy() != self.policy).then_some(self.policy);
        }
        match result {
            Ok(fresh) => *plan = fresh,
            Err(error) => self.error = Some(error),
        }
        None
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ReviewAction {
    Approve,
    Cancel,
    Policy(ConflictPolicy),
}

pub(super) fn replan_work(plan: &Plan, policy: ConflictPolicy, id: u64) -> Option<Work> {
    Some(Work::Replan {
        id,
        operation: plan.operation(),
        sources: plan.sources().to_vec(),
        destination: plan.destination()?.to_owned(),
        policy,
    })
}

pub(super) fn plan_review(
    ui: &mut egui::Ui,
    plan: &Plan,
    review: &mut PlanReview,
    focus: &mut bool,
) -> Option<ReviewAction> {
    ui.set_width(520.0);
    let count = plan.sources().len();
    ui.heading(format!(
        "{} {count} {}",
        plan.operation().name(),
        if count == 1 { "item" } else { "items" }
    ));
    if let Some(destination) = plan.destination() {
        confirmation_path(ui, "To", destination);
    } else {
        ui.label("To: Recycle Bin");
    }
    confirmation_sources(ui, plan.sources());
    let conflicts = plan.conflicts();
    let across = plan
        .move_strategy()
        .filter(|strategy| strategy.cross_volume());
    let mut chosen = None;
    if conflicts.any() {
        ui.label(match plan.operation() {
            Operation::Move => "Sources are moved; emptied merged folders are removed.",
            _ => "Sources are kept.",
        });
        cross_volume_line(ui, across);
        chosen = conflict_section(ui, plan, &conflicts, review, across);
    } else {
        ui.label(match plan.operation() {
            Operation::Copy => "Sources are kept. No overwrite.",
            Operation::Move => "Sources are moved. No overwrite.",
            Operation::Recycle => "Recoverable from the Recycle Bin. No permanent deletion.",
        });
        cross_volume_line(ui, across);
        if let Some(strategy) = across {
            ui.collapsing("Details", |ui| {
                ui.add(egui::Label::new(strategy.detail()).wrap());
            });
        }
    }
    ui.separator();
    let label = if conflicts.replace > 0 {
        format!(
            "{} + replace {}",
            plan.operation().name(),
            conflicts.replace
        )
    } else {
        plan.operation().name().to_owned()
    };
    let (approve, cancel) = confirmation_buttons_enabled(ui, &label, focus, review.ready(plan));
    // A choice made in the same frame as Enter must never approve the previous plan.
    if cancel {
        Some(ReviewAction::Cancel)
    } else if let Some(policy) = chosen {
        Some(ReviewAction::Policy(policy))
    } else if approve {
        Some(ReviewAction::Approve)
    } else {
        None
    }
}

/// The one-line effect of a Move between drives, shown under the review summary.
pub(super) fn cross_volume_text(strategy: MoveStrategy) -> Option<&'static str> {
    match strategy {
        MoveStrategy::SameVolume => None,
        MoveStrategy::CrossVolume {
            recycle_originals: true,
        } => Some("Across drives: copy, verify, then send originals to the Recycle Bin"),
        MoveStrategy::CrossVolume {
            recycle_originals: false,
        } => Some(
            "Source drive has no Recycle Bin: files will be copied and verified, originals are kept",
        ),
    }
}

fn cross_volume_line(ui: &mut egui::Ui, strategy: Option<MoveStrategy>) {
    if let Some(text) = strategy.and_then(cross_volume_text) {
        ui.add(egui::Label::new(RichText::new(text).strong()).wrap());
    }
}

fn conflict_section(
    ui: &mut egui::Ui,
    plan: &Plan,
    conflicts: &files::ConflictSummary,
    review: &mut PlanReview,
    strategy: Option<MoveStrategy>,
) -> Option<ConflictPolicy> {
    let plural = |count: usize, one: &str, many: &str| {
        format!("{count} {}", if count == 1 { one } else { many })
    };
    ui.add_space(4.0);
    ui.label(RichText::new("Some items already exist in the destination").strong());
    let mut counts = vec![plural(conflicts.new_items, "new item", "new items")];
    if conflicts.merged > 0 {
        counts.push(plural(conflicts.merged, "folder merged", "folders merged"));
    }
    ui.label(counts.join(" / "));
    let files = conflicts.newer + conflicts.older_or_same;
    if files > 0 {
        ui.label(format!(
            "{}: {} source newer / {} source older or same",
            plural(files, "file conflict", "file conflicts"),
            conflicts.newer,
            conflicts.older_or_same
        ));
    }
    if conflicts.mismatched > 0 {
        ui.label(plural(
            conflicts.mismatched,
            "file/folder type conflict",
            "file/folder type conflicts",
        ));
    }
    if !conflicts.examples.is_empty() {
        let mut names = conflicts
            .examples
            .iter()
            .take(5)
            .map(|path| files::display_path(path))
            .collect::<Vec<_>>()
            .join(", ");
        if conflicts.examples.len() > 5 {
            names.push_str(&format!(" (+{} more)", conflicts.examples.len() - 5));
        }
        ui.add(egui::Label::new(RichText::new(names).monospace().size(12.0)).wrap());
    }
    ui.collapsing("Details", |ui| {
        egui::ScrollArea::vertical()
            .id_salt("conflict-rows")
            .max_height(ROW_HEIGHT * 5.0)
            .show(ui, |ui| {
                if let Some(strategy) = strategy {
                    ui.add(egui::Label::new(strategy.detail()).wrap());
                }
                for row in plan.conflict_rows() {
                    ui.add(egui::Label::new(row).wrap_mode(egui::TextWrapMode::Extend));
                }
            });
    });
    ui.label("If an item already exists:");
    // Enter/Escape belong to the confirmation buttons, never to a focused choice.
    let keys = ui.input(|input| {
        input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: Key::Enter | Key::Escape,
                    pressed: true,
                    ..
                }
            )
        })
    });
    let responses: Vec<egui::Response> = ui
        .scope_builder(
            egui::UiBuilder::new().id(Id::new("conflict-policy")),
            |ui| {
                ui.horizontal(|ui| {
                    ConflictPolicy::CHOICES
                        .iter()
                        .map(|policy| {
                            let response = ui.add(egui::Button::selectable(
                                review.policy == *policy,
                                policy.label(),
                            ));
                            ui.ctx().accesskit_node_builder(response.id, |node| {
                                node.set_role(egui::accesskit::Role::RadioButton);
                                node.set_label(format!(
                                    "If an item already exists: {}",
                                    policy.label()
                                ));
                            });
                            response
                        })
                        .collect()
                })
                .inner
            },
        )
        .inner;
    let mut chosen = None;
    for (index, response) in responses.iter().enumerate() {
        review.option_ids[index] = response.id;
        if response.clicked() && !keys {
            chosen = Some(ConflictPolicy::CHOICES[index]);
        }
    }
    if let Some(index) = responses.iter().position(egui::Response::has_focus) {
        let step = ui.input_mut(|input| {
            if input.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                Some(-1)
            } else if input.consume_key(Modifiers::NONE, Key::ArrowRight) {
                Some(1)
            } else {
                None
            }
        });
        if let Some(step) = step {
            // Arrows change the choice here instead of moving focus spatially.
            ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            let next = index.saturating_add_signed(step).min(responses.len() - 1);
            responses[next].request_focus();
            chosen = Some(ConflictPolicy::CHOICES[next]);
        }
    }
    if review.pending.is_some() {
        ui.label("Updating the plan for this choice...");
    } else if let Some(error) = &review.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    } else {
        let mut effects = Vec::new();
        if conflicts.replace > 0 {
            effects.push(format!(
                "{} replaced; originals go to the Recycle Bin",
                conflicts.replace
            ));
        }
        if conflicts.skip > 0 {
            effects.push(format!("{} skipped", conflicts.skip));
        }
        if conflicts.rename > 0 {
            effects.push(format!("{} kept with a new name", conflicts.rename));
        }
        ui.label(if effects.is_empty() {
            "Folders are merged; no existing file changes.".to_owned()
        } else {
            effects.join(" / ")
        });
    }
    chosen.filter(|policy| *policy != review.policy)
}

pub(super) fn confirmation_path(ui: &mut egui::Ui, label: &str, path: &std::path::Path) {
    ui.horizontal_top(|ui| {
        ui.label(RichText::new(format!("{label}:")).strong());
        ui.add(
            egui::Label::new(
                RichText::new(files::display_path(path))
                    .monospace()
                    .size(12.0),
            )
            .wrap(),
        );
    });
}

fn confirmation_sources(ui: &mut egui::Ui, sources: &[PathBuf]) {
    let parent = sources
        .first()
        .and_then(|path| path.parent())
        .filter(|parent| sources.iter().all(|path| path.parent() == Some(*parent)));
    if let Some(parent) = parent {
        confirmation_path(ui, "From", parent);
    }
    egui::ScrollArea::vertical()
        .id_salt("confirmation-sources")
        .max_height(ROW_HEIGHT * 5.0)
        .show(ui, |ui| {
            for path in sources {
                let name = if parent.is_some() {
                    path.file_name().map(std::path::Path::new).unwrap_or(path)
                } else {
                    path
                };
                ui.add(
                    egui::Label::new(
                        RichText::new(files::display_path(name))
                            .monospace()
                            .size(12.0),
                    )
                    .wrap(),
                )
                .on_hover_text(files::display_path(path));
            }
        });
}

pub(super) struct DrivePicker {
    pub(super) pane: usize,
    drives: Vec<PathBuf>,
    pub(super) index: usize,
}

impl DrivePicker {
    pub(super) fn new(pane: usize, drives: Vec<PathBuf>, current: &std::path::Path) -> Self {
        let current = files::display_path(current);
        let index = drives
            .iter()
            .position(|drive| files::display_path(drive).eq_ignore_ascii_case(&current))
            .unwrap_or(0);
        Self {
            pane,
            drives,
            index,
        }
    }

    pub(super) fn show(&mut self, ctx: &egui::Context) -> (Option<PathBuf>, bool) {
        let mut selected = None;
        let mut close = false;
        let modal = egui::Modal::new(Id::new("choose-drive")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.heading("Choose drive");
            if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                self.index = self.index.saturating_sub(1);
            }
            if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                self.index = (self.index + 1).min(self.drives.len().saturating_sub(1));
            }
            if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter)) {
                selected = self.drives.get(self.index).cloned();
            }
            if self.drives.is_empty() {
                ui.label("No Windows drives are available.");
            }
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for (index, drive) in self.drives.iter().enumerate() {
                        let response = ui.add_sized(
                            Vec2::new(ui.available_width(), 28.0),
                            egui::Button::new(files::display_path(drive))
                                .selected(index == self.index),
                        );
                        if response.clicked() {
                            selected = Some(drive.clone());
                        }
                        if index == self.index {
                            response.scroll_to_me(None);
                        }
                    }
                });
            ui.separator();
            close = ui.button("Cancel").on_hover_text("Escape").clicked();
        });
        (selected, close || modal.should_close())
    }
}
