use eframe::egui::{self, Id, Key, Modifiers, RichText};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};
use tomas_commander::{
    ai,
    file_agent::{self, Outcome, Request, State, Status},
    file_tools::{Observed, Settings},
    files::{self, Scope},
};

pub struct FileAgentPanel {
    pub open: bool,
    pub pane: usize,
    prompt: String,
    current_folder: PathBuf,
    scope: Scope,
    settings: Option<Settings>,
    pending: Option<Receiver<ai::Result<Outcome>>>,
    progress: Option<Receiver<(String, State)>>,
    cancel: Arc<AtomicBool>,
    outcome: Option<Outcome>,
    snapshot: State,
    status: String,
    error: Option<String>,
    index: usize,
    focus_prompt: bool,
    unread: bool,
    started: Option<std::time::Instant>,
    continuation: bool,
    /// A finished (or refused) task for the usage log; taken by the application.
    pub finished: Option<TaskUsage>,
}

/// Prompt, outcome and size of one AI task; no answer content.
pub struct TaskUsage {
    pub prompt: String,
    pub outcome: &'static str,
    pub calls: u32,
    pub results: usize,
    pub duration_ms: u64,
    pub continuation: bool,
}

pub enum Action {
    Locate {
        pane: usize,
        settings: Box<Settings>,
        record: Observed,
        preview: bool,
        /// Position of the chosen result (0 = first).
        rank: usize,
    },
    Settings,
}

impl FileAgentPanel {
    pub fn new() -> Self {
        Self {
            open: false,
            pane: 0,
            prompt: String::new(),
            current_folder: PathBuf::new(),
            scope: Scope::unrestricted(),
            settings: None,
            pending: None,
            progress: None,
            cancel: Arc::new(AtomicBool::new(false)),
            outcome: None,
            snapshot: State::default(),
            status: String::new(),
            error: None,
            index: 0,
            focus_prompt: false,
            unread: false,
            started: None,
            continuation: false,
            finished: None,
        }
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    pub fn hide(&mut self) {
        self.open = false;
        self.focus_prompt = false;
    }

    pub fn reveal(&mut self) {
        self.open = true;
        self.unread = false;
        self.focus_prompt = false;
    }

    fn notification_label(&self) -> &'static str {
        if self.busy() {
            "Notifications / AI running"
        } else if self.unread {
            "Notifications / 1 unread result"
        } else {
            "Notifications"
        }
    }

    pub fn notification_button(&mut self, ui: &mut egui::Ui) {
        let has_task = self.busy() || self.outcome.is_some();
        let response = crate::app::action_button(ui, "    ", "", has_task);
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_label(self.notification_label());
        });
        let center = response.rect.center();
        let stroke = ui.style().interact(&response).fg_stroke;
        let points = [
            (-7.0, 5.0),
            (-5.0, 2.0),
            (-5.0, -3.0),
            (-3.0, -6.0),
            (3.0, -6.0),
            (5.0, -3.0),
            (5.0, 2.0),
            (7.0, 5.0),
            (-7.0, 5.0),
        ];
        ui.painter().add(egui::Shape::line(
            points
                .iter()
                .map(|&(x, y)| center + egui::vec2(x, y))
                .collect(),
            stroke,
        ));
        ui.painter().line_segment(
            [
                center + egui::vec2(0.0, -8.0),
                center + egui::vec2(0.0, -6.0),
            ],
            stroke,
        );
        ui.painter()
            .circle_filled(center + egui::vec2(0.0, 8.0), 1.5, stroke.color);
        if self.busy() || self.unread {
            let badge = center + egui::vec2(10.0, -9.0);
            ui.painter().circle_filled(
                badge,
                if self.unread { 6.0 } else { 3.0 },
                ui.visuals().selection.bg_fill,
            );
            if self.unread {
                ui.painter().text(
                    badge,
                    egui::Align2::CENTER_CENTER,
                    "1",
                    egui::FontId::proportional(10.0),
                    ui.visuals().selection.stroke.color,
                );
            }
        }
        let response = response.on_hover_text(if !has_task {
            "No AI tasks".to_owned()
        } else if self.busy() {
            format!("{}\nOpen AI progress and partial results", self.status)
        } else {
            "Open the latest AI result".to_owned()
        });
        if response.clicked() {
            self.reveal();
        }
    }

    pub fn prepare(&mut self, prompt: String, current_folder: PathBuf, pane: usize, scope: Scope) {
        self.reveal();
        if self.busy() {
            return;
        }
        self.prompt = prompt;
        self.current_folder = current_folder;
        self.pane = pane;
        self.scope = scope;
        self.outcome = None;
        self.snapshot = State::default();
        self.error = None;
        self.status.clear();
        self.index = 0;
        self.focus_prompt = true;
    }

    pub fn start(&mut self, ctx: &egui::Context, continuation: bool) {
        if self.busy() {
            return;
        }
        let prepared = (|| -> ai::Result<Request> {
            let settings_path = Settings::path()?;
            let settings = Settings::load(&settings_path)?;
            if let Some(root) = self.scope.root() {
                for allowed in &settings.roots {
                    let resolved = self.scope.resolve(&allowed.path)?;
                    if !resolved.starts_with(root) {
                        return Err("AI roots escape this fixture session.".into());
                    }
                }
            }
            let state = if continuation {
                let outcome = self
                    .outcome
                    .as_ref()
                    .ok_or("No run available to continue.")?;
                if !matches!(outcome.status, Status::Capped) {
                    return Err("Only a capped run can be continued.".into());
                }
                if self.settings.as_ref() != Some(&settings) {
                    return Err("Access settings changed; start a new search.".into());
                }
                if serde_json::to_vec(&outcome.state.transcript)
                    .map_err(|_| "Encode continuation failed.")?
                    .len()
                    > file_agent::MAX_CONTEXT
                {
                    return Err("Context is full; narrow the prompt and start a new search.".into());
                }
                Some(outcome.state.clone())
            } else {
                None
            };
            let request = Request {
                settings_path,
                config_path: ai::config_path()?,
                settings: settings.clone(),
                current_folder: self.current_folder.clone(),
                prompt: self.prompt.clone(),
                state,
                reasoning_effort: file_agent::ReasoningEffort::Medium,
            };
            request.validate()?;
            self.settings = Some(settings);
            Ok(request)
        })();
        let request = match prepared {
            Ok(request) => request,
            Err(error) => {
                self.error = Some(error);
                self.finished = Some(TaskUsage {
                    prompt: self.prompt.clone(),
                    outcome: "not_started",
                    calls: 0,
                    results: 0,
                    duration_ms: 0,
                    continuation,
                });
                return;
            }
        };
        self.started = Some(std::time::Instant::now());
        self.continuation = continuation;
        if !continuation {
            self.outcome = None;
            self.snapshot = State::default();
        }
        self.cancel = Arc::new(AtomicBool::new(false));
        self.error = None;
        self.status = "Starting file search".into();
        let cancel = self.cancel.clone();
        let context = ctx.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        let (progress_tx, progress_rx) = mpsc::sync_channel(file_agent::MAX_CALLS as usize * 3 + 6);
        self.pending = Some(rx);
        self.progress = Some(progress_rx);
        self.unread = false;
        self.hide();
        std::thread::spawn(move || {
            let result = file_agent::run(request, cancel, |stage, state| {
                // Retain authentication progress and a whole ten-call segment.
                let _ = progress_tx.try_send((stage, state));
                context.request_repaint();
            });
            let _ = tx.send(result);
            context.request_repaint();
        });
    }

    pub fn poll(&mut self) {
        if let Some(progress) = &self.progress {
            while let Ok((stage, state)) = progress.try_recv() {
                self.status = stage;
                self.snapshot = state;
            }
        }
        let result = match &self.pending {
            Some(receiver) => match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "AI file worker disconnected; partial results retained.".into(),
                )),
            },
            None => None,
        };
        if let Some(result) = result {
            self.pending = None;
            self.progress = None;
            match result {
                Ok(mut outcome) => {
                    if self.cancel.load(Ordering::Acquire) {
                        outcome.status = Status::Cancelled;
                        outcome.summary = "Cancelled; known results retained.".into();
                    }
                    self.snapshot = outcome.state.clone();
                    self.outcome = Some(outcome);
                }
                Err(error) => {
                    let status = if self.cancel.load(Ordering::Acquire) {
                        Status::Cancelled
                    } else {
                        Status::Failed
                    };
                    self.error = Some(error.clone());
                    self.outcome = Some(file_agent::outcome(
                        status,
                        error,
                        &self.snapshot,
                        std::time::Instant::now(),
                    ));
                }
            }
            if let Some(outcome) = &self.outcome {
                self.finished = Some(TaskUsage {
                    prompt: self.prompt.clone(),
                    outcome: match outcome.status {
                        Status::Done => "completed",
                        Status::Clarify => "clarify",
                        Status::Capped => "capped",
                        Status::Failed => "failed",
                        Status::Cancelled => "cancelled",
                    },
                    calls: outcome.state.calls,
                    results: outcome.files.len(),
                    duration_ms: self.started.take().map_or(outcome.elapsed_ms, |started| {
                        started.elapsed().as_millis() as u64
                    }),
                    continuation: self.continuation,
                });
            }
            self.unread = !self.open;
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<Action> {
        if !self.open {
            return None;
        }
        if crate::app::consume_prompt_key(ctx, Modifiers::CTRL, Key::Space) {
            self.hide();
            return None;
        }
        let mut submit = false;
        let mut resume = false;
        let mut close = false;
        let mut cancel = false;
        let mut action = None;
        let busy = self.busy();
        let modal = egui::Modal::new(Id::new("file-agent")).show(ctx, |ui| {
            ui.set_width(720.0);
            ui.heading("Ask");
            let id = Id::new("file-agent-prompt");
            let query_focused = ui.memory(|memory| memory.has_focus(id));
            let enter = crate::app::consume_prompt_key(ctx, Modifiers::NONE, Key::Enter);
            let preview = crate::app::consume_prompt_key(ctx, Modifiers::CTRL, Key::Enter);
            let down = crate::app::consume_prompt_key(ctx, Modifiers::NONE, Key::ArrowDown);
            let up = crate::app::consume_prompt_key(ctx, Modifiers::NONE, Key::ArrowUp);
            let accessible_changed =
                !busy && crate::app::accessible_text_value(ctx, id, &mut self.prompt);
            let response = ui.add_enabled(
                !busy,
                egui::TextEdit::multiline(&mut self.prompt)
                    .id(id)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .char_limit(8192)
                    .hint_text("What are you looking for?"),
            );
            ctx.accesskit_node_builder(response.id, |node| {
                node.set_label("AI file prompt");
                node.add_action(egui::accesskit::Action::SetValue);
            });
            if self.focus_prompt {
                response.request_focus();
                self.focus_prompt = false;
            }
            if response.changed() || accessible_changed {
                self.outcome = None;
                self.error = None;
                self.snapshot = State::default();
            }
            ui.horizontal(|ui| {
                submit = ui
                    .add_enabled(
                        !busy && !self.prompt.trim().is_empty(),
                        egui::Button::new("Search"),
                    )
                    .clicked()
                    || enter && query_focused && !busy;
                if busy {
                    ui.spinner();
                    ui.label(&self.status);
                    cancel = ui.button("Cancel").on_hover_text("Escape").clicked();
                }
                if self
                    .outcome
                    .as_ref()
                    .is_some_and(|outcome| matches!(outcome.status, Status::Capped))
                {
                    resume = ui
                        .add_enabled(!busy, egui::Button::new("Continue"))
                        .clicked();
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
                if ui.button("AI settings").clicked() {
                    action = Some(Action::Settings);
                    close = true;
                }
            }
            if let Some(outcome) = &self.outcome {
                if self.error.as_deref() != Some(outcome.summary.as_str()) {
                    ui.label(&outcome.summary);
                }
                ui.label(
                    RichText::new(format!(
                        "{} files / {} calls / {} input + {} output tokens",
                        outcome.files.len(),
                        outcome.state.calls,
                        outcome.state.input_tokens,
                        outcome.state.output_tokens
                    ))
                    .small(),
                )
                .on_hover_text(format!(
                    "{} examined / {} skipped{}\n{}",
                    outcome.state.examined,
                    outcome.state.skipped,
                    if outcome.state.truncated {
                        " / incomplete traversal"
                    } else {
                        ""
                    },
                    outcome.state.errors.join("\n")
                ));
            }
            let results = self
                .outcome
                .as_ref()
                .map(|outcome| outcome.files.as_slice())
                .unwrap_or(&self.snapshot.observed);
            self.index = self.index.min(results.len().saturating_sub(1));
            if (down || up) && !results.is_empty() {
                self.index = if query_focused {
                    0
                } else if down {
                    (self.index + 1).min(results.len() - 1)
                } else {
                    self.index.saturating_sub(1)
                };
                ui.memory_mut(|memory| {
                    memory.surrender_focus(id);
                    memory.move_focus(egui::FocusDirection::None);
                });
            }
            let mut selected = None;
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .id_salt("agent-results")
                .show_rows(ui, 29.0, results.len(), |ui, range| {
                    for index in range {
                        let file = &results[index];
                        let response = ui
                            .add(
                                egui::Button::new(format!(
                                    "{} {}",
                                    if file.directory { "[/]" } else { "[.]" },
                                    files::display_path(&file.path)
                                ))
                                .selected(index == self.index)
                                .truncate()
                                .min_size(egui::vec2(ui.available_width(), 29.0)),
                            )
                            .on_hover_text(&file.reason);
                        if response.clicked() {
                            self.index = index;
                        }
                        if response.double_clicked() {
                            selected = Some((index, false));
                        }
                        if (down || up) && index == self.index {
                            response.request_focus();
                            response.scroll_to_me(None);
                        }
                    }
                });
            if !query_focused && enter || preview {
                selected = Some((self.index, preview));
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!results.is_empty(), egui::Button::new("Locate"))
                    .on_hover_text("Enter")
                    .clicked()
                {
                    selected = Some((self.index, false));
                }
                if ui
                    .add_enabled(
                        results.get(self.index).is_some_and(|file| !file.directory),
                        egui::Button::new("Preview"),
                    )
                    .on_hover_text("Ctrl Enter")
                    .clicked()
                {
                    selected = Some((self.index, true));
                }
            });
            if let Some((index, preview)) = selected
                && let Some(record) = results.get(index)
                && let Some(settings) = &self.settings
            {
                action = Some(Action::Locate {
                    pane: self.pane,
                    settings: Box::new(settings.clone()),
                    record: record.clone(),
                    preview,
                    rank: index,
                });
                close = true;
            }
            close |= ui.button(if busy { "Hide" } else { "Close" }).clicked();
        });
        if modal.should_close() {
            if busy {
                cancel = true;
            } else {
                close = true;
            }
        }
        if cancel {
            self.cancel();
            self.status = "Cancelling; known results will remain available".into();
        }
        if close {
            self.hide();
        }
        if submit {
            self.start(ctx, false);
        }
        if resume {
            self.start(ctx, true);
        }
        action
    }
}

impl Drop for FileAgentPanel {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> FileAgentPanel {
        let mut panel = FileAgentPanel::new();
        panel.prepare(
            "Synthetic prompt".into(),
            PathBuf::from(r"D:\synthetic"),
            1,
            Scope::unrestricted(),
        );
        panel.settings = Some(Settings {
            version: 1,
            roots: vec![],
            connection: ai::Config::default(),
            allow_content: true,
            allow_cloud_download: false,
        });
        panel.snapshot.observed = (0..2)
            .map(|index| Observed {
                path: PathBuf::from(format!(r"D:\synthetic\{index}.txt")),
                root: PathBuf::from(r"D:\synthetic"),
                directory: false,
                size: 1,
                modified: None,
                identity: (1, index),
                reason: "Synthetic observation".into(),
            })
            .collect();
        panel
    }

    fn frame(
        panel: &mut FileAgentPanel,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> Option<Action> {
        let mut action = None;
        ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |_| action = panel.show(ctx),
        )
        .textures_delta
        .clear();
        action
    }

    fn key(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn cancellation_keeps_partial_results_and_relabels_late_completion() {
        let mut panel = panel();
        let (tx, rx) = mpsc::sync_channel(1);
        panel.pending = Some(rx);
        let outcome = file_agent::outcome(
            Status::Done,
            "Late completion".into(),
            &panel.snapshot,
            std::time::Instant::now(),
        );
        tx.send(Ok(outcome)).unwrap();
        panel.cancel();
        panel.poll();
        assert!(!panel.busy());
        let outcome = panel.outcome.as_ref().unwrap();
        assert!(matches!(outcome.status, Status::Cancelled));
        assert_eq!(outcome.files.len(), 2);
        assert!(panel.open);
    }

    #[test]
    fn hidden_runs_keep_working_and_notify_without_opening_a_modal() {
        for status in [Status::Done, Status::Capped] {
            let mut panel = panel();
            let (tx, rx) = mpsc::sync_channel(1);
            panel.pending = Some(rx);
            panel.hide();
            assert!(panel.busy());
            assert!(!panel.cancel.load(Ordering::Acquire));
            assert_eq!(panel.notification_label(), "Notifications / AI running");
            let ctx = egui::Context::default();
            assert!(frame(&mut panel, &ctx, vec![]).is_none());
            assert!(!panel.open);
            tx.send(Ok(file_agent::outcome(
                status,
                "Ready".into(),
                &panel.snapshot,
                std::time::Instant::now(),
            )))
            .unwrap();
            panel.poll();
            assert!(!panel.open);
            assert!(!panel.busy());
            assert_eq!(
                panel.notification_label(),
                "Notifications / 1 unread result"
            );
            assert_eq!(panel.outcome.as_ref().unwrap().files.len(), 2);
            panel.reveal();
            assert!(panel.open);
            assert!(!panel.unread);
            assert_eq!(panel.notification_label(), "Notifications");
        }
    }

    #[test]
    fn hidden_failure_notifies_and_an_active_task_cannot_be_replaced() {
        let mut panel = panel();
        let (tx, rx) = mpsc::sync_channel(1);
        panel.pending = Some(rx);
        panel.prepare(
            "Do not replace".into(),
            PathBuf::from(r"D:\other"),
            0,
            Scope::unrestricted(),
        );
        assert_eq!(panel.prompt, "Synthetic prompt");
        assert_eq!(panel.pane, 1);
        assert_eq!(panel.current_folder, PathBuf::from(r"D:\synthetic"));
        assert!(panel.error.is_none());
        panel.hide();
        tx.send(Err("Synthetic worker failure".into())).unwrap();
        panel.poll();
        assert!(!panel.open);
        assert!(panel.unread);
        assert_eq!(panel.error.as_deref(), Some("Synthetic worker failure"));
        assert_eq!(panel.outcome.as_ref().unwrap().files.len(), 2);
    }

    #[test]
    fn ctrl_space_hides_active_results_without_cancelling() {
        let mut panel = panel();
        let (tx, rx) = mpsc::sync_channel(1);
        panel.pending = Some(rx);
        let ctx = egui::Context::default();
        frame(&mut panel, &ctx, vec![key(Key::Space, Modifiers::CTRL)]);
        assert!(!panel.open);
        assert!(panel.busy());
        assert!(!panel.cancel.load(Ordering::Acquire));
        drop(tx);
    }

    #[test]
    fn disconnect_is_explicit_and_preserves_latest_progress() {
        let mut panel = panel();
        let (tx, rx) = mpsc::sync_channel(1);
        panel.pending = Some(rx);
        let (progress_tx, progress_rx) = mpsc::sync_channel(30);
        panel.progress = Some(progress_rx);
        let mut snapshot = panel.snapshot.clone();
        snapshot.calls = 10;
        progress_tx
            .send(("Last progress".into(), snapshot))
            .unwrap();
        drop(tx);
        panel.poll();
        assert!(panel.error.as_ref().unwrap().contains("disconnected"));
        let outcome = panel.outcome.as_ref().unwrap();
        assert!(matches!(outcome.status, Status::Failed));
        assert_eq!(outcome.files.len(), 2);
        assert_eq!(outcome.state.calls, 10);
    }

    #[test]
    fn arrows_transfer_prompt_focus_and_enter_locates_or_previews() {
        for preview in [false, true] {
            let mut panel = panel();
            let ctx = egui::Context::default();
            frame(&mut panel, &ctx, vec![]);
            frame(&mut panel, &ctx, vec![]);
            frame(&mut panel, &ctx, vec![key(Key::ArrowDown, Modifiers::NONE)]);
            assert_eq!(panel.index, 0);
            frame(&mut panel, &ctx, vec![key(Key::ArrowDown, Modifiers::NONE)]);
            assert_eq!(panel.index, 1);
            let action = frame(
                &mut panel,
                &ctx,
                vec![key(
                    Key::Enter,
                    if preview {
                        Modifiers::CTRL
                    } else {
                        Modifiers::NONE
                    },
                )],
            );
            let Some(Action::Locate {
                pane,
                record,
                preview: actual,
                ..
            }) = action
            else {
                panic!("Expected existing result activation");
            };
            assert_eq!(pane, 1);
            assert_eq!(record.path, PathBuf::from(r"D:\synthetic\1.txt"));
            assert_eq!(actual, preview);
            assert!(!panel.open);
            assert!(!panel.busy());
        }
    }

    #[test]
    fn accessible_prompt_change_invalidates_capped_state_and_results() {
        let mut panel = panel();
        panel.outcome = Some(file_agent::outcome(
            Status::Capped,
            "Previous search".into(),
            &panel.snapshot,
            std::time::Instant::now(),
        ));
        let ctx = egui::Context::default();
        frame(&mut panel, &ctx, vec![]);
        frame(
            &mut panel,
            &ctx,
            vec![egui::Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::SetValue,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    target_node: Id::new("file-agent-prompt").accesskit_id(),
                    data: Some(egui::accesskit::ActionData::Value(
                        "New synthetic search".into(),
                    )),
                },
            )],
        );
        assert_eq!(panel.prompt, "New synthetic search");
        assert!(panel.outcome.is_none());
        assert!(panel.snapshot.observed.is_empty());
        assert!(!panel.busy());
    }

    #[test]
    fn shift_enter_does_not_submit_and_escape_cancels_before_closing() {
        let mut panel = panel();
        let ctx = egui::Context::default();
        frame(&mut panel, &ctx, vec![]);
        frame(&mut panel, &ctx, vec![]);
        assert!(frame(&mut panel, &ctx, vec![key(Key::Enter, Modifiers::SHIFT)]).is_none());
        assert!(!panel.busy());
        assert!(panel.prompt.ends_with('\n'));
        let (tx, rx) = mpsc::sync_channel(1);
        panel.pending = Some(rx);
        frame(&mut panel, &ctx, vec![key(Key::Escape, Modifiers::NONE)]);
        assert!(panel.cancel.load(Ordering::Acquire));
        assert!(panel.open);
        drop(tx);
        panel.poll();
        frame(&mut panel, &ctx, vec![key(Key::Escape, Modifiers::NONE)]);
        assert!(!panel.open);
    }
}
