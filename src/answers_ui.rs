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
    answers::{self, Config, Credential, ReadConsent, Retrieved, Source},
    platform,
};

pub struct AnswerPanel {
    pub open: bool,
    config: Config,
    path: Option<PathBuf>,
    question: String,
    key: String,
    consent: bool,
    status: String,
    retrieved: Option<Retrieved>,
    answer: Option<answers::Answer>,
    receipt: Option<ai::Reply>,
    selected: usize,
    link_ids: Vec<Id>,
    focus_sources: bool,
    pending: Option<Receiver<answers::Result<answers::Answered>>>,
    cancel: Arc<AtomicBool>,
    was_open: bool,
    previous_focus: Option<Id>,
    #[cfg(feature = "native-answer-tests")]
    test_mode: bool,
}

impl AnswerPanel {
    pub fn new() -> Self {
        let path = Config::path();
        let (config, status) = match &path {
            Ok(path) if path.exists() => match Config::load(path) {
                Ok(config) => (config, "Disconnected. No source/auth check has run.".into()),
                Err(error) => (Config::default(), error),
            },
            Ok(_) => (
                Config { endpoint: answers::WEB_ENDPOINT.into(), ..Config::default() },
                "Not configured. App-specific source credentials and explicit read consent required.".into(),
            ),
            Err(error) => (Config::default(), error.clone()),
        };
        Self {
            open: false,
            config,
            path: path.ok(),
            question: String::new(),
            key: String::new(),
            consent: false,
            status,
            retrieved: None,
            answer: None,
            receipt: None,
            selected: 0,
            link_ids: Vec::new(),
            focus_sources: false,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            was_open: false,
            previous_focus: None,
            #[cfg(feature = "native-answer-tests")]
            test_mode: false,
        }
    }

    #[cfg(feature = "native-answer-tests")]
    pub fn enable_test(&mut self) {
        self.test_mode = true;
        self.path = None;
        self.config = Config {
            endpoint: answers::WEB_ENDPOINT.into(),
            ..Config::default()
        };
        self.reset_data();
        self.status =
            "TEST ONLY / isolated failure and navigation harness / zero MCP or model requests."
                .into();
    }

    #[cfg(feature = "native-answer-tests")]
    fn test_passages(&mut self) {
        use answers::{Passage, SafeUrl};
        self.retrieved = Some(Retrieved {
            source: Source::PublicWeb,
            passages: vec![
                Passage { id:1, source:Source::PublicWeb, title:"TEST ONLY / MCP transport".into(),
                    url:SafeUrl::inspected("https://modelcontextprotocol.io/specification/2025-06-18/basic/transports").expect("fixed permitted test URL"),
                    text:"MCP uses JSON-RPC to encode messages. This fixture is not a live retrieval.".into(), retrieved_unix_seconds:0 },
                Passage { id:2, source:Source::PublicWeb, title:"TEST ONLY / Work IQ overview".into(),
                    url:SafeUrl::inspected("https://learn.microsoft.com/en-us/microsoft-365/copilot/extensibility/work-iq/mcp/overview").expect("fixed permitted test URL"),
                    text:"The Work IQ MCP server uses Microsoft Entra ID for authentication. This fixture is not a live retrieval.".into(), retrieved_unix_seconds:0 },
            ],
            posts:0,cumulative_posts:0,source_ms:0,local_us:0,
        });
        self.selected = 0;
        self.link_ids.clear();
        self.focus_sources = true;
        self.status = "TEST ONLY / two injected navigation passages / no successful provider or claim-support evidence.".into();
    }

    #[cfg(feature = "native-answer-tests")]
    fn test_pending(&mut self, ctx: &egui::Context) {
        self.retrieved = None;
        self.answer = None;
        self.receipt = None;
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let context = ctx.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        self.pending = Some(receiver);
        self.status = "TEST ONLY / injected pending source failure / zero network requests.".into();
        std::thread::spawn(move || {
            for _ in 0..1500 {
                if cancel.load(Ordering::Acquire) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let _ = sender.send(Err(
                "TEST ONLY / injected source timeout, no network or retry.".into(),
            ));
            context.request_repaint();
        });
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.consent = false;
        self.retrieved = None;
        self.answer = None;
        self.receipt = None;
        self.link_ids.clear();
        self.status = "Cancellation requested; awaiting contained worker termination. Submitted reads may still complete; no retry.".into();
    }

    fn clear_key(&mut self) {
        unsafe {
            for byte in self.key.as_bytes_mut() {
                std::ptr::write_volatile(byte, 0);
            }
        }
        self.key.clear();
    }

    fn reset_data(&mut self) {
        self.consent = false;
        self.question.clear();
        self.retrieved = None;
        self.answer = None;
        self.receipt = None;
        self.selected = 0;
        self.link_ids.clear();
        self.focus_sources = true;
        self.clear_key();
    }

    fn start(&mut self, ctx: &egui::Context) {
        #[cfg(feature = "native-answer-tests")]
        if self.test_mode {
            self.status =
                "TEST ONLY / source/auth entry is disabled; no MCP or inference request.".into();
            return;
        }
        if self.busy() {
            self.status = "A read is pending. Cancel or wait; no concurrent request.".into();
            return;
        }
        if !self.consent {
            self.status =
                "Confirm the exact public question/source read boundary before sending.".into();
            return;
        }
        if !self.key.is_empty() {
            self.status = "Unsaved key input exists. Save it locally before reading; the app will not silently use a different previously saved key.".into();
            self.consent = false;
            return;
        }
        let consent = match ReadConsent::confirm_public(&self.config, &self.question) {
            Ok(consent) => consent,
            Err(error) => {
                self.status = error;
                self.consent = false;
                return;
            }
        };
        let Some(path) = self.path.clone() else {
            self.status = "Source settings path unavailable; nothing sent.".into();
            return;
        };
        if let Err(error) = self.config.save(&path) {
            self.status = error;
            return;
        }
        let ai_path = path.with_file_name("ai-connection.json");
        let ai_config = match ai::Config::load(&ai_path).and_then(|config| {
            ai::validate_answer_authority(&config, &ai_path, &self.question).map(|()| config)
        }) {
            Ok(config) => config,
            Err(error) => {
                self.status = error;
                self.consent = false;
                return;
            }
        };
        let credential = match Credential::load_locally() {
            Ok(credential) => credential,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let config = self.config.clone();
        let question = self.question.clone();
        self.consent = false;
        self.retrieved = None;
        self.answer = None;
        self.receipt = None;
        self.link_ids.clear();
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let context = ctx.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        self.pending = Some(receiver);
        self.status =
            "Approved public pilot: app-side WebIQ, then one exact Sol request with public passages only. No retry or fallback."
                .into();
        std::thread::spawn(move || {
            let result = answers::answer_public(
                config, question, credential, consent, path, cancel, ai_config,
            );
            let _ = sender.send(result);
            context.request_repaint();
        });
    }

    pub fn poll(&mut self) {
        let result = self
            .pending
            .as_ref()
            .and_then(|receiver| match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Source worker disconnected; no retry.".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = result {
            self.pending = None;
            if self.cancel.load(Ordering::Acquire) {
                self.status = "Cancelled; late result discarded. Inspect answer-budget.lock if outcome is uncertain.".into();
                return;
            }
            match result {
                Ok(answered) => {
                    let retrieved = answered.context.retrieved;
                    self.status = if retrieved.passages.is_empty() {
                        "No relevant source passages returned; no factual answer can be supported."
                            .into()
                    } else {
                        format!(
                            "Public pilot / {} passages / source {} ms / local {} us / {} POSTs / cumulative {}/16.",
                            retrieved.passages.len(),
                            retrieved.source_ms,
                            retrieved.local_us,
                            retrieved.posts,
                            retrieved.cumulative_posts
                        )
                    };
                    self.selected = 0;
                    self.link_ids.clear();
                    self.retrieved = Some(retrieved);
                    self.answer = Some(answered.answer);
                    self.receipt = Some(answered.reply);
                }
                Err(error) => self.status = error,
            }
        }
    }

    fn open_selected(&mut self) {
        let result = self
            .retrieved
            .as_ref()
            .and_then(|result| result.passages.get(self.selected))
            .ok_or_else(|| "No inspected source link selected.".to_string())
            .and_then(|passage| platform::open_web_url(&passage.url));
        self.status = match result {
            Ok(()) => "Selected inspected source URL handed to the default browser.".into(),
            Err(error) => error,
        };
    }

    fn check_inference(&mut self) {
        let result = ai::config_path()
            .and_then(|path| ai::Config::load(&path))
            .and_then(|config| {
                ai::validate_answer_authority(&config, &ai::config_path()?, &self.question)
                    .map(|()| config)
            });
        self.status = match result {
            Err(error) => error,
            Ok(config) => format!(
                "Next frozen question authorized: Foundry resource {} / deployment {} / model {}; source {} / vault {}. Source/key still require local setup/confirmation. No request executed.",
                config.resource,
                config.stronger_deployment,
                config.stronger_model,
                self.config.endpoint,
                Credential::VAULT_TARGET
            ),
        };
    }

    fn save_key(&mut self) {
        #[cfg(feature = "native-answer-tests")]
        if self.test_mode {
            self.clear_key();
            self.status = "TEST ONLY / vault read/write disabled; no credential stored.".into();
            return;
        }
        if let Err(error) = self.config.validate() {
            self.clear_key();
            self.status = error;
            return;
        }
        let result = Credential::api_key(std::mem::take(&mut self.key))
            .and_then(|credential| credential.save_locally());
        self.consent = false;
        self.status = match result {
            Ok(()) => "Existing WebIQ key saved in this Windows user's Credential Manager for the exact app/provider target. No network or inference.".into(),
            Err(error) => error,
        };
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            if self.was_open {
                if self.busy() {
                    self.cancel();
                }
                self.clear_key();
                self.consent = false;
                if let Some(id) = self.previous_focus.take() {
                    ctx.memory_mut(|memory| memory.request_focus(id));
                }
                self.was_open = false;
            }
            return;
        }
        if !self.was_open {
            self.previous_focus = ctx.memory(|memory| memory.focused());
            ctx.memory_mut(|memory| memory.request_focus(Id::new("answer-question")));
            self.was_open = true;
        }
        let busy = self.busy();
        let mut start = false;
        let mut open_link = false;
        let mut close = false;
        let link_focus = ctx.memory(|memory| self.link_ids.iter().any(|id| memory.has_focus(*id)));
        if link_focus && let Some(result) = &self.retrieved {
            let mut moved = false;
            if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                self.selected = (self.selected + 1).min(result.passages.len().saturating_sub(1));
                moved = true;
            }
            if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                self.selected = self.selected.saturating_sub(1);
                moved = true;
            }
            if moved && let Some(id) = self.link_ids.get(self.selected) {
                ctx.memory_mut(|memory| memory.request_focus(*id));
                ctx.request_repaint();
            }
            open_link = ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
        }
        let modal = egui::Modal::new(Id::new("connected-answers")).show(ctx, |ui| {
            ui.set_width(680.0);
            ui.heading("Connected answers / read-only");
            #[cfg(feature = "native-answer-tests")]
            if self.test_mode {
                ui.label(RichText::new("TEST ONLY / not a shipping integration or live answer").strong());
                ui.horizontal(|ui| {
                    if ui.add_enabled(!busy, egui::Button::new("Inject navigation passages")).clicked() { self.test_passages(); }
                    if ui.add_enabled(!busy, egui::Button::new("Inject pending failure")).clicked() { self.test_pending(ctx); }
                });
            }
            ui.label("App-side MCP, not chat connectors. Only two owner-approved public questions, official passages and scoped Sol grant; no workplace access.");
            let old = self.config.source;
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.config.source, Source::PublicWeb, "Public web / WebIQ");
                    ui.selectable_value(&mut self.config.source, Source::WorkplaceMail, "Workplace / separate permission");
                });
            });
            if self.config.source != old {
                self.reset_data();
                self.config.endpoint = if self.config.source == Source::PublicWeb {
                    answers::WEB_ENDPOINT.into()
                } else { String::new() };
                self.status = "Mode changed; question, credentials, consent and evidence cleared. No cross-source query derived.".into();
            }
            egui::ScrollArea::vertical().max_height(440.0).show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    ui.collapsing("Nonsecret connection settings", |ui| {
                        ui.label("Saving settings never enables a source or grants access.");
                        for (label, value) in [
                            ("Endpoint", &mut self.config.endpoint),
                            ("Tenant (workplace)", &mut self.config.tenant),
                            ("Client ID (workplace)", &mut self.config.client_id),
                            ("Expected identity (workplace)", &mut self.config.expected_identity),
                        ] {
                            ui.label(label);
                            let id = Id::new(("answer-config", label));
                            let mut changed = crate::app::accessible_text_value(ctx, id, value);
                            let response = ui.add(egui::TextEdit::singleline(value).id(id).desired_width(f32::INFINITY));
                            ctx.accesskit_node_builder(response.id, |node| node.add_action(egui::accesskit::Action::SetValue));
                            changed |= response.changed();
                            if changed { self.consent = false; self.retrieved = None; self.answer = None; self.receipt = None; }
                        }
                        if ui.button("Save").on_hover_text("Save nonsecret settings; no network.").clicked() {
                            self.status = match &self.path {
                                Some(path) => match self.config.save(path) {
                                    Ok(()) => "Settings saved, not connected; no source read/model request.".into(),
                                    Err(error) => error,
                                },
                                None => "Settings path unavailable.".into(),
                            };
                        }
                    });
                    if self.config.source == Source::PublicWeb {
                        ui.horizontal(|ui| {
                            for (index, question) in answers::PILOT_QUESTIONS.iter().enumerate() {
                                if ui.button(format!("Load approved question {}", index+1)).clicked() {
                                    self.question = (*question).into();
                                    self.consent = false;
                                    self.retrieved = None;
                                    self.answer = None;
                                    self.receipt = None;
                                }
                            }
                        });
                        ui.label("Public question only: no private files, workplace content, identifiers or credentials. This is not a privacy classifier.");
                        let id = Id::new("answer-question");
                        let mut changed = crate::app::accessible_text_value(ctx, id, &mut self.question);
                        let response = ui.add(egui::TextEdit::multiline(&mut self.question)
                            .id(id).desired_rows(3).desired_width(f32::INFINITY).hint_text("Ask a public question (1,000 UTF-8 bytes maximum)"));
                        ctx.accesskit_node_builder(response.id, |node| node.add_action(egui::accesskit::Action::SetValue));
                        changed |= response.changed();
                        if changed { self.consent = false; self.retrieved = None; self.answer = None; self.receipt = None; }
                        ui.label("Enter your existing WebIQ key locally, never in chat. Save to this Windows user's Credential Manager, bound to this app and the exact endpoint. No host-cache reuse, new key or cloud change.");
                        let response = ui.add(egui::TextEdit::singleline(&mut self.key)
                            .id(Id::new("answer-memory-key")).password(true).desired_width(f32::INFINITY));
                        if response.changed() { self.consent = false; self.retrieved = None; self.answer = None; self.receipt = None; }
                        if ui.button("Store").on_hover_text("Save the existing WebIQ key in this user's local credential vault.").clicked() { self.save_key(); }
                        ui.label("Read sources uses only that exact app-owned vault target; the key is never shown or written to JSON/logs.");
                        ui.checkbox(&mut self.consent, "Send this public question and public passages to Sol.");
                        ui.collapsing("Limits", |ui| {
                            ui.label("WebIQ: up to 8 requests / 30 seconds.");
                            ui.label("Sol: up to 3 passages / 512 output tokens / 1 attempt.");
                            ui.label("Lifetime: 11 AI attempts. No retries or private data.");
                        });
                    } else {
                        ui.label(RichText::new("BLOCKED / workplace is not enabled").strong());
                        ui.label("Need owner-bound endpoint, delegated OAuth app/tenant/user, named workload/records and data routing. No mail, calendar, Teams or SharePoint reads/writes are authorized here.");
                        let proposed = answers::WorkplaceReadPlan;
                        ui.label(format!("Unapproved adapter proposal only: {} {}", proposed.tool(), proposed.arguments()));
                        ui.label("Documented fetch schema is distinct from the host's workload connectors. No automatic public-web fallback or copying workplace evidence into a public question.");
                    }
                });
                ui.separator();
                ui.label(&self.status);
                if let Some(receipt) = &self.receipt {
                    ui.label(format!("Provider {} ms / auth {} ms / guards {} us / tokens {} + {} / cumulative model {}/11",
                        receipt.provider_ms,receipt.authentication_ms,receipt.local_us,
                        receipt.input_tokens,receipt.output_tokens,receipt.attempts));
                    ui.label(format!("Request {} / response {} / public pilot receipt saved locally",
                        receipt.request_id,receipt.response_id));
                }
                if let Some(answer) = &self.answer {
                    ui.label(RichText::new("Cited public answer / exact inspected excerpts").strong());
                    for claim in &answer.claims { ui.label(format!("\"{}\" [{}]",claim.quote,claim.source_id)); }
                    if answer.evidence_gap { ui.label("Missing evidence: retrieved passages do not establish every requested fact."); }
                    if answer.conflicting_evidence { ui.label("Conflicting evidence: inspect the source passages before deciding."); }
                    ui.label("Local app fact (not a web-source claim): WorkIQ remains disconnected; only a proposed typed fetch/mail-preview boundary exists. No workplace write tool or workplace record access is enabled.");
                }
                if let Some(retrieved) = &self.retrieved {
                    self.link_ids.clear();
                    ui.label(RichText::new("Inspected passages / provenance").strong());
                    ui.label("Tab to sources; Up/Down select; Enter opens. Summaries below are exact first-sentence excerpts, not unverified paraphrases.");
                    for (index, passage) in retrieved.passages.iter().enumerate() {
                        ui.push_id(("answer-source", index), |ui| {
                            let response = ui.add(egui::Button::new(format!("[{}] {}", passage.id, passage.title))
                                .selected(index == self.selected));
                            self.link_ids.push(response.id);
                            if self.focus_sources && index == self.selected {
                                response.request_focus();
                                response.scroll_to_me(Some(egui::Align::Center));
                                self.focus_sources = false;
                            }
                            if response.has_focus() {
                                ui.memory_mut(|memory| memory.set_focus_lock_filter(response.id, egui::EventFilter {
                                    vertical_arrows: true,
                                    ..Default::default()
                                }));
                                if !ui.memory(|memory| memory.had_focus_last_frame(response.id)) {
                                    ctx.request_repaint();
                                }
                                self.selected = index;
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            if response.clicked() { self.selected = index; open_link = true; }
                            ui.label(passage.url.as_str());
                            ui.label(format!("Source excerpt: {}", passage.summary()));
                            ui.collapsing(format!("Passage [{}] / retrieved {}", passage.id, passage.retrieved_unix_seconds), |ui| {
                                ui.label(&passage.text);
                            });
                        });
                    }
                }
            });
            ui.horizontal(|ui| {
                start = ui.add_enabled(!busy && self.config.source == Source::PublicWeb, egui::Button::new("Answer")).on_hover_text("Ctrl Enter").clicked();
                if ui.button("Check").on_hover_text("Check the pilot grant; no model request.").clicked() { self.check_inference(); }
            });
            ui.horizontal(|ui| {
                if self.retrieved.as_ref().is_some_and(|result| !result.passages.is_empty())
                    && ui.button("Sources").clicked() {
                    self.focus_sources = true;
                }
                if busy && ui.button("Cancel").clicked() { self.cancel(); }
                close = ui.button("Close").on_hover_text("Escape").clicked();
            });
        });
        if !busy
            && self.config.source == Source::PublicWeb
            && ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::Enter))
        {
            start = true;
        }
        close |= modal.should_close();
        if close {
            self.open = false;
        }
        if open_link && !close {
            self.open_selected();
        }
        if start && !close {
            self.start(ctx);
        }
    }
}

impl Drop for AnswerPanel {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.clear_key();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "native-answer-tests")]
    #[test]
    fn source_arrows_keep_focus_on_links_not_passage_controls() {
        let mut panel = AnswerPanel::new();
        panel.enable_test();
        panel.open = true;
        panel.test_passages();
        let ctx = egui::Context::default();
        for _ in 0..4 {
            ctx.run_ui(egui::RawInput::default(), |ui| panel.show(ui.ctx()))
                .textures_delta
                .clear();
        }
        assert!(ctx.memory(|memory| memory.has_focus(panel.link_ids[0])));
        for (key, expected) in [(Key::ArrowDown, 1), (Key::ArrowUp, 0)] {
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            };
            ctx.run_ui(input, |ui| panel.show(ui.ctx()))
                .textures_delta
                .clear();
            for _ in 0..2 {
                ctx.run_ui(egui::RawInput::default(), |ui| panel.show(ui.ctx()))
                    .textures_delta
                    .clear();
            }
            assert_eq!(panel.selected, expected);
            assert!(ctx.memory(|memory| memory.has_focus(panel.link_ids[expected])));
        }
    }

    #[test]
    fn mode_change_clears_all_source_data_and_permission() {
        let mut panel = AnswerPanel::new();
        panel.question = "Private mail identifiers must not survive mode changes".into();
        panel.key = "test-only-key".into();
        panel.consent = true;
        panel.reset_data();
        assert!(panel.question.is_empty());
        assert!(panel.key.is_empty());
        assert!(!panel.consent);
        assert!(panel.retrieved.is_none());
    }

    #[cfg(feature = "native-answer-tests")]
    #[test]
    fn source_enter_is_consumed_before_widgets_and_surfaces_missing_source() {
        let mut panel = AnswerPanel::new();
        panel.enable_test();
        panel.open = true;
        panel.test_passages();
        let ctx = egui::Context::default();
        for _ in 0..4 {
            ctx.run_ui(egui::RawInput::default(), |ui| panel.show(ui.ctx()))
                .textures_delta
                .clear();
        }
        assert!(ctx.memory(|memory| memory.has_focus(panel.link_ids[0])));
        // Keep prior source focus but remove its data, so no browser can be opened.
        panel.retrieved.as_mut().unwrap().passages.clear();
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };
        ctx.run_ui(input, |ui| panel.show(ui.ctx()))
            .textures_delta
            .clear();
        assert_eq!(panel.status, "No inspected source link selected.");
    }

    #[test]
    fn missing_consent_configuration_or_key_never_starts_a_worker() {
        let mut panel = AnswerPanel::new();
        panel.path = None;
        let ctx = egui::Context::default();
        panel.start(&ctx);
        assert!(!panel.busy());
        panel.config = Config {
            endpoint: answers::WEB_ENDPOINT.into(),
            ..Config::default()
        };
        panel.question = "Harmless public question".into();
        panel.consent = true;
        panel.start(&ctx);
        assert!(!panel.busy());
        assert!(panel.status.contains("path"));
        panel.config.source = Source::WorkplaceMail;
        panel.start(&ctx);
        assert!(!panel.busy());
        assert!(panel.status.contains("WorkIQ"));
    }

    #[test]
    fn cancelled_or_closed_read_discards_late_source_results() {
        let mut panel = AnswerPanel::new();
        let (sender, receiver) = mpsc::sync_channel(1);
        panel.pending = Some(receiver);
        panel.cancel();
        sender
            .send(Ok(answers::Answered {
                context: answers::PublicContext {
                    question: "TEST ONLY".into(),
                    retrieved: Retrieved {
                        source: Source::PublicWeb,
                        passages: vec![],
                        posts: 0,
                        cumulative_posts: 0,
                        source_ms: 0,
                        local_us: 0,
                    },
                },
                answer: answers::Answer {
                    claims: vec![],
                    evidence_gap: true,
                    conflicting_evidence: false,
                },
                reply: ai::Reply {
                    text: "TEST ONLY".into(),
                    request_id: "no-request".into(),
                    response_id: "no-response".into(),
                    input_tokens: 0,
                    output_tokens: 0,
                    elapsed_ms: 0,
                    authentication_ms: 0,
                    authentication_cached: false,
                    authentication_stages: ai::AuthTimings::default(),
                    provider_ms: 0,
                    local_us: 0,
                    attempts: 0,
                },
            }))
            .unwrap();
        panel.poll();
        assert!(!panel.busy());
        assert!(panel.retrieved.is_none());
        assert!(panel.status.contains("late result"));
    }
}
