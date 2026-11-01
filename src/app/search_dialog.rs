use super::*;

pub(super) struct SearchRequest {
    pub(super) generation: u64,
    pub(super) root: PathBuf,
    pub(super) query: String,
    pub(super) intent: Option<search::Intent>,
    #[cfg(feature = "native-search-tests")]
    pub(super) slow_test_walk: bool,
}

pub(super) struct SearchDialog {
    pub(super) pane: usize,
    pub(super) root: PathBuf,
    pub(super) query: String,
    pub(super) focus_query: bool,
    pub(super) busy: bool,
    pub(super) index: usize,
    pub(super) results: Option<search::Results>,
    pub(super) error: Option<String>,
    pub(super) scroll_offset: f32,
    pub(super) ai_assisted: bool,
    pub(super) interpreting: bool,
    pub(super) interpretation: Option<String>,
    /// When the running search started (usage log).
    pub(super) started: Option<Instant>,
    /// The latest finished search, recorded once its result is opened or the dialog moves on.
    pub(super) finished: Option<SearchUsage>,
}

pub(super) struct SearchUsage {
    query: String,
    results: usize,
    duration: Option<Duration>,
    truncated: bool,
    ai: bool,
    outcome: &'static str,
}

impl SearchDialog {
    fn usage(&mut self, results: usize, truncated: bool, outcome: &'static str) -> SearchUsage {
        SearchUsage {
            query: self.query.clone(),
            results,
            duration: self.started.take().map(|started| started.elapsed()),
            truncated,
            ai: self.ai_assisted,
            outcome,
        }
    }
}

fn record_search(log: &UsageLog, root: &Path, usage: SearchUsage, opened: Option<usize>) {
    log.record(|| {
        Event::new("search")
            .with("root", files::display_path(root))
            .with("query", usage.query)
            .with("results", usage.results)
            .with("duration_ms", usage.duration)
            .with("truncated", usage.truncated)
            .with("ai", usage.ai)
            .with("outcome", usage.outcome)
            .with("opened_rank", opened)
    });
}

impl Ledger {
    pub(super) fn poll_search(&mut self) {
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
                            dialog.finished = Some(dialog.usage(0, false, "ambiguous"));
                            dialog.error = Some("Ambiguous prompt. Specify filename concepts, folder terms or a file type. No scan started.".into());
                        }
                        search::IntentStatus::Unsupported => {
                            dialog.busy = false;
                            dialog.finished = Some(dialog.usage(0, false, "unsupported"));
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
                                    dialog.finished = Some(dialog.usage(0, false, "error"));
                                    dialog.error =
                                        Some(format!("Local search was not queued: {error}"));
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    dialog.busy = false;
                    dialog.finished = Some(dialog.usage(0, false, "interpretation_failed"));
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
                    Ok(results) => {
                        dialog.finished =
                            Some(dialog.usage(results.matches.len(), results.truncated, "results"));
                        dialog.results = Some(results);
                    }
                    Err(error) => {
                        dialog.finished = Some(dialog.usage(0, false, "error"));
                        dialog.error = Some(error);
                    }
                }
            }
        }
    }

    pub(super) fn search_dialog_ui(&mut self, ctx: &egui::Context) {
        if let Some(dialog) = &mut self.search_dialog {
            let mut start = false;
            let mut cancel = false;
            let mut close = false;
            let mut activate = None;
            let mut configure = false;
            let modal = egui::Modal::new(Id::new(if dialog.ai_assisted { "palette" } else { "filename-search" })).show(ctx, |ui| {
                ui.set_width(680.0);
                ui.heading(if dialog.ai_assisted { "Ask" } else { "Search files / local" });
                if !dialog.ai_assisted {
                    ui.label(format!("In: {}", files::display_path(&dialog.root)))
                        .on_hover_text(format!("Includes subfolders; matches filenames without case sensitivity. Links are skipped.\nLimits: {} examined items, {} results.", search::MAX_EXAMINED, search::MAX_RESULTS));
                }
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
                                    dialog.results = None;
                                    dialog.error = None;
                                    dialog.interpretation = None;
                                }
                            }
                        });
                    });
                }
                ui.horizontal(|ui| {
                    let edit = if dialog.ai_assisted {
                        egui::TextEdit::multiline(&mut dialog.query).desired_rows(4)
                    } else {
                        egui::TextEdit::singleline(&mut dialog.query)
                    };
                    let response = ui.add_enabled(!dialog.busy, edit.id(query_id).char_limit(2048)
                        .hint_text(if dialog.ai_assisted { "What are you looking for?" } else { "Filename contains..." }).desired_width(480.0));
                    changed |= response.changed();
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.set_label(if dialog.ai_assisted { "AI search prompt" } else { "Recursive filename query" });
                        node.add_action(egui::accesskit::Action::SetValue);
                    });
                    if dialog.focus_query {
                        response.request_focus();
                        dialog.focus_query = false;
                    }
                    start = ui.add_enabled(!dialog.busy && !dialog.query.trim().is_empty(),
                        egui::Button::new("Search")).on_hover_text(if dialog.ai_assisted {
                            format!("Interpret the prompt with Foundry, then search filenames under {}.", files::display_path(&dialog.root))
                        } else {
                            "Search filenames locally.".into()
                        }).clicked();
                });
                if dialog.ai_assisted {
                    if changed {
                        dialog.results = None;
                        dialog.interpretation = None;
                        dialog.error = None;
                    }
                    if dialog.error.is_some() {
                        configure = ui.add_enabled(!dialog.busy, egui::Button::new("AI settings")).clicked();
                    }
                }
                if query_focused_before && enter {
                    start = !dialog.busy && !dialog.query.trim().is_empty();
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
                                .min_size(Vec2::new(ui.available_width(), ROW_HEIGHT)).truncate())
                                .on_hover_text(format!("{}\n{} / score {}", result.relative.display(), result.explanation, result.score));
                            if response.clicked() { dialog.index = index; }
                            if response.double_clicked() { activate = Some((index, result.clone())); }
                            if (down || up) && index == dialog.index { response.request_focus(); }
                        }
                    });
                    dialog.scroll_offset = output.state.offset.y;
                    if activate_enter {
                        activate = results.matches.get(dialog.index).cloned().map(|result| (dialog.index, result));
                    }
                    if ui.add_enabled(!results.matches.is_empty(), egui::Button::new("Show result in pane / Enter")).clicked() {
                        activate = results.matches.get(dialog.index).cloned().map(|result| (dialog.index, result));
                    }
                }
                close = ui.button("Close").on_hover_text("Escape").clicked();
            });
            close |= modal.should_close();
            if cancel || close {
                if dialog.started.is_some() {
                    let usage = dialog.usage(0, false, "cancelled");
                    record_search(&self.usage.log, &dialog.root, usage, None);
                }
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
                if let Some(usage) = dialog.finished.take() {
                    record_search(&self.usage.log, &dialog.root, usage, None);
                }
                dialog.started = Some(Instant::now());
                dialog.index = 0;
                dialog.scroll_offset = 0.0;
                dialog.results = None;
                dialog.error = None;
                dialog.interpretation = None;
                if dialog.ai_assisted {
                    dialog.interpreting = self.ai.start_search(ctx, &dialog.query);
                    dialog.busy = dialog.interpreting;
                    if !dialog.busy {
                        dialog.started = None;
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
                            dialog.started = None;
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
            if let Some((rank, result)) = activate {
                match self.work.try_send(Work::Locate { pane, result }) {
                    Ok(()) => {
                        if let Some(usage) = dialog.finished.take() {
                            record_search(&self.usage.log, &dialog.root, usage, Some(rank));
                        }
                        self.usage.outcome("search", "confirmed");
                        close = true;
                    }
                    Err(error) => {
                        dialog.error = Some(format!("Result activation was not queued: {error}"))
                    }
                }
            }
            if close {
                if let Some(usage) = dialog.finished.take() {
                    record_search(&self.usage.log, &dialog.root, usage, None);
                }
                if dialog.ai_assisted && !configure {
                    self.ai.disconnect();
                }
                self.search_dialog = None;
            }
        }
    }
}
