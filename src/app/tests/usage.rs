use super::*;
use eframe::App as _;
use std::fs;
use tomas_commander::usage::{self, Captured, UsageLog};

/// A real `Ledger` over an owned fixture folder, driven frame by frame.
struct Harness {
    app: Ledger,
    ctx: egui::Context,
    frame: eframe::Frame,
    events: Captured,
    root: PathBuf,
    files: PathBuf,
    time: f64,
}

impl Harness {
    fn new(name: &str) -> Self {
        Self::launch(name, UsageLaunch::default(), true)
    }

    fn launch(name: &str, launch: UsageLaunch, capture: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "tc-usage-app-{name}-{}-{}",
            std::process::id(),
            usage::now_ms()
        ));
        let files = root.join("files");
        fs::create_dir_all(files.join("Gamma")).unwrap();
        fs::write(files.join("alpha.txt"), b"alpha").unwrap();
        fs::write(files.join("beta.md"), b"# beta").unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let creation = eframe::CreationContext::_new_kittest(ctx.clone());
        let scope = Scope::fixture(&files).unwrap();
        let initial = scope.resolve(&files).unwrap();
        let files = initial.clone();
        let mut app = Ledger::new(
            &creation,
            initial,
            scope,
            root.join("preferences.txt"),
            Instant::now(),
            false,
            launch,
        );
        if !launch.allowed {
            assert!(!app.usage.log.is_enabled(), "logging needs an opt-in");
            assert!(app.usage.log.folder().is_none());
        }
        let (log, events) = UsageLog::memory();
        if capture {
            app.usage.log = log;
        }
        let mut harness = Self {
            app,
            ctx,
            frame: eframe::Frame::_new_kittest(),
            events,
            root,
            files,
            time: 0.0,
        };
        harness.until(|app| app.panes.iter().all(|pane| !pane.busy));
        harness
    }

    fn run(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.5;
        let app = &mut self.app;
        let frame = &mut self.frame;
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1240.0, 800.0),
                )),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ui| {
                app.logic(ui.ctx(), frame);
                app.ui(ui, frame);
            },
        );
        output.textures_delta.clear();
        output
    }

    fn until(&mut self, done: impl Fn(&Ledger) -> bool) {
        for _ in 0..300 {
            self.run(vec![]);
            if done(&self.app) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the application did not reach the expected state");
    }

    fn take(&self) -> Vec<usage::Event> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }

    fn kinds(events: &[usage::Event], kind: &str) -> Vec<usage::Event> {
        events
            .iter()
            .filter(|event| event.kind == kind)
            .cloned()
            .collect()
    }

    /// Clicks the first accessible button with `label` through AccessKit.
    fn click(&mut self, label: &str) {
        let output = self.run(vec![]);
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let (id, _) = nodes
            .iter()
            .find(|(_, node)| {
                node.role() == egui::accesskit::Role::Button && node.label() == Some(label)
            })
            .unwrap_or_else(|| panic!("no {label} button"));
        let request = egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: *id,
            data: None,
        };
        self.run(vec![egui::Event::AccessKitActionRequest(request)]);
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn key(key: Key, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn release(key: Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

fn text(event: &usage::Event, field: &str) -> String {
    event.text(field).unwrap_or_default().to_owned()
}

fn unhandled(harness: &Harness) -> Vec<String> {
    Harness::kinds(&harness.take(), "key_unhandled")
        .iter()
        .map(|event| text(event, "key"))
        .collect()
}

#[test]
fn shortcuts_and_buttons_record_how_a_command_was_invoked() {
    let mut harness = Harness::new("via");
    harness.take();
    harness.run(vec![key(Key::S, Modifiers::CTRL | Modifiers::SHIFT)]);
    harness.click("Refresh");
    let events = harness.take();
    let commands = Harness::kinds(&events, "command");
    let sort = commands
        .iter()
        .find(|event| text(event, "cmd") == "/sort")
        .expect("Ctrl+Shift+S records /sort");
    assert_eq!(text(sort, "via"), "shortcut");
    assert_eq!(text(sort, "shortcut"), "Ctrl Shift S");
    assert_eq!(sort.field("ok"), Some(&usage::Value::Bool(true)));
    assert_eq!(text(sort, "surface"), "files");
    let refresh = commands
        .iter()
        .find(|event| text(event, "cmd") == "/refresh")
        .expect("the Refresh button records /refresh");
    assert_eq!(text(refresh, "via"), "button");
    assert!(
        Harness::kinds(&events, "key_unhandled").is_empty(),
        "handled chords are not unhandled: {events:?}"
    );
    assert!(
        Harness::kinds(&events, "navigate").is_empty(),
        "refreshing the same folder is not navigation"
    );

    let gamma = harness.app.panes[0]
        .entries
        .iter()
        .find(|entry| entry.name == "Gamma")
        .unwrap()
        .path
        .clone();
    harness.app.panes[0].focused = Some(gamma.clone());
    harness.run(vec![key(Key::Enter, Modifiers::NONE)]);
    let navigation = Harness::kinds(&harness.take(), "navigate");
    assert_eq!(navigation.len(), 1, "{navigation:?}");
    assert_eq!(text(&navigation[0], "via"), "enter");
    assert_eq!(text(&navigation[0], "to"), files::display_path(&gamma));
    assert_eq!(
        text(&navigation[0], "from"),
        files::display_path(&harness.files)
    );
    harness.until(|app| !app.panes[0].busy);
    harness.run(vec![key(Key::Backspace, Modifiers::NONE)]);
    let events = harness.take();
    let back = Harness::kinds(&events, "navigate");
    assert_eq!(text(&back[0], "via"), "backspace");
    let parent = Harness::kinds(&events, "command");
    assert_eq!(text(&parent[0], "cmd"), "/parent");
    assert_eq!(text(&parent[0], "via"), "shortcut");
}

#[test]
fn unhandled_chords_are_recorded_and_plain_typing_is_only_counted() {
    let mut harness = Harness::new("keys");
    harness.take();
    harness.run(vec![
        key(Key::F5, Modifiers::CTRL),
        key(Key::F8, Modifiers::NONE),
        egui::Event::ModifiersChanged(Modifiers::CTRL),
        egui::Event::Copy,
        egui::Event::ModifiersChanged(Modifiers::NONE),
    ]);
    let unhandled = Harness::kinds(&harness.take(), "key_unhandled");
    let keys: Vec<_> = unhandled.iter().map(|event| text(event, "key")).collect();
    assert_eq!(keys, ["Ctrl+F5", "F8", "Ctrl+C"]);
    assert!(unhandled.iter().all(|event| text(event, "focus") == "pane"));

    harness.run(vec![
        key(Key::A, Modifiers::NONE),
        egui::Event::Text("a".into()),
    ]);
    harness.run(vec![
        key(Key::L, Modifiers::NONE),
        egui::Event::Text("l".into()),
        key(Key::A, Modifiers::CTRL),
    ]);
    assert_eq!(harness.app.panes[0].filter, "al");
    let events = harness.take();
    assert!(
        Harness::kinds(&events, "key_unhandled").is_empty(),
        "typing and text-editing chords are not unhandled: {events:?}"
    );
    assert!(
        events
            .iter()
            .all(|event| !event.fields.iter().any(|(_, value)| value
                .as_str()
                .is_some_and(|text| text == "a" || text == "l"))),
        "no per-key text is logged: {events:?}"
    );
    assert_eq!(harness.app.usage.counters.get("typed", "pane"), 1);
    assert_eq!(harness.app.usage.counters.get("typed", "filter"), 1);
}

#[test]
fn a_held_clipboard_chord_is_one_unhandled_key_with_its_real_chord() {
    let mut harness = Harness::new("clipboard");
    harness.take();
    // Windows' Shift+Delete and Shift+Insert arrive as bare Cut and Paste events.
    harness.run(vec![
        egui::Event::ModifiersChanged(Modifiers::SHIFT),
        egui::Event::Cut,
    ]);
    harness.run(vec![release(Key::Delete)]);
    harness.run(vec![egui::Event::Paste("clipboard".into())]);
    harness.run(vec![
        release(Key::Insert),
        egui::Event::ModifiersChanged(Modifiers::NONE),
    ]);
    assert_eq!(unhandled(&harness), ["Shift+Delete", "Shift+Insert"]);

    // Auto-repeat sends one Copy per repeat and no repeat flag.
    harness.run(vec![
        egui::Event::ModifiersChanged(Modifiers::CTRL),
        egui::Event::Copy,
    ]);
    for _ in 1..30 {
        harness.run(vec![egui::Event::Copy]);
    }
    assert_eq!(unhandled(&harness), ["Ctrl+C"]);
    harness.run(vec![release(Key::C)]);
    harness.run(vec![egui::Event::Copy]);
    harness.run(vec![egui::Event::Copy]);
    assert_eq!(
        unhandled(&harness),
        ["Ctrl+C"],
        "releasing C ends the press"
    );
    harness.run(vec![egui::Event::ModifiersChanged(Modifiers::NONE)]);
    harness.run(vec![
        egui::Event::ModifiersChanged(Modifiers::CTRL),
        egui::Event::Copy,
    ]);
    harness.run(vec![egui::Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(
        unhandled(&harness),
        ["Ctrl+C"],
        "releasing Ctrl ends the press"
    );
}

#[test]
fn logic_ticks_while_hidden_do_not_count_the_last_input_again() {
    let mut harness = Harness::new("hidden");
    harness.take();
    let pos = egui::Pos2::new(100.0, 25.0);
    let click = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    harness.run(vec![
        egui::Event::PointerMoved(pos),
        click(true),
        click(false),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        },
        key(Key::F8, Modifiers::NONE),
    ]);
    assert_eq!(harness.app.usage.counters.get("pointer", "primary"), 1);
    assert_eq!(harness.app.usage.counters.get("pointer", "wheel"), 1);
    assert_eq!(harness.app.usage.counters.get("key", "F8"), 1);
    harness.take();
    let counters = harness.app.usage.counters.clone();
    let activity = harness.app.usage.activity.clone();
    // A minimized window gets `logic` without a pass, over the last shown pass's input.
    let mut hidden = egui::RawInput::default();
    hidden.viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo {
            minimized: Some(true),
            ..Default::default()
        },
    );
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(2));
        let (app, frame) = (&mut harness.app, &mut harness.frame);
        let _ = harness.ctx.run_logic(&hidden, |ctx| app.logic(ctx, frame));
    }
    assert!(
        harness.ctx.input(|input| input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::PointerButton { .. }))),
        "the hidden ticks still see the last pass's events"
    );
    assert_eq!(harness.app.usage.counters, counters);
    assert_eq!(
        harness.app.usage.activity, activity,
        "no minimized active time"
    );
    assert!(unhandled(&harness).is_empty());

    harness.run(vec![click(true), click(false)]);
    assert_eq!(harness.app.usage.counters.get("pointer", "primary"), 2);
    assert_ne!(harness.app.usage.activity, activity);
}

#[test]
fn a_failed_new_folder_leaves_no_pending_operation_behind() {
    let mut harness = Harness::new("new-folder");
    harness.take();
    harness.app.name_dialog = Some(NameDialog {
        folder: harness.files.clone(),
        source: None,
        name: "Gamma".into(),
        focus: false,
        error: None,
        inline: false,
        pane: 0,
    });
    harness.app.prepare_name_change();
    assert!(harness.app.usage.operation.is_some());
    harness.until(|app| !app.operation_busy);
    assert!(
        harness
            .app
            .error
            .as_deref()
            .is_some_and(|error| error.contains("already exists")),
        "{:?}",
        harness.app.error
    );
    assert!(
        harness.app.usage.operation.is_none(),
        "the failed new folder is not pending"
    );
    harness.app.error = None;
    harness.run(vec![]);
    harness.take();

    // Later slow preparation is still reported.
    harness.app.usage.last_command = "/copy";
    harness.app.operation_busy = true;
    harness.run(vec![]);
    thread::sleep(Duration::from_millis(550));
    harness.app.operation_busy = false;
    harness.run(vec![]);
    let slow = Harness::kinds(&harness.take(), "slow_action");
    assert_eq!(slow.len(), 1, "{slow:?}");
    assert_eq!(text(&slow[0], "cmd"), "/copy");
}

#[test]
fn a_filter_session_produces_one_event_when_it_ends() {
    let mut harness = Harness::new("filter");
    harness.take();
    harness.run(vec![egui::Event::Text("a".into())]);
    harness.run(vec![egui::Event::Text("l".into())]);
    harness.run(vec![]);
    assert!(Harness::kinds(&harness.take(), "filter").is_empty());
    harness.run(vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(harness.app.panes[0].filter.is_empty());
    let events = harness.take();
    let filters = Harness::kinds(&events, "filter");
    assert_eq!(filters.len(), 1, "{events:?}");
    assert_eq!(text(&filters[0], "query"), "al");
    assert_eq!(text(&filters[0], "outcome"), "cleared");
    assert_eq!(
        filters[0].field("matches").and_then(usage::Value::as_i64),
        Some(1)
    );
    assert!(Harness::kinds(&events, "key_unhandled").is_empty());
}

#[test]
fn ask_records_unmet_text_and_dialogs_record_outcomes() {
    let mut harness = Harness::new("ask");
    harness.take();
    harness.run(vec![key(Key::Space, Modifiers::CTRL)]);
    assert!(harness.app.palette);
    harness.run(vec![]);
    harness.run(vec![egui::Event::Text("open my notes".into())]);
    assert_eq!(harness.app.palette_query, "open my notes");
    harness.run(vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(!harness.app.palette);
    harness.run(vec![]);
    let events = harness.take();
    let opened = Harness::kinds(&events, "command");
    assert_eq!(text(&opened[0], "cmd"), "/ask");
    assert_eq!(text(&opened[0], "via"), "shortcut");
    let asks = Harness::kinds(&events, "ask");
    assert_eq!(asks.len(), 1, "{events:?}");
    assert_eq!(text(&asks[0], "text"), "open my notes");
    assert_eq!(text(&asks[0], "outcome"), "cancelled");
    assert_eq!(
        asks[0].field("candidates").and_then(usage::Value::as_i64),
        Some(0)
    );

    harness.click("Settings");
    assert!(harness.app.settings_open);
    harness.run(vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(!harness.app.settings_open);
    harness.run(vec![]);
    let events = harness.take();
    let dialogs = Harness::kinds(&events, "dialog");
    assert_eq!(dialogs.len(), 1, "{events:?}");
    assert_eq!(text(&dialogs[0], "name"), "settings");
    assert_eq!(text(&dialogs[0], "outcome"), "closed");
    assert!(
        Harness::kinds(&events, "key_unhandled").is_empty(),
        "Escape closing a dialog is handled: {events:?}"
    );
    let settings = Harness::kinds(&events, "command");
    assert_eq!(text(&settings[0], "cmd"), "/settings");
    assert_eq!(text(&settings[0], "via"), "button");
}

#[test]
fn an_opted_in_run_writes_its_session_to_the_usage_folder() {
    let mut harness = Harness::launch(
        "session",
        UsageLaunch {
            allowed: true,
            fixture: true,
            settings: true,
            ..Default::default()
        },
        false,
    );
    harness.run(vec![key(Key::S, Modifiers::CTRL | Modifiers::SHIFT)]);
    let folder = harness.app.usage.log.folder().unwrap().to_owned();
    assert_eq!(folder, harness.root.join("usage"));
    harness.app.usage_session_end();
    let read = || -> String {
        fs::read_dir(&folder)
            .unwrap()
            .flatten()
            .filter(|entry| usage::parse_day_file(&entry.file_name().to_string_lossy()).is_some())
            .map(|entry| fs::read_to_string(entry.path()).unwrap())
            .collect()
    };
    // The exit wait is bounded; under a loaded test run the writer may finish just after it.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !read().contains("\"k\":\"session_end\"") && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let text = read();
    for kind in ["session_start", "command", "summary", "session_end"] {
        assert!(
            text.contains(&format!("\"k\":\"{kind}\"")),
            "{kind} missing:\n{text}"
        );
    }
    assert!(text.contains("\"fixture\":true"));
    let report = usage::report::Report::load(&folder, 1, usage::now_ms()).unwrap();
    assert_eq!(report.commands["/sort"].total, 1);
    assert!(
        !harness.app.usage.log.is_enabled(),
        "shutdown stops recording"
    );
}

#[test]
fn reportable_keys_skip_text_editing_and_altgr() {
    let ctrl = Modifiers::CTRL;
    assert!(reportable_key(Key::F5, Modifiers::NONE, false));
    assert!(reportable_key(Key::Delete, Modifiers::NONE, false));
    assert!(reportable_key(Key::Z, ctrl, false));
    assert!(!reportable_key(Key::Z, ctrl, true));
    assert!(!reportable_key(Key::Delete, Modifiers::NONE, true));
    assert!(!reportable_key(Key::Escape, Modifiers::NONE, true));
    assert!(reportable_key(Key::F5, Modifiers::NONE, true));
    assert!(!reportable_key(Key::A, Modifiers::NONE, false));
    assert!(!reportable_key(Key::A, Modifiers::SHIFT, false));
    assert!(!reportable_key(
        Key::Q,
        Modifiers::CTRL | Modifiers::ALT,
        false
    ));
    assert!(!reportable_key(Key::F4, Modifiers::ALT, false));
    assert_eq!(
        key_label(Key::F5, Modifiers::CTRL | Modifiers::SHIFT),
        "Ctrl+Shift+F5"
    );
    assert_eq!(shortcut_hint(Command::Copy), "Ctrl Shift C");
    assert_eq!(shortcut_hint(Command::Usage), "");
    assert_eq!(ClipboardKind::Copy.label(Modifiers::CTRL), "Ctrl+C");
    assert_eq!(
        ClipboardKind::Copy.label(Modifiers::CTRL | Modifiers::SHIFT),
        "Ctrl+Shift+C"
    );
    assert_eq!(ClipboardKind::Cut.label(Modifiers::CTRL), "Ctrl+X");
    assert_eq!(ClipboardKind::Cut.label(Modifiers::SHIFT), "Shift+Delete");
    assert_eq!(ClipboardKind::Paste.label(Modifiers::CTRL), "Ctrl+V");
    assert_eq!(ClipboardKind::Paste.label(Modifiers::SHIFT), "Shift+Insert");
    assert_eq!(ClipboardKind::Cut.label(Modifiers::NONE), "Cut");
}
