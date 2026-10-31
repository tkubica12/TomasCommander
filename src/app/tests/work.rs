use super::*;

#[test]
fn archive_open_queue_does_not_depend_on_an_internal_preview_request() {
    for source in [0, 1] {
        let mut panel =
            crate::viewer_ui::ReaderPanel::new(Scope::unrestricted(), egui::Context::default());
        let archive = std::env::temp_dir().join(format!(
            "tc-queue-{}-{}.zip",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        panel.start(source, archive.clone()).unwrap();
        let (work, jobs) = mpsc::sync_channel(1);
        for preview in [None, Some("preview.json".to_owned())] {
            panel.preview_entry = preview.clone();
            panel.open_entry = Some("open.txt".into());
            queue_archive_open(&work, &mut panel, true).unwrap();
            assert!(matches!(
                jobs.try_recv().unwrap(),
                Work::OpenArchiveEntry { archive: path, entry }
                    if path == archive && entry == "open.txt"
            ));
            assert!(panel.open_entry.is_none());
            assert_eq!(panel.preview_entry, preview);
        }
        queue_archive_open(&work, &mut panel, false).unwrap();
        assert!(jobs.try_recv().is_err());
    }
}

#[test]
fn archive_open_queue_reports_blocked_and_full_queues_without_replaying_requests() {
    let mut panel =
        crate::viewer_ui::ReaderPanel::new(Scope::unrestricted(), egui::Context::default());
    let (work, jobs) = mpsc::sync_channel(1);
    panel.open_entry = Some("blocked.txt".into());
    assert!(
        queue_archive_open(&work, &mut panel, false)
            .unwrap_err()
            .contains("Finish pending work")
    );
    assert!(panel.open_entry.is_none());
    assert!(jobs.try_recv().is_err());
    panel.open_entry = Some("first.txt".into());
    queue_archive_open(&work, &mut panel, true).unwrap();
    panel.open_entry = Some("second.txt".into());
    assert!(
        queue_archive_open(&work, &mut panel, true)
            .unwrap_err()
            .contains("Could not queue ZIP member opening")
    );
    assert!(panel.open_entry.is_none());
    assert!(matches!(
        jobs.try_recv().unwrap(),
        Work::OpenArchiveEntry { entry, .. } if entry == "first.txt"
    ));
    queue_archive_open(&work, &mut panel, true).unwrap();
    assert!(jobs.try_recv().is_err());
}

#[test]
fn submitted_folder_name_creates_once_but_rename_still_requires_review() {
    use std::fs;
    let root = std::env::temp_dir().join(format!(
        "tc-name-submit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let scope = Scope::fixture(&root).unwrap();
    let root = scope.resolve(&root).unwrap();
    let journal_root = root.join("operations");
    let journal = tomas_commander::journal::Journal::new(journal_root.clone());
    let cancel = AtomicBool::new(false);
    let created = root.join("Created");
    match prepare_name_work(&scope, &journal, 1, &root, None, "Created", &cancel) {
        WorkResult::Changed { outcome, editor } => {
            assert!(!editor);
            assert!(outcome.error.is_none(), "{:?}", outcome.error);
            assert_eq!(outcome.created, vec![created.clone()]);
        }
        _ => panic!("New folder submission must execute without another review dialog."),
    }
    assert!(created.is_dir());
    assert_eq!(fs::read_dir(&created).unwrap().count(), 0);
    for name in ["Created", "bad\\name"] {
        assert!(matches!(
            prepare_name_work(&scope, &journal, 2, &root, None, name, &cancel),
            WorkResult::NamePrepared(Err(_))
        ));
    }
    cancel.store(true, Ordering::Release);
    assert!(matches!(
        prepare_name_work(&scope, &journal, 3, &root, None, "Cancelled", &cancel),
        WorkResult::Changed {
            outcome: Outcome {
                cancelled: true,
                ..
            },
            ..
        }
    ));
    assert!(!root.join("Cancelled").exists());
    cancel.store(false, Ordering::Release);
    let source = root.join("source.txt");
    fs::write(&source, b"unchanged").unwrap();
    match prepare_name_work(
        &scope,
        &journal,
        4,
        &root,
        Some(&source),
        "renamed.txt",
        &cancel,
    ) {
        WorkResult::NamePrepared(Ok(plan)) => {
            assert_eq!(plan.source(), Some(source.as_path()));
            assert_eq!(plan.target(), root.join("renamed.txt"));
            let mut editor = tomas_commander::changes::Editor::open(&scope, &source).unwrap();
            editor.text.push_str(" edited");
            let archive = tomas_commander::archive_changes::prepare(
                &scope,
                6,
                tomas_commander::archive_changes::Request::Create {
                    sources: vec![source.clone()],
                    target: root.join("sample.zip"),
                },
                &cancel,
            )
            .unwrap();
            for change in [
                Change::Name(plan),
                Change::Save(editor.plan(5).unwrap()),
                Change::Archive(archive),
            ] {
                let ctx = egui::Context::default();
                let mut height = 0.0;
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    ui.set_width(520.0);
                    change.show_summary(ui);
                    height = ui.min_rect().height();
                });
                let text: String = output
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape {
                            Some(text.galley.job.text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains("Details"));
                assert!(!text.contains("Staging"));
                assert!(!text.contains("frozen"));
                assert!(height <= 220.0, "Compact summary exceeded 220 px: {height}");
                assert_eq!(change.confirm_label().split_whitespace().count(), 1);
                output.textures_delta.clear();
            }
        }
        _ => panic!("Rename must still return a review plan."),
    }
    assert_eq!(fs::read(&source).unwrap(), b"unchanged");
    assert!(!root.join("renamed.txt").exists());
    let records: Vec<_> = fs::read_dir(&journal_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(records.len(), 2);
    for path in records {
        if path.file_name().is_some_and(|name| name == "journal.lock") {
            assert_eq!(fs::metadata(&path).unwrap().len(), 0);
        } else {
            assert_eq!(path.extension().unwrap(), "operation");
            assert!(fs::read_to_string(&path).unwrap().contains("id=1\n"));
        }
        fs::remove_file(path).unwrap();
    }
    fs::remove_file(source).unwrap();
    fs::remove_dir(created).unwrap();
    fs::remove_dir(journal_root).unwrap();
    fs::remove_dir(root).unwrap();
}

#[test]
fn undo_request_prepares_a_review_and_never_executes() {
    use std::fs;
    let root = std::env::temp_dir().join(format!(
        "tc-undo-review-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let scope = Scope::fixture(&root).unwrap();
    let root = scope.resolve(&root).unwrap();
    let journal_root = root.join("operations");
    let journal = tomas_commander::journal::Journal::new(journal_root.clone());
    let cancel = AtomicBool::new(false);
    let source = root.join("a.txt");
    fs::write(&source, b"unchanged").unwrap();
    let rename =
        match prepare_name_work(&scope, &journal, 1, &root, Some(&source), "b.txt", &cancel) {
            WorkResult::NamePrepared(Ok(plan)) => plan,
            _ => panic!("Rename must return a review plan."),
        };
    assert!(
        rename
            .execute(&scope, &journal, true, &cancel)
            .error
            .is_none()
    );
    let renamed = root.join("b.txt");
    match prepare_undo_work(&scope, &journal, 2, None, &cancel) {
        WorkResult::UndoPrepared(Ok(plan)) => {
            assert!(
                renamed.exists() && !source.exists(),
                "Preparing is not executing."
            );
            let change = Change::Undo(plan);
            assert_eq!(change.confirm_label(), "Undo");
            assert_eq!(change.label(), "Undo Rename");
            assert_eq!(change.heading(), "Undo Rename: b.txt back to a.txt");
            let ctx = egui::Context::default();
            let mut height = 0.0;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(520.0);
                change.show_summary(ui);
                height = ui.min_rect().height();
            });
            let text: String = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some(text.galley.job.text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            for expected in ["Details", "This undo cannot be undone (no redo)."] {
                assert!(text.contains(expected), "{expected:?} missing in {text}");
            }
            assert!(height <= 220.0, "Compact summary exceeded 220 px: {height}");
            output.textures_delta.clear();
        }
        _ => panic!("Undo must return a review plan, not an outcome."),
    }
    // A cancel already set targeted earlier work; the Operations read still completes.
    match history_work(&scope, &journal, &AtomicBool::new(true)) {
        WorkResult::History(Ok(entries)) => {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].status, tomas_commander::undo::Status::Undoable);
        }
        _ => panic!("The Operations read must complete."),
    }
    assert_eq!(fs::read(&renamed).unwrap(), b"unchanged");
    let records: Vec<_> = fs::read_dir(&journal_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(
        records.len(),
        2,
        "Only the rename and the journal lock exist."
    );
    for path in records {
        fs::remove_file(path).unwrap();
    }
    fs::remove_file(renamed).unwrap();
    fs::remove_dir(journal_root).unwrap();
    fs::remove_dir(root).unwrap();
}

#[test]
fn history_reads_coalesce_and_never_fill_the_work_queue() {
    let (work, jobs) = mpsc::sync_channel(4);
    let mut pending = false;
    for _ in 0..3 {
        queue_history(&work, &mut pending).unwrap();
    }
    assert!(pending);
    assert!(matches!(jobs.try_recv().unwrap(), Work::History));
    assert!(jobs.try_recv().is_err(), "One pending read at a time.");
    pending = false;
    queue_history(&work, &mut pending).unwrap();
    assert!(matches!(jobs.try_recv().unwrap(), Work::History));

    let (work, jobs) = mpsc::sync_channel(1);
    work.try_send(Work::History).unwrap();
    let mut pending = false;
    assert!(
        queue_history(&work, &mut pending)
            .unwrap_err()
            .contains("Operation history was not queued")
    );
    assert!(!pending, "A refused request can be retried.");
    drop(jobs);
}
