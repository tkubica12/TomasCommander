use super::*;

pub(super) struct CommandDialog {
    pub(super) pane: usize,
    pub(super) roots: [PathBuf; 2],
    pub(super) prompt: String,
    pub(super) consent: bool,
    pub(super) focus_prompt: bool,
    pub(super) interpreting: bool,
    pub(super) resolving: bool,
    pub(super) resolution: Option<commands::Resolution>,
    pub(super) source: Option<usize>,
    pub(super) destination: Option<usize>,
    pub(super) details: Option<String>,
    pub(super) error: Option<String>,
}

impl Ledger {
    pub(super) fn poll_command_interpretation(&mut self) {
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

    pub(super) fn command_dialog_ui(&mut self, ctx: &egui::Context) {
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
                    egui::Button::new("Interpret")).clicked();
                configure = ui.add_enabled(!busy,
                    egui::Button::new("Connection")).clicked();
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
                ui.label("Review the operation before confirming.");
                if mutation {
                    prepare = ui.add_enabled(dialog.source.is_some() && (!needs_destination || dialog.destination.is_some()),
                        egui::Button::new("Review")).clicked();
                } else {
                    reveal = ui.add_enabled(dialog.source.is_some(), egui::Button::new("Locate")).clicked();
                }
            }
            close = ui.button(if busy { "Cancel" } else { "Close" }).on_hover_text("Escape").clicked();
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
}
