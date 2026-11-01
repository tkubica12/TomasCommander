use super::*;

impl eframe::App for Ledger {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.usage_session_end();
        if let Err(error) = self.recent.shutdown() {
            platform::show_error(&error);
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.usage_frame_begin(ctx);
        enforce_stable_minimum_window_size(ctx, &mut self.viewport_zoom, self.preferences.zoom);
        self.zoom_input(ctx);
        #[cfg(feature = "native-desktop-tests")]
        if let Some(metrics) = &mut self.desktop_metrics {
            metrics.begin(ctx);
        }
        self.ai.poll();
        self.file_agent.poll();
        self.answers.poll();
        self.reader.poll();
        self.archive.poll();
        self.usage_panels();
        if let Some(path) = self.archive.recent_activity.take()
            && let Err(error) = self.recent.record(path, RecentActivity::Opened)
        {
            self.fail(error);
        }
        if let Some(path) = self.reader.recent_activity.take()
            && let Err(error) = self.recent.record(path, RecentActivity::Previewed)
        {
            self.fail(error);
        }
        match self.recent.poll() {
            Ok(Some(snapshot)) => self.recent_pending = Some(snapshot),
            Ok(None) => {}
            Err(error) => {
                self.recent_config_pending = false;
                self.fail(error);
            }
        }
        if !ctx.egui_is_using_pointer()
            && !egui::Popup::is_any_open(ctx)
            && !egui::DragAndDrop::has_payload_of_type::<DragItems>(ctx)
            && let Some(snapshot) = self.recent_pending.take()
        {
            if self.rail == Rail::Recent {
                self.rail_index = remap_recent_cursor(
                    &self.recent_snapshot.items,
                    &snapshot.items,
                    self.rail_index,
                );
            }
            for warning in &snapshot.warnings {
                if !self.recent_snapshot.warnings.contains(warning) {
                    self.log(warning.clone());
                }
                if snapshot.configuration_revision > self.recent_snapshot.configuration_revision {
                    self.recent_config_pending = false;
                }
            }
            if self.scope.root().is_none()
                && let Some(roots) = &snapshot.roots
                && self.preferences.recent_roots.as_ref() != Some(roots)
            {
                self.preferences.recent_roots = Some(roots.clone());
                self.save_pending = true;
                if self.recent_roots_edit.is_empty() {
                    self.recent_roots_edit = roots
                        .iter()
                        .map(|path| files::display_path(path))
                        .collect::<Vec<_>>()
                        .join("\n");
                }
            }
            self.recent_snapshot = snapshot;
        }
        self.poll(ctx);
        if std::mem::take(&mut self.reader.edit_requested) {
            self.dispatch(Via::Button, Command::EditSource);
        }
        if std::mem::take(&mut self.reader.save_requested) {
            self.dispatch(Via::Button, Command::SaveSource);
        }
        if let Some(entry) = self.archive.preview_entry.take() {
            let (path, _) = self.archive.archive_location();
            if let Err(error) = self.reader.start_virtual(self.archive.source, path, entry) {
                self.fail(error);
            }
        }
        let allowed = self.file_actions_allowed();
        if let Err(error) = queue_archive_open(&self.work, &mut self.archive, allowed) {
            self.fail(error);
        }
        let allowed = self.file_actions_allowed();
        if let Err(error) = queue_archive_open(&self.work, &mut self.reader, allowed) {
            self.fail(error);
        }
        if let Some(entry) = self.reader.preview_entry.take() {
            let (path, _) = self.reader.archive_location();
            if let Err(error) = self.reader.start_virtual(self.reader.source, path, entry) {
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
            || self.drives_popup.is_some()
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
            || self.file_agent.open
            || self.answers.open
            || self.usage.insights.is_some()
            || self.proposal_review.is_some()
        {
            ui.disable();
        }
        self.handle_close_request(&ctx);
        if self.first_frame {
            self.first_frame = false;
            self.usage_session_start(&ctx);
            self.log(format!(
                "Native Ledger initialized in {:.0} ms / real filesystem",
                self.started.elapsed().as_secs_f64() * 1000.0
            ));
        }
        self.header_panel(ui);
        self.status_panel(ui);
        self.actions_panel(ui);
        self.context_rail(ui);
        self.file_panes(ui, &ctx);
        self.dialogs(&ctx);
        self.ai.show(&ctx);
        if let Some(action) = self.file_agent.show(&ctx) {
            match action {
                crate::file_agent_ui::Action::Settings => self.ai.open = true,
                crate::file_agent_ui::Action::Review {
                    proposal,
                    settings,
                    settings_path,
                } => self.open_proposal(*proposal, *settings, settings_path),
                crate::file_agent_ui::Action::Locate {
                    pane,
                    settings,
                    record,
                    preview,
                    rank,
                } => {
                    let path = files::display_path(&record.path);
                    self.usage.log.record(|| {
                        Event::new("ai_result")
                            .with("rank", rank)
                            .with("preview", preview)
                            .with("path", path)
                    });
                    if let Err(error) = self.work.try_send(Work::LocateAgent {
                        pane,
                        settings,
                        record,
                        preview,
                    }) {
                        self.fail(format!("AI result activation was not queued: {error}"));
                    }
                }
            }
        }
        self.answers.show(&ctx);
        self.finish_drag(&ctx);
        self.usage_frame_end(&ctx);
        #[cfg(feature = "native-desktop-tests")]
        if let Some(metrics) = &mut self.desktop_metrics {
            let listing_ready = self.panes.iter().all(|pane| !pane.busy) && self.error.is_none();
            let pending_scan = self
                .panes
                .iter()
                .any(|pane| pane.busy || !pane.pending_aggregates.is_empty());
            let pending_search = self
                .search_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.busy);
            metrics.finish(listing_ready, pending_scan, pending_search);
        }
    }
}

impl Ledger {
    fn handle_close_request(&mut self, ctx: &egui::Context) {
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
        } else if self.file_agent.busy() && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.file_agent.cancel();
            self.file_agent.open = true;
        } else if self.ai.busy() && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.ai.cancel();
            if self.search_dialog.is_none() {
                self.ai.open = true;
            }
        } else if self.recent_config_pending
            && ctx.input(|input| input.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.log("Recent settings are still being validated. Close again after they finish.");
        } else if ctx.input(|input| input.viewport().close_requested()) {
            self.sync_tab_preferences();
            if (self.save_pending && self.preferences_writable) || self.saves_in_flight > 0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.close_after_save = true;
            }
        }
    }

    fn header_panel(&mut self, ui: &mut egui::Ui) {
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
                            self.dispatch(Via::Button, Command::Settings);
                        }
                        #[cfg(feature = "native-desktop-tests")]
                        if self.desktop_metrics.is_some()
                            && ui.button("Command guard tests").clicked()
                        {
                            self.dispatch(Via::Other, Command::AiPlan);
                        }
                        if action_button(ui, "Ask", "Ctrl Space", true).clicked() {
                            self.usage_action("/ask", Via::Button, "Ctrl Space");
                            self.palette = true;
                            self.palette_focus = true;
                            self.palette_command_selection = false;
                            self.palette_query.clear();
                        }
                        self.file_agent.notification_button(ui);
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
                            self.dispatch(Via::Button, Command::Theme);
                        }
                        if action_button(
                            ui,
                            &format!("Accent: {}", ACCENTS[self.preferences.accent].0),
                            "",
                            true,
                        )
                        .clicked()
                        {
                            self.dispatch(Via::Button, Command::Accent);
                        }
                    });
                });
            });
    }

    fn status_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let available = self.plan.is_none()
                    && self.error.is_none()
                    && self.search_dialog.is_none()
                    && self.command_dialog.is_none()
                    && !self.ai.open
                    && !self.file_agent.open
                    && !self.answers.open
                    && self.places_popup.is_none()
                    && self.drives_popup.is_none()
                    && self.place_edit.is_none()
                    && self.proposal_review.is_none();
                if ui
                    .add_enabled(available, egui::Button::new("? Help"))
                    .clicked()
                {
                    self.dispatch(Via::Button, Command::Help);
                }
                if ui
                    .button(format!("{:.0}%", self.preferences.zoom * 100.0))
                    .on_hover_text("UI zoom / click to reset / Ctrl +, Ctrl -, Ctrl wheel")
                    .clicked()
                {
                    self.dispatch(Via::Button, Command::ZoomReset);
                }
                if ui
                    .button(if self.operation_busy {
                        "Operations / running"
                    } else if self.last_outcome.as_ref().is_some_and(|o| {
                        o.error.is_some() || o.cancelled || !o.incomplete.is_empty()
                    }) || self
                        .last_batch
                        .as_ref()
                        .is_some_and(|report| report.done() < report.steps.len())
                    {
                        "Operations / attention"
                    } else {
                        "Operations"
                    })
                    .clicked()
                {
                    self.usage_action("/operations", Via::Button, "");
                    self.operations_open = true;
                    self.request_history();
                }
                if self.operation_busy && ui.button("Cancel work").clicked() {
                    self.usage_action("/cancel-work", Via::Button, "");
                    self.cancellation.store(true, Ordering::Relaxed);
                }
                ui.add(
                    egui::Label::new(RichText::new(&self.status).monospace().small()).truncate(),
                )
                .on_hover_text(&self.status);
            });
        });
    }

    fn actions_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("actions").show(ui, |ui| {
            let zip_label = if self.archive.target == Some(self.active) || self.zip_extract_target().is_some() {
                "Extract ZIP"
            } else {
                "Create ZIP"
            };
            ui.horizontal_wrapped(|ui| {
                for (command, label, hint) in [
                    (Command::Copy, "Copy", "Ctrl Shift C"),
                    (Command::Clone, "Clone", "Ctrl Shift K"),
                    (Command::Move, "Move", "Ctrl Shift M"),
                    (Command::Recycle, "Delete", "Ctrl Shift D"),
                    (Command::Refresh, "Refresh", "Ctrl Shift R"),
                    (Command::Search, "Search", "Ctrl Shift F"),
                    (Command::Rename, "Rename", "Ctrl Shift E"),
                    (Command::NewFolder, "New folder", "Ctrl Shift N"),
                    (Command::CreateZip, zip_label, "Ctrl Shift A"),
                    (Command::OpenWith, "Open with", "Shift Enter"),
                ] {
                    let response = action_button(ui, label, hint, !self.operation_busy && self.plan.is_none());
                    if response.clicked() {
                        self.dispatch(Via::Button, command);
                    }
                    if matches!(command, Command::CopilotCli) {
                        response.on_hover_text("Runs copilot --yolo in this folder. The CLI can act without permission prompts.");
                    }
                }
            });
        });
    }

    fn file_panes(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
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
    }
}

pub(crate) fn action_button(
    ui: &mut egui::Ui,
    label: &str,
    hint: &str,
    enabled: bool,
) -> egui::Response {
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
        + 6.0
        + hint_size.y.max(12.0);
    ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
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
