use eframe::egui::{self, Id, RichText};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};
use tomas_commander::ai::{self, Completion, Config, Job};

pub struct AiPanel {
    pub open: bool,
    config: Config,
    path: Option<PathBuf>,
    config_error: Option<String>,
    prompt: String,
    consent: bool,
    status: String,
    answer: Option<ai::Reply>,
    pending: Option<Receiver<ai::Result<Completion>>>,
    cancel: Arc<AtomicBool>,
    session: Option<SyncSender<SessionJob>>,
    events: Receiver<ai::Result<bool>>,
    event_sender: SyncSender<ai::Result<bool>>,
    cache_status: String,
    previous_focus: Option<Id>,
    was_open: bool,
    submitted: Option<Instant>,
}

struct SessionJob {
    job: Job,
    cancel: Arc<AtomicBool>,
    result: SyncSender<ai::Result<Completion>>,
}

impl AiPanel {
    pub fn new() -> Self {
        let path = ai::config_path();
        let (config, config_error) = match &path {
            Ok(path) if path.exists() => match Config::load(path) {
                Ok(config) => (config, None),
                Err(error) => (Config::default(), Some(error)),
            },
            Ok(_) => (Config::default(), None),
            Err(error) => (Config::default(), Some(error.clone())),
        };
        let (event_sender, events) = mpsc::sync_channel(1);
        Self {
            open: false,
            config,
            path: path.ok(),
            config_error,
            prompt: String::new(),
            consent: false,
            status: "Not connected. Configure and check your Azure CLI identity.".into(),
            answer: None,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            session: None,
            events,
            event_sender,
            cache_status: "No reusable verified session.".into(),
            previous_focus: None,
            was_open: false,
            submitted: None,
        }
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    fn disconnect(&mut self) {
        self.cancel();
        self.session = None;
        self.consent = false;
        self.cache_status =
            "Disconnected; worker termination clears the verified session. No token is persisted."
                .into();
    }

    fn start(&mut self, ctx: &egui::Context, inference: bool) {
        if self.pending.is_some() {
            return;
        }
        let Some(path) = &self.path else {
            self.status = "AI settings path unavailable.".into();
            return;
        };
        if self.config_error.is_some() {
            self.status =
                "Invalid configuration was preserved. Repair the local file before connecting."
                    .into();
            return;
        }
        if let Err(error) = self.config.validate() {
            self.status = error;
            return;
        }
        let prompt = if inference {
            if !self.consent {
                self.status =
                    "Confirm the explicit prompt-only data boundary before sending.".into();
                return;
            }
            if let Err(error) = ai::request(
                &self.config,
                ai::Input::ExplicitPrompt(&self.prompt),
                ai::Route::Primary,
            ) {
                self.status = error;
                return;
            }
            Some(self.prompt.clone())
        } else {
            None
        };
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let job = Job {
            config: self.config.clone(),
            prompt,
            config_path: path.clone(),
            recheck: !inference,
        };
        let context = ctx.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        if self.session.is_none() {
            let (sender, requests) = mpsc::sync_channel::<SessionJob>(1);
            let events = self.event_sender.clone();
            std::thread::spawn(move || {
                let mut session = ai::Session::default();
                loop {
                    match requests.recv_timeout(Duration::from_secs(1)) {
                        Ok(work) => {
                            let result = work
                                .job
                                .config
                                .save(&work.job.config_path)
                                .and_then(|()| session.run(work.job, work.cancel));
                            if result.is_err()
                                && let Err(error) = session.clear()
                            {
                                let _ = events.send(Err(error));
                            }
                            let _ = work.result.send(result);
                            context.request_repaint();
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => match session.expire_idle() {
                            Ok(false) => {}
                            result => {
                                let _ = events.send(result);
                                context.request_repaint();
                            }
                        },
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            if let Err(error) = session.clear() {
                                let _ = events.send(Err(error));
                                context.request_repaint();
                            }
                            break;
                        }
                    }
                }
            });
            self.session = Some(sender);
        }
        let work = SessionJob {
            job,
            cancel,
            result: tx,
        };
        if self
            .session
            .as_ref()
            .is_none_or(|sender| sender.try_send(work).is_err())
        {
            self.disconnect();
            self.status =
                "AI session was unavailable; no request queued. Recheck explicitly.".into();
            return;
        }
        self.pending = Some(rx);
        self.submitted = Some(Instant::now());
        self.answer = None;
        self.status = if inference {
            "Verifying identity and deployment, then sending one bounded request. No retry/fallback."
        } else { "Checking identity, endpoint, deployments and token; no inference request." }.into();
    }

    pub fn poll(&mut self) {
        let result = match &self.pending {
            Some(receiver) => match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("AI worker disconnected; no retry.".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            },
            None => None,
        };
        if let Some(result) = result {
            self.pending = None;
            let elapsed = self
                .submitted
                .take()
                .map(|started| started.elapsed().as_millis() as u64);
            match result {
                Ok(Completion::Connected(connection)) => {
                    self.status = format!(
                        "{} Authentication {} ms / total {} ms / cached {}.",
                        connection.message,
                        connection.authentication_ms,
                        elapsed.unwrap_or(connection.elapsed_ms),
                        connection.cached
                    );
                    self.cache_status = "Verified session: reuse for at most 60 s; cleared after 60 s idle. Logout/account changes require Disconnect or recheck.".into();
                }
                Ok(Completion::Answer(mut answer)) => {
                    if let Some(elapsed) = elapsed {
                        answer.elapsed_ms = elapsed;
                    }
                    self.status = format!(
                        "Completed / {} ms / {} input + {} output tokens / pilot attempt {}/{}",
                        answer.elapsed_ms,
                        answer.input_tokens,
                        answer.output_tokens,
                        answer.attempts,
                        self.config.limits.requests
                    );
                    self.answer = Some(answer);
                }
                Ok(Completion::Error(error)) | Err(error) => {
                    self.disconnect();
                    self.status = error;
                }
            }
        }
        while let Ok(event) = self.events.try_recv() {
            match event {
                Ok(true) => {
                    self.cache_status =
                        "Idle session expired; no reusable identity/token remains.".into()
                }
                Ok(false) => {}
                Err(error) => {
                    self.disconnect();
                    self.status = error;
                }
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            if self.was_open {
                self.restore_focus(ctx);
            }
            return;
        }
        if !self.was_open {
            self.previous_focus = ctx.memory(|memory| memory.focused());
            let id = if self.config.validate().is_ok() {
                Id::new("foundry-prompt")
            } else {
                Id::new(("foundry-config", "Resource name"))
            };
            ctx.memory_mut(|memory| memory.request_focus(id));
            self.was_open = true;
        }
        let busy = self.busy();
        let mut check = false;
        let mut send = false;
        let mut close = false;
        let before = self.config.clone();
        let modal = egui::Modal::new(Id::new("foundry-panel")).show(ctx, |ui| {
            ui.set_width(660.0);
            ui.heading("Foundry / bounded text pilot");
            ui.label("Direct Azure connection. Your prompt only; no files, selections, history, MCP or shell.");
            egui::ScrollArea::vertical().id_salt("foundry-content").max_height(490.0).show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    egui::CollapsingHeader::new("Connection / non-secret local settings")
                        .default_open(self.config.deployment.is_empty()).show(ui, |ui| {
                            if let Some(error) = &self.config_error { ui.label(RichText::new(error).strong()); }
                            for (label, value) in [
                                ("Resource name", &mut self.config.resource),
                                ("Resource group", &mut self.config.resource_group),
                                ("Subscription GUID", &mut self.config.subscription),
                                ("Tenant GUID", &mut self.config.tenant),
                                ("Expected identity", &mut self.config.identity),
                                ("Primary deployment", &mut self.config.deployment),
                                ("Expected primary model", &mut self.config.model),
                                ("Stronger deployment (disabled)", &mut self.config.stronger_deployment                                ),
                                ("Expected stronger model", &mut self.config.stronger_model),
                            ] {
                                ui.horizontal(|ui| {
                                    ui.label(label);
                                    let id = Id::new(("foundry-config", label));
                                    super::app::accessible_text_value(ui.ctx(), id, value);
                                    let response = ui.add(egui::TextEdit::singleline(value).id(id).char_limit(254).desired_width(400.0));
                                    ui.ctx().accesskit_node_builder(response.id, |node| {
                                        node.set_label(label);
                                        node.add_action(egui::accesskit::Action::SetValue);
                                    });
                                });
                            }
                            ui.label("Uses the installed Azure CLI sign-in. The app never requests keys, adds roles, or opens an authentication prompt.");
                            if let Some(path) = &self.path { ui.label(RichText::new(path.display().to_string()).small().weak()); }
                        });
                    egui::CollapsingHeader::new("Pilot safety limits (not performance acceptance)").show(ui, |ui| {
                        for (label, value, max) in [
                            ("Input bytes", &mut self.config.limits.input_bytes, 2048),
                            ("Conservative input tokens", &mut self.config.limits.input_token_ceiling, 3072),
                            ("Output bytes", &mut self.config.limits.output_bytes, 8192),
                        ] {
                            ui.horizontal(|ui| { ui.label(label); ui.add(egui::DragValue::new(value).range(1..=max)); });
                        }
                        ui.horizontal(|ui| { ui.label("Output tokens"); ui.add(egui::DragValue::new(&mut self.config.limits.output_tokens).range(1..=512)); });
                        ui.horizontal(|ui| { ui.label("Lifetime pilot requests"); ui.add(egui::DragValue::new(&mut self.config.limits.requests).range(1..=5)); });
                        ui.horizontal(|ui| { ui.label("Deadline seconds"); ui.add(egui::DragValue::new(&mut self.config.limits.timeout_seconds).range(1..=60)); });
                        ui.label("Single request, no agent loop. Durable attempts include uncertain outcomes. No automatic budget reset.");
                    });
                    check = ui.button("Save and check connection (no inference)").clicked();
                    ui.separator();
                    ui.label(RichText::new("Explicit prompt").strong());
                    let id = Id::new("foundry-prompt");
                    if super::app::accessible_text_value(ui.ctx(), id, &mut self.prompt) { self.consent = false; }
                    let response = ui.add(egui::TextEdit::multiline(&mut self.prompt).id(id)
                        .hint_text("Use harmless text; do not paste credentials, filenames, private code or workplace data.")
                        .char_limit(2048).desired_rows(4).desired_width(f32::INFINITY));
                    if response.changed() { self.consent = false; }
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.set_label("Explicit Foundry prompt");
                        node.add_action(egui::accesskit::Action::SetValue);
                    });
                    ui.checkbox(&mut self.consent, "This prompt contains no secrets, filenames, private code or workplace data; send only this text.");
                    send = ui.add_enabled(self.consent && !self.prompt.trim().is_empty(),
                        egui::Button::new(format!("Send once / {}", self.config.deployment))).clicked();
                });
                ui.separator();
                ui.label(RichText::new(&self.status).strong());
                if let Some(answer) = &self.answer {
                    ui.label(format!("Authentication {} ms / provider {} ms / local guards {} us",
                        answer.authentication_ms, answer.provider_ms, answer.local_us));
                    ui.label(format!("Verified authentication reused: {}", answer.authentication_cached));
                    ui.label(RichText::new("Model answer / untrusted text, never executed").weak());
                    ui.add(egui::Label::new(&answer.text).selectable(true).wrap());
                    ui.label(RichText::new(format!("Request {} / response {}", answer.request_id, answer.response_id)).small().monospace());
                }
            });
            ui.horizontal(|ui| {
                if busy && ui.button("Cancel request").clicked() { self.cancel(); }
                if !busy && ui.button("Disconnect / clear verified session").clicked() { self.disconnect(); }
                close = ui.button(if busy { "Close and cancel / Escape" } else { "Close / Escape" }).clicked();
            });
            ui.label(RichText::new(&self.cache_status).small().weak());
        });
        if before != self.config {
            self.disconnect();
        }
        close |= modal.should_close();
        if close {
            self.disconnect();
            self.open = false;
            self.restore_focus(ctx);
        }
        if check {
            self.start(ctx, false);
        }
        if send {
            self.start(ctx, true);
        }
    }

    fn restore_focus(&mut self, ctx: &egui::Context) {
        ctx.memory_mut(|memory| {
            if let Some(id) = self.previous_focus.take() {
                memory.request_focus(id);
            } else if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        self.was_open = false;
    }
}

impl Drop for AiPanel {
    fn drop(&mut self) {
        self.cancel();
        self.session = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_resubmission_cancel_result_disconnect_and_focus_are_wired() {
        let mut panel = AiPanel::new();
        panel.consent = true;
        let (sender, receiver) = mpsc::sync_channel(1);
        panel.pending = Some(receiver);
        panel.status = "Synthetic pending operation.".into();
        let ctx = egui::Context::default();
        panel.start(&ctx, true);
        assert_eq!(panel.status, "Synthetic pending operation.");
        assert!(panel.busy());
        panel.cancel();
        assert!(panel.cancel.load(Ordering::Acquire));
        sender
            .send(Err("Synthetic cancelled operation; no network call.".into()))
            .unwrap();
        panel.poll();
        assert!(!panel.busy());
        assert!(!panel.consent);
        assert!(panel.status.contains("cancelled"));
        assert!(panel.session.is_none());
        let previous = Id::new("synthetic-previous-focus");
        panel.previous_focus = Some(previous);
        panel.was_open = true;
        panel.restore_focus(&ctx);
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(previous));
        assert!(!panel.was_open);
    }

    #[test]
    fn invalid_configuration_and_unconfirmed_prompts_never_start_a_worker() {
        let mut panel = AiPanel::new();
        panel.open = true;
        panel.config = Config::default();
        panel.config_error = None;
        panel.path = Some(PathBuf::from("unused-owned-test-config"));
        panel.prompt = "synthetic".into();
        panel.consent = true;
        panel.start(&egui::Context::default(), true);
        assert!(!panel.busy());
        assert!(panel.status.contains("Invalid"));
        panel.config = Config {
            resource: "synthetic-foundry".into(),
            resource_group: "synthetic".into(),
            subscription: "00000000-0000-0000-0000-000000000001".into(),
            tenant: "00000000-0000-0000-0000-000000000002".into(),
            identity: "test@example.org".into(),
            deployment: "luna".into(),
            model: "synthetic-model".into(),
            ..Config::default()
        };
        panel.consent = false;
        panel.start(&egui::Context::default(), true);
        assert!(!panel.busy());
        assert!(panel.status.contains("Confirm"));
        panel.consent = true;
        panel.prompt = "Bearer synthetic-secret".into();
        panel.start(&egui::Context::default(), true);
        assert!(!panel.busy());
        assert!(panel.status.contains("credential"));
        assert!(!panel.status.contains("synthetic-secret"));
    }
}
