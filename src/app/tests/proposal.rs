use super::*;
use eframe::App as _;
use std::fs;
use tomas_commander::{
    ai,
    file_tools::{Observed, Root, Settings},
    proposal::{Kind, Place, Proposal, Step},
    usage::{self, UsageLog},
};

fn observed(path: &str, directory: bool, identity: u64) -> Observed {
    Observed {
        path: PathBuf::from(path),
        root: PathBuf::from(r"D:\"),
        directory,
        size: 1,
        modified: None,
        identity: (1, identity),
        reason: "Synthetic observation".into(),
    }
}

fn step(
    kind: Kind,
    source: Option<Observed>,
    place: Option<Place>,
    name: Option<&str>,
    target: &str,
) -> Step {
    Step {
        kind,
        source,
        place,
        name: name.map(str::to_owned),
        reason: format!("Synthetic reason for {target}"),
        target: PathBuf::from(target),
    }
}

/// New folders PDF and PDF\2026, a move into 2026, a rename and a delete.
fn synthetic() -> Proposal {
    Proposal {
        summary: "Utřídit Downloads podle typu".into(),
        steps: vec![
            step(
                Kind::NewFolder,
                None,
                Some(Place::Existing(observed(r"D:\Downloads", true, 1))),
                Some("PDF"),
                r"D:\Downloads\PDF",
            ),
            step(
                Kind::NewFolder,
                None,
                Some(Place::New(0)),
                Some("2026"),
                r"D:\Downloads\PDF\2026",
            ),
            step(
                Kind::Move,
                Some(observed(r"D:\Downloads\a.pdf", false, 2)),
                Some(Place::New(1)),
                None,
                r"D:\Downloads\PDF\2026\a.pdf",
            ),
            step(
                Kind::Rename,
                Some(observed(r"D:\Downloads\photo.jpg", false, 3)),
                None,
                Some("2026-01-01_photo.jpg"),
                r"D:\Downloads\2026-01-01_photo.jpg",
            ),
            step(
                Kind::Recycle,
                Some(observed(r"D:\Downloads\setup.exe", false, 4)),
                None,
                None,
                r"D:\Downloads\setup.exe",
            ),
        ],
    }
}

fn settings() -> Settings {
    Settings {
        version: 1,
        roots: vec![],
        connection: ai::Config::default(),
        allow_content: true,
        allow_cloud_download: false,
    }
}

fn review() -> ProposalReview {
    ProposalReview::new(synthetic(), settings(), PathBuf::from(r"D:\access.json"))
}

fn key(key: Key, modifiers: Modifiers, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat,
        modifiers,
    }
}

/// A complete press and release; egui marks a press while the key is still down as a repeat.
fn tap(key: Key, modifiers: Modifiers) -> Vec<egui::Event> {
    vec![
        self::key(key, modifiers, false),
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: false,
            repeat: false,
            modifiers,
        },
    ]
}

/// One frame of the review: its choice, shown texts and AccessKit nodes.
fn frame(
    ctx: &egui::Context,
    review: &mut ProposalReview,
    events: Vec<egui::Event>,
) -> (
    Option<ProposalChoice>,
    Vec<String>,
    Vec<(egui::accesskit::NodeId, egui::accesskit::Node)>,
) {
    let mut choice = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1240.0, 900.0),
            )),
            events,
            ..Default::default()
        },
        |ui| choice = review.content(ui),
    );
    output.textures_delta.clear();
    let texts = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
            _ => None,
        })
        .collect();
    let nodes = output
        .platform_output
        .accesskit_update
        .map(|update| update.nodes)
        .unwrap_or_default();
    (choice, texts, nodes)
}

fn node(
    nodes: &[(egui::accesskit::NodeId, egui::accesskit::Node)],
    label: impl Fn(&str) -> bool,
) -> egui::accesskit::NodeId {
    nodes
        .iter()
        .find(|(_, node)| node.label().is_some_and(&label))
        .map(|(id, _)| *id)
        .expect("accessible node")
}

fn action(target: egui::accesskit::NodeId, action: egui::accesskit::Action) -> Vec<egui::Event> {
    vec![egui::Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            action,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: target,
            data: None,
        },
    )]
}

#[test]
fn deletes_start_unchecked_and_marked_while_other_changes_start_checked() {
    let mut review = review();
    assert_eq!(review.checked, [true, true, true, true, false]);
    assert_eq!(review.selected(), 4);
    assert_eq!(review.unchecked_deletes(), 1);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (choice, texts, _) = frame(&ctx, &mut review, vec![]);
    assert!(choice.is_none(), "Opening never runs anything.");
    for expected in [
        "Review AI proposal: 5 changes",
        "Utřídit Downloads podle typu",
        r"Paths relative to D:\Downloads",
        "New folders (2)",
        "(new folder) → PDF",
        r"(new folder) → PDF\2026",
        "Moves (1)",
        r"a.pdf → PDF\2026\a.pdf",
        "Renames (1)",
        "photo.jpg → 2026-01-01_photo.jpg",
        "Deletes (Recycle Bin) (1)",
        "DELETE  setup.exe → Recycle Bin",
        "Run selected (4)",
        "0 of 1 deletes selected",
    ] {
        assert!(
            texts.iter().any(|text| text == expected),
            "{expected:?} in {texts:?}"
        );
    }
    review.set(4, true);
    assert_eq!(review.selected(), 5);
    assert_eq!(review.unchecked_deletes(), 0);
}

#[test]
fn unchecking_a_new_folder_disables_its_dependents_and_rechecking_restores_them() {
    let mut review = review();
    review.set(0, false);
    assert_eq!(review.checked, [false, false, false, true, false]);
    assert_eq!(review.blocked(1), Some(0));
    assert_eq!(review.blocked(2), Some(1));
    review.set(2, true);
    assert!(!review.checked[2], "A change without its folder stays off.");
    assert_eq!(review.selection(), [false, false, false, true, false]);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (_, texts, _) = frame(&ctx, &mut review, vec![]);
    assert!(
        texts
            .iter()
            .any(|text| text == r"a.pdf → PDF\2026\a.pdf  (needs unchecked new folder PDF\2026)"),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text == "Run selected (1)"),
        "{texts:?}"
    );
    review.set(0, true);
    assert_eq!(review.checked, [true, true, true, true, false]);
    // A change the user unchecked on purpose stays unchecked.
    review.set(2, false);
    review.set(0, false);
    review.set(0, true);
    assert_eq!(review.checked, [true, true, false, true, false]);
    // Clicking a folder row in the dialog cascades the same way.
    let (_, _, nodes) = frame(&ctx, &mut review, vec![]);
    let folder = node(&nodes, |label| label == r"(new folder) → PDF\2026");
    frame(
        &ctx,
        &mut review,
        action(folder, egui::accesskit::Action::Click),
    );
    assert_eq!(review.checked, [true, false, false, true, false]);
}

#[test]
fn enter_runs_only_from_the_focused_run_button_and_held_enter_never_runs() {
    let mut review = review();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    // Enter is still held from the action that opened the review.
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![key(Key::Enter, Modifiers::NONE, false)],
            ..Default::default()
        },
        |_| (),
    );
    output.textures_delta.clear();
    let (choice, _, nodes) = frame(&ctx, &mut review, vec![]);
    assert!(choice.is_none());
    let run = node(&nodes, |label| label == "Run selected (4)");
    // Run has initial focus, but the held key's repeats never confirm.
    for _ in 0..3 {
        let (choice, ..) = frame(
            &ctx,
            &mut review,
            vec![key(Key::Enter, Modifiers::NONE, true)],
        );
        assert!(choice.is_none(), "{choice:?}");
    }
    let mut released = tap(Key::Enter, Modifiers::NONE);
    released.remove(0);
    assert!(frame(&ctx, &mut review, released).0.is_none());
    // Space on the focused Run button neither runs nor toggles anything.
    let (choice, ..) = frame(&ctx, &mut review, tap(Key::Space, Modifiers::NONE));
    assert!(choice.is_none());
    assert_eq!(review.checked, [true, true, true, true, false]);
    // Focus on a row: Space toggles it, Enter does nothing.
    let row = node(&nodes, |label| label.starts_with("photo.jpg → "));
    frame(
        &ctx,
        &mut review,
        action(row, egui::accesskit::Action::Focus),
    );
    let (choice, ..) = frame(&ctx, &mut review, tap(Key::Space, Modifiers::NONE));
    assert!(choice.is_none());
    assert_eq!(review.checked, [true, true, true, false, false]);
    let (choice, ..) = frame(&ctx, &mut review, tap(Key::Enter, Modifiers::NONE));
    assert!(choice.is_none(), "Enter on a row never runs.");
    // Arrows move between rows; Space then toggles the newly focused row.
    frame(&ctx, &mut review, tap(Key::ArrowDown, Modifiers::NONE));
    frame(&ctx, &mut review, tap(Key::Space, Modifiers::NONE));
    assert_eq!(
        review.checked,
        [true, true, true, false, true],
        "The delete row."
    );
    // Back on Run, a fresh Enter runs the selection.
    frame(
        &ctx,
        &mut review,
        action(run, egui::accesskit::Action::Focus),
    );
    let (choice, ..) = frame(&ctx, &mut review, tap(Key::Enter, Modifiers::NONE));
    assert_eq!(choice, Some(ProposalChoice::Run));
}

#[test]
fn checking_a_row_of_a_delete_only_proposal_never_moves_focus_to_run() {
    let deletes = Proposal {
        summary: "Smazat staré instalačky".into(),
        steps: synthetic().steps.split_off(4),
    };
    let mut review = ProposalReview::new(deletes, settings(), PathBuf::from(r"D:\access.json"));
    assert_eq!(
        review.selected(),
        0,
        "Deletes start unchecked; Run is disabled."
    );
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (choice, _, nodes) = frame(&ctx, &mut review, vec![]);
    assert!(choice.is_none());
    // Opening with Run disabled focuses Cancel, so Enter can only cancel.
    let row = node(&nodes, |label| label.starts_with("DELETE"));
    frame(
        &ctx,
        &mut review,
        action(row, egui::accesskit::Action::Focus),
    );
    let (choice, ..) = frame(&ctx, &mut review, tap(Key::Space, Modifiers::NONE));
    assert!(choice.is_none());
    assert_eq!(review.selected(), 1, "Space checked the focused delete.");
    // Focus stays on the row: Enter does nothing and Space unchecks it again.
    let (choice, ..) = frame(&ctx, &mut review, tap(Key::Enter, Modifiers::NONE));
    assert!(
        choice.is_none(),
        "Enter on a row never runs, even after checking it."
    );
    frame(&ctx, &mut review, tap(Key::Space, Modifiers::NONE));
    assert_eq!(review.selected(), 0, "The same row was toggled back.");
}
#[test]
fn escape_and_tab_to_cancel_close_the_review_and_failures_keep_it_open() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut review = review();
    frame(&ctx, &mut review, vec![]);
    let (choice, ..) = frame(&ctx, &mut review, tap(Key::Escape, Modifiers::NONE));
    assert_eq!(choice, Some(ProposalChoice::Cancel));

    let mut review = self::review();
    frame(&ctx, &mut review, vec![]);
    frame(&ctx, &mut review, tap(Key::Tab, Modifiers::SHIFT));
    let (choice, ..) = frame(&ctx, &mut review, tap(Key::Enter, Modifiers::NONE));
    assert_eq!(
        choice,
        Some(ProposalChoice::Cancel),
        "Enter on Cancel cancels."
    );

    let mut review = self::review();
    review.failures =
        vec!["Step 1 (New folder: D:\\Downloads\\PDF): That name already exists".into()];
    review.checking = true;
    let (choice, texts, _) = frame(&ctx, &mut review, vec![]);
    assert!(choice.is_none());
    assert!(
        texts
            .iter()
            .any(|text| text.starts_with("Checking the selected changes")),
        "{texts:?}"
    );
    let (choice, ..) = frame(&ctx, &mut review, tap(Key::Enter, Modifiers::NONE));
    assert!(choice.is_none(), "Run is disabled while checking.");
    review.checking = false;
    let (_, texts, _) = frame(&ctx, &mut review, vec![]);
    assert!(
        texts
            .iter()
            .any(|text| text == "Not started; nothing changed. Uncheck or fix these changes:"),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.contains("That name already exists"))
    );
}

/// A real `Ledger` over owned fixture files, driven frame by frame.
struct App {
    app: Ledger,
    ctx: egui::Context,
    frame: eframe::Frame,
    events: usage::Captured,
    root: PathBuf,
    time: f64,
}

impl App {
    fn new(root: &std::path::Path, files: &std::path::Path) -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let creation = eframe::CreationContext::_new_kittest(ctx.clone());
        let scope = Scope::fixture(files).unwrap();
        let initial = scope.resolve(files).unwrap();
        let mut app = Ledger::new(
            &creation,
            initial,
            scope,
            root.join("preferences.txt"),
            Instant::now(),
            false,
            UsageLaunch::default(),
        );
        let (log, events) = UsageLog::memory();
        app.usage.log = log;
        let mut this = Self {
            app,
            ctx,
            frame: eframe::Frame::_new_kittest(),
            events,
            root: root.to_path_buf(),
            time: 0.0,
        };
        this.until(|app| app.panes.iter().all(|pane| !pane.busy));
        this
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        self.time += 0.5;
        let app = &mut self.app;
        let frame = &mut self.frame;
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1240.0, 900.0),
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
    }

    fn until(&mut self, done: impl Fn(&Ledger) -> bool) {
        for _ in 0..400 {
            self.run(vec![]);
            if done(&self.app) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let review = self.app.proposal_review.as_ref().map(|review| {
            (
                review.checking,
                review.failures.clone(),
                review.error.clone(),
            )
        });
        panic!(
            "the application did not reach the expected state: review {review:?}, busy {}, error {:?}, activity {:?}",
            self.app.operation_busy, self.app.error, self.app.activity
        );
    }

    fn kinds(&self, kind: &str) -> Vec<usage::Event> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.kind == kind)
            .cloned()
            .collect()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // The Recent monitor persists its state on shutdown; stop it before removing the root.
        let _ = self.app.recent.shutdown();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_failed_check_prevents_any_start_and_an_approved_run_journals_each_step() {
    let root = std::env::temp_dir().join(format!(
        "tc-ai-proposal-app-{}-{}",
        std::process::id(),
        usage::now_ms()
    ));
    let files = root.join("files");
    let downloads = files.join("Downloads");
    fs::create_dir_all(&downloads).unwrap();
    fs::write(downloads.join("a.txt"), b"a").unwrap();
    fs::write(downloads.join("b.txt"), b"b").unwrap();
    let settings = Settings {
        version: 1,
        roots: vec![Root {
            name: "Synthetic fixture".into(),
            path: fs::canonicalize(&files).unwrap(),
        }],
        connection: ai::Config {
            resource: "synthetic-foundry".into(),
            resource_group: "synthetic".into(),
            subscription: "00000000-0000-0000-0000-000000000001".into(),
            tenant: "00000000-0000-0000-0000-000000000002".into(),
            identity: "test@example.org".into(),
            deployment: "synthetic-luna".into(),
            model: "synthetic-luna".into(),
            ..Default::default()
        },
        allow_content: true,
        allow_cloud_download: true,
    };
    let settings_path = root.join("access.json");
    settings.save(&settings_path).unwrap();
    let mut tools = tomas_commander::file_tools::Tools::new(settings.clone()).unwrap();
    let mut id = |path: PathBuf| tools.observe(&path, "test").unwrap()["id"].clone();
    let (folder, a, b) = (
        id(downloads.clone()),
        id(downloads.join("a.txt")),
        id(downloads.join("b.txt")),
    );
    let proposal = tomas_commander::proposal::validate(
        &serde_json::json!({"summary":"Sort texts","operations":[
            {"op":"new_folder","parent":folder,"name":"Texts","reason":"Texts"},
            {"op":"move","source":a,"destination":"f1","reason":"Text"},
            {"op":"move","source":b,"destination":"f1","reason":"Text"}
        ]})
        .to_string(),
        &mut tools,
    )
    .unwrap();
    let mut harness = App::new(&root, &files);
    harness
        .app
        .open_proposal(proposal, settings, settings_path.clone());
    // The planned folder appears after the review opened: the check fails, nothing starts.
    fs::create_dir(downloads.join("Texts")).unwrap();
    harness.until(|app| app.panes.iter().all(|pane| !pane.busy));
    harness.run(vec![]);
    harness.run(tap(Key::Enter, Modifiers::NONE));
    harness.until(|app| {
        app.proposal_review
            .as_ref()
            .is_some_and(|review| !review.checking && !review.failures.is_empty())
    });
    let review = harness.app.proposal_review.as_ref().unwrap();
    assert_eq!(review.failures.len(), 3, "{:?}", review.failures);
    assert!(review.failures[0].contains("already exists"));
    assert!(!harness.app.operation_busy);
    assert!(harness.app.last_batch.is_none());
    assert!(downloads.join("a.txt").exists() && downloads.join("b.txt").exists());
    assert!(
        !root.join("preferences.operations").exists() || {
            fs::read_dir(root.join("preferences.operations"))
                .unwrap()
                .all(|entry| {
                    !entry
                        .unwrap()
                        .path()
                        .to_string_lossy()
                        .ends_with(".operation")
                })
        }
    );

    // Resolved: Enter on the refocused Run checks again and runs every step.
    fs::remove_dir(downloads.join("Texts")).unwrap();
    harness.until(|app| app.panes.iter().all(|pane| !pane.busy));
    harness.run(vec![]);
    harness.run(tap(Key::Enter, Modifiers::NONE));
    harness.until(|app| app.proposal_review.is_none() && !app.operation_busy);
    let report = harness.app.last_batch.as_ref().expect("batch report");
    assert_eq!(report.done(), 3, "{:?}", report.rows());
    assert!(harness.app.error.is_none(), "{:?}", harness.app.error);
    assert_eq!(fs::read(downloads.join("Texts\\a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(downloads.join("Texts\\b.txt")).unwrap(), b"b");
    let records: Vec<String> = fs::read_dir(root.join("preferences.operations"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "operation")
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert_eq!(records.len(), 3);
    let marker = format!("\nbatch={}\n", report.key);
    assert!(records.iter().all(|record| record.contains(&marker)));
    let proposals = harness.kinds("ai_proposal");
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].text("outcome"), Some("run"));
    assert_eq!(proposals[0].field("checked"), Some(&usage::Value::Int(3)));
    assert_eq!(
        proposals[0].field("failed_checks"),
        Some(&usage::Value::Int(1)),
        "One check found problems before the run."
    );
    assert_eq!(
        proposals[0].field("ops"),
        Some(&usage::Value::Counts(vec![
            ("new_folder".into(), 1),
            ("move".into(), 2)
        ]))
    );
    let batches = harness.kinds("ai_batch");
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].field("done"), Some(&usage::Value::Int(3)));
    assert_eq!(batches[0].field("failed_at"), Some(&usage::Value::Null));
}

#[test]
fn operations_group_a_batch_and_offer_undo_batch_only_when_a_step_is_undoable() {
    use tomas_commander::undo::{Entry, Status};
    let entry = |key: &str, label: &str, status: Status, batch: Option<&str>| Entry {
        key: key.into(),
        label: label.into(),
        age: None,
        status,
        batch: batch.map(str::to_owned),
    };
    let entries = vec![
        entry("5-1-0", "Rename: a.txt to b.txt", Status::Undoable, None),
        entry(
            "4-1-0",
            "Move: 1 item to D:\\Downloads\\PDF",
            Status::Undoable,
            Some("ai-7"),
        ),
        entry("3-1-0", "New folder: PDF", Status::Undoable, Some("ai-7")),
        entry("2-1-0", "Delete", Status::Undone, Some("ai-6")),
    ];
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let render = |events: Vec<egui::Event>| {
        let mut chosen = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| chosen = history_rows(ui, &entries, true),
        );
        output.textures_delta.clear();
        let texts: Vec<String> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                _ => None,
            })
            .collect();
        (chosen, texts, output.platform_output)
    };
    let (_, texts, output) = render(vec![]);
    assert!(
        texts.iter().any(|text| text == "AI changes: 2 steps"),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text == "AI changes: 1 step"),
        "{texts:?}"
    );
    assert_eq!(
        texts.iter().filter(|text| *text == "Undo batch").count(),
        1,
        "An undone batch offers no Undo batch: {texts:?}"
    );
    let nodes = output.accesskit_update.unwrap().nodes;
    let button = node(&nodes, |label| label == "Undo batch");
    let (chosen, ..) = render(action(button, egui::accesskit::Action::Click));
    assert_eq!(chosen, Some(HistoryChoice::UndoBatch("ai-7".into())));
}
