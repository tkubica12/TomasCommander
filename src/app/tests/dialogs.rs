use super::*;

#[test]
fn drive_picker_keyboard_selects_exact_drive_and_keeps_its_pane() {
    let ctx = egui::Context::default();
    let mut picker = DrivePicker::new(
        1,
        vec![PathBuf::from(r"C:\"), PathBuf::from(r"D:\")],
        std::path::Path::new(r"\\?\C:\"),
    );
    assert_eq!(picker.index, 0);
    let mut result = None;
    for key in [None, Some(Key::ArrowDown), Some(Key::Enter)] {
        let events = key
            .map(|key| {
                vec![egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }]
            })
            .unwrap_or_default();
        ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |_| {
                let (selected, close) = picker.show(&ctx);
                assert!(!close);
                result = selected.or(result.take());
            },
        )
        .textures_delta
        .clear();
    }
    assert_eq!(picker.pane, 1);
    assert_eq!(result, Some(PathBuf::from(r"D:\")));
    assert_eq!(
        DrivePicker::new(
            0,
            vec![PathBuf::from(r"C:\"), PathBuf::from(r"D:\")],
            std::path::Path::new(r"D:\")
        )
        .index,
        1
    );
}

#[test]
fn drive_picker_empty_list_and_escape_never_select_a_path() {
    let ctx = egui::Context::default();
    let mut picker = DrivePicker::new(0, Vec::new(), std::path::Path::new(r"C:\"));
    for key in [
        None,
        Some(Key::ArrowDown),
        Some(Key::Enter),
        Some(Key::Escape),
    ] {
        let events = key
            .map(|key| {
                vec![egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }]
            })
            .unwrap_or_default();
        ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |_| {
                let (selected, close) = picker.show(&ctx);
                assert!(selected.is_none());
                assert_eq!(close, key == Some(Key::Escape));
            },
        )
        .textures_delta
        .clear();
    }
}

#[test]
fn confirmation_defaults_to_confirm_and_plain_enter_escape_are_unambiguous() {
    for (keys, expected) in [
        (vec![(Key::Enter, Modifiers::NONE, false)], (true, false)),
        (vec![(Key::Escape, Modifiers::NONE, false)], (false, true)),
        (
            vec![
                (Key::Enter, Modifiers::NONE, false),
                (Key::Escape, Modifiers::NONE, false),
            ],
            (false, true),
        ),
        (vec![(Key::Enter, Modifiers::CTRL, false)], (false, false)),
        (vec![(Key::Enter, Modifiers::SHIFT, false)], (false, false)),
    ] {
        let ctx = egui::Context::default();
        let mut focus = true;
        for frame in 0..2 {
            let events = if frame == 0 {
                vec![]
            } else {
                keys.iter()
                    .map(|(key, modifiers, repeat)| egui::Event::Key {
                        key: *key,
                        physical_key: Some(*key),
                        pressed: true,
                        repeat: *repeat,
                        modifiers: *modifiers,
                    })
                    .collect()
            };
            ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    let chosen = confirmation_buttons(ui, "Confirm", &mut focus);
                    assert_eq!(
                        chosen,
                        if frame == 0 { (false, false) } else { expected },
                        "{keys:?}, frame {frame}"
                    );
                },
            )
            .textures_delta
            .clear();
            assert!(!focus);
            if frame == 0 {
                assert!(ctx.memory(|memory| memory.focused()).is_some());
            }
        }
    }
}

#[test]
fn held_enter_cannot_confirm_a_newly_displayed_dialog() {
    let ctx = egui::Context::default();
    let mut focus = true;
    for frame in 0..3 {
        let events = if frame == 0 {
            vec![]
        } else {
            vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: Some(Key::Enter),
                pressed: true,
                repeat: frame == 2,
                modifiers: Modifiers::NONE,
            }]
        };
        if frame == 2 {
            focus = true;
        }
        ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                assert_eq!(
                    confirmation_buttons(ui, "Confirm", &mut focus),
                    if frame == 1 {
                        (true, false)
                    } else {
                        (false, false)
                    }
                );
            },
        )
        .textures_delta
        .clear();
    }
}

#[test]
fn tab_can_focus_cancel_and_enter_cancels_instead_of_confirming() {
    let ctx = egui::Context::default();
    let mut focus = true;
    for frame in 0..4 {
        let events = match frame {
            1 => vec![egui::Event::Key {
                key: Key::Tab,
                physical_key: Some(Key::Tab),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::SHIFT,
            }],
            3 => vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: Some(Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            _ => vec![],
        };
        ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                assert_eq!(
                    confirmation_buttons(ui, "Confirm", &mut focus),
                    if frame == 3 {
                        (false, true)
                    } else {
                        (false, false)
                    }
                );
            },
        )
        .textures_delta
        .clear();
    }
}

struct ReviewFixture {
    root: PathBuf,
    scope: Scope,
    sources: Vec<PathBuf>,
    destination: PathBuf,
}

impl ReviewFixture {
    fn new(name: &str) -> Self {
        use std::fs;
        let root = std::env::temp_dir().join(format!(
            "tc-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("Source")).unwrap();
        fs::create_dir(root.join("Destination")).unwrap();
        for (path, data, seconds) in [
            ("Source\\a.txt", "source a", 1_100_000_000),
            ("Source\\b.txt", "source b", 1_100_000_000),
            ("Destination\\a.txt", "existing a", 1_000_000_000),
        ] {
            fs::write(root.join(path), data).unwrap();
            fs::File::options()
                .write(true)
                .open(root.join(path))
                .unwrap()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
                .unwrap();
        }
        Self {
            scope: Scope::fixture(&root).unwrap(),
            sources: vec![root.join("Source\\a.txt"), root.join("Source\\b.txt")],
            destination: root.join("Destination"),
            root,
        }
    }

    fn plan(&self, id: u64, sources: &[PathBuf], policy: ConflictPolicy) -> Plan {
        self.plan_with(
            id,
            sources,
            Operation::Copy,
            policy,
            files::PlanOptions::default(),
        )
    }

    fn plan_with(
        &self,
        id: u64,
        sources: &[PathBuf],
        operation: Operation,
        policy: ConflictPolicy,
        options: files::PlanOptions,
    ) -> Plan {
        files::plan_with_options(
            &self.scope,
            id,
            operation,
            sources,
            Some(&self.destination),
            policy,
            options,
            &AtomicBool::new(false),
        )
        .unwrap()
    }
}

impl Drop for ReviewFixture {
    fn drop(&mut self) {
        // Review tests only plan; these are the only fixture paths.
        for path in ["Source\\a.txt", "Source\\b.txt", "Destination\\a.txt"] {
            std::fs::remove_file(self.root.join(path)).ok();
        }
        for path in ["Source", "Destination"] {
            std::fs::remove_dir(self.root.join(path)).ok();
        }
        std::fs::remove_dir(&self.root).ok();
    }
}

fn key_event(key: Key, modifiers: Modifiers, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat,
        modifiers,
    }
}

fn key_release(key: Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

/// egui marks a press as repeated while the key is still down, so taps also release.
fn key_tap(key: Key) -> Vec<egui::Event> {
    vec![key_event(key, Modifiers::NONE, false), key_release(key)]
}

fn review_frame(
    ctx: &egui::Context,
    plan: &Plan,
    review: &mut PlanReview,
    focus: &mut bool,
    events: Vec<egui::Event>,
) -> (Option<ReviewAction>, String, egui::PlatformOutput) {
    let mut action = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            events,
            ..Default::default()
        },
        |ui| action = plan_review(ui, plan, review, focus),
    );
    output.textures_delta.clear();
    let text = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape {
                Some(text.galley.job.text.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    (action, text, output.platform_output)
}

#[test]
fn conflict_review_appears_only_for_conflicts_and_defaults_to_skip() {
    let fixture = ReviewFixture::new("review-sections");
    let clean = fixture.plan(1, &fixture.sources[1..], ConflictPolicy::Skip);
    assert!(!clean.conflicts().any());
    let ctx = egui::Context::default();
    let mut review = PlanReview::new(&clean);
    let mut focus = true;
    let (action, text, _) = review_frame(&ctx, &clean, &mut review, &mut focus, vec![]);
    assert!(action.is_none());
    assert!(text.contains("Sources are kept. No overwrite."), "{text}");
    for absent in ["already exist", "Skip", "Replace", "Keep both", "Details"] {
        assert!(!text.contains(absent), "{absent:?} in {text}");
    }
    assert_eq!(review.option_ids, [Id::NULL; 4]);
    assert!(ctx.memory(|memory| memory.focused()).is_some());

    let conflicted = fixture.plan(2, &fixture.sources, ConflictPolicy::Skip);
    let review = PlanReview::new(&conflicted);
    assert_eq!(
        (conflicted.policy(), review.policy),
        (ConflictPolicy::Skip, ConflictPolicy::Skip)
    );
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut review = review;
    let mut focus = true;
    let (action, text, output) = review_frame(&ctx, &conflicted, &mut review, &mut focus, vec![]);
    assert!(action.is_none());
    for expected in [
        "Some items already exist in the destination",
        "1 new item",
        "1 file conflict: 1 source newer / 0 source older or same",
        "a.txt",
        "Details",
        "If an item already exists:",
        "Skip",
        "Replace older",
        "Replace all",
        "Keep both",
        "1 skipped",
    ] {
        assert!(text.contains(expected), "{expected:?} missing in {text}");
    }
    assert!(!text.contains("No overwrite."));
    assert!(review.ready(&conflicted));
    assert!(review.option_ids.iter().all(|id| *id != Id::NULL));
    let nodes = output.accesskit_update.unwrap().nodes;
    for (policy, id) in ConflictPolicy::CHOICES.iter().zip(review.option_ids) {
        let node = &nodes
            .iter()
            .find(|(node, _)| *node == id.accesskit_id())
            .unwrap()
            .1;
        assert_eq!(node.role(), egui::accesskit::Role::RadioButton);
        assert_eq!(
            node.label(),
            Some(format!("If an item already exists: {}", policy.label()).as_str())
        );
        assert_eq!(
            node.toggled(),
            Some(egui::accesskit::Toggled::from(
                *policy == ConflictPolicy::Skip
            ))
        );
    }

    let replace = fixture.plan(3, &fixture.sources, ConflictPolicy::ReplaceAll);
    let mut review = PlanReview::new(&replace);
    let (_, text, _) = review_frame(
        &egui::Context::default(),
        &replace,
        &mut review,
        &mut true,
        vec![],
    );
    assert!(text.contains("Copy + replace 1"), "{text}");
    assert!(text.contains("1 replaced; originals go to the Recycle Bin"));
}

#[test]
fn conflict_policy_change_replans_and_blocks_confirmation_until_the_new_plan_arrives() {
    let fixture = ReviewFixture::new("review-replan");
    let mut plan = fixture.plan(1, &fixture.sources, ConflictPolicy::Skip);
    let ctx = egui::Context::default();
    let mut review = PlanReview::new(&plan);
    let mut focus = true;
    let (action, _, _) = review_frame(&ctx, &plan, &mut review, &mut focus, vec![]);
    assert!(action.is_none());
    let click = egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::Click,
        target_tree: egui::accesskit::TreeId::ROOT,
        target_node: review.option_ids[2].accesskit_id(),
        data: None,
    });
    let (action, _, _) = review_frame(&ctx, &plan, &mut review, &mut focus, vec![click]);
    assert_eq!(
        action,
        Some(ReviewAction::Policy(ConflictPolicy::ReplaceAll))
    );
    assert_eq!(
        review.select(ConflictPolicy::ReplaceAll, &plan),
        Some(ConflictPolicy::ReplaceAll)
    );
    assert!(!review.ready(&plan));
    let job = replan_work(&plan, ConflictPolicy::ReplaceAll, 7).unwrap();
    assert!(matches!(
        job,
        Work::Replan { id: 7, operation: Operation::Copy, ref sources, ref destination, policy: ConflictPolicy::ReplaceAll }
            if sources == plan.sources() && plan.destination() == Some(destination.as_path())
    ));
    review.pending = Some((7, ConflictPolicy::ReplaceAll));
    let (action, text, _) = review_frame(
        &ctx,
        &plan,
        &mut review,
        &mut focus,
        vec![key_event(Key::Enter, Modifiers::NONE, false)],
    );
    assert!(action.is_none(), "Confirm is disabled while re-planning.");
    assert!(text.contains("Updating the plan"), "{text}");

    // A choice made while planning is coalesced; the stale result is discarded.
    assert_eq!(review.select(ConflictPolicy::KeepBoth, &plan), None);
    let stale = fixture.plan(7, &fixture.sources, ConflictPolicy::ReplaceAll);
    assert_eq!(
        review.arrived(7, Ok(stale), &mut plan),
        Some(ConflictPolicy::KeepBoth)
    );
    assert_eq!((plan.id(), plan.policy()), (1, ConflictPolicy::Skip));
    review.pending = Some((8, ConflictPolicy::KeepBoth));
    assert_eq!(review.arrived(99, Err("unrelated".into()), &mut plan), None);
    assert!(review.pending.is_some() && review.error.is_none());
    let keep = fixture.plan(8, &fixture.sources, ConflictPolicy::KeepBoth);
    assert_eq!(review.arrived(8, Ok(keep), &mut plan), None);
    assert_eq!((plan.id(), plan.policy()), (8, ConflictPolicy::KeepBoth));
    assert!(review.ready(&plan));
    assert_eq!(review.modal, 1, "The dialog identity survives re-planning.");
    let (action, text, _) = review_frame(
        &ctx,
        &plan,
        &mut review,
        &mut focus,
        vec![key_event(Key::Enter, Modifiers::NONE, true)],
    );
    assert!(action.is_none(), "Held Enter never approves the new plan.");
    assert!(text.contains("1 kept with a new name"), "{text}");
    let mut events = vec![key_release(Key::Enter)];
    events.extend(key_tap(Key::Enter));
    let (action, _, _) = review_frame(&ctx, &plan, &mut review, &mut focus, events);
    assert_eq!(action, Some(ReviewAction::Approve));

    assert_eq!(
        review.select(ConflictPolicy::ReplaceOlder, &plan),
        Some(ConflictPolicy::ReplaceOlder)
    );
    review.pending = Some((9, ConflictPolicy::ReplaceOlder));
    assert_eq!(
        review.arrived(9, Err("Synthetic planning failure".into()), &mut plan),
        None
    );
    let (action, text, _) = review_frame(&ctx, &plan, &mut review, &mut focus, key_tap(Key::Enter));
    assert!(action.is_none());
    assert!(text.contains("Synthetic planning failure"), "{text}");
    assert_eq!(review.select(ConflictPolicy::KeepBoth, &plan), None);
    assert!(
        review.ready(&plan),
        "Returning to the current plan's policy needs no re-plan."
    );
    let (action, _, _) = review_frame(&ctx, &plan, &mut review, &mut focus, key_tap(Key::Escape));
    assert_eq!(action, Some(ReviewAction::Cancel));
}

#[test]
fn conflict_policy_is_reachable_by_tab_and_arrows_without_stealing_enter() {
    let fixture = ReviewFixture::new("review-keyboard");
    let plan = fixture.plan(1, &fixture.sources, ConflictPolicy::Skip);
    let ctx = egui::Context::default();
    let mut review = PlanReview::new(&plan);
    let mut focus = true;
    for events in [
        vec![],
        vec![key_event(Key::Tab, Modifiers::SHIFT, false)],
        vec![],
        vec![key_event(Key::Tab, Modifiers::SHIFT, false)],
        vec![],
    ] {
        let (action, _, _) = review_frame(&ctx, &plan, &mut review, &mut focus, events);
        assert!(action.is_none());
    }
    assert_eq!(
        ctx.memory(|memory| memory.focused()),
        Some(review.option_ids[3]),
        "Shift+Tab from Confirm passes Cancel and reaches the last choice."
    );
    ctx.memory_mut(|memory| memory.request_focus(review.option_ids[0]));
    review_frame(&ctx, &plan, &mut review, &mut focus, vec![]);
    let (action, _, _) = review_frame(
        &ctx,
        &plan,
        &mut review,
        &mut focus,
        key_tap(Key::ArrowRight),
    );
    assert_eq!(
        action,
        Some(ReviewAction::Policy(ConflictPolicy::ReplaceOlder))
    );
    assert_eq!(
        ctx.memory(|memory| memory.focused()),
        Some(review.option_ids[1]),
        "Focus follows the arrow choice instead of moving spatially."
    );
    review.select(ConflictPolicy::ReplaceOlder, &plan);
    review.pending = Some((2, ConflictPolicy::ReplaceOlder));
    let (action, _, _) = review_frame(
        &ctx,
        &plan,
        &mut review,
        &mut focus,
        key_tap(Key::ArrowLeft),
    );
    assert_eq!(action, Some(ReviewAction::Policy(ConflictPolicy::Skip)));
    assert_eq!(review.select(ConflictPolicy::Skip, &plan), None);
    review.pending = None;
    assert!(review.ready(&plan));
    // Enter on a focused choice confirms the displayed plan; it never selects a policy.
    ctx.memory_mut(|memory| memory.request_focus(review.option_ids[2]));
    review_frame(&ctx, &plan, &mut review, &mut focus, vec![]);
    let (action, _, _) = review_frame(&ctx, &plan, &mut review, &mut focus, key_tap(Key::Enter));
    assert_eq!(action, Some(ReviewAction::Approve));
    assert_eq!(review.policy, ConflictPolicy::Skip);
}

fn across(recycle_bin: bool) -> files::PlanOptions {
    files::PlanOptions {
        force_cross_volume: true,
        recycle_bin: Some(recycle_bin),
        ..Default::default()
    }
}

const ACROSS_RECYCLE: &str = "Across drives: copy, verify, then send originals to the Recycle Bin";
const ACROSS_KEEP: &str =
    "Source drive has no Recycle Bin: files will be copied and verified, originals are kept";

#[test]
fn cross_volume_move_review_adds_one_strategy_line_and_keeps_the_move_label() {
    let fixture = ReviewFixture::new("review-across");
    let clean = &fixture.sources[1..];
    for (id, recycle_bin, shown, hidden) in [
        (1, true, ACROSS_RECYCLE, ACROSS_KEEP),
        (2, false, ACROSS_KEEP, ACROSS_RECYCLE),
    ] {
        let plan = fixture.plan_with(
            id,
            clean,
            Operation::Move,
            ConflictPolicy::Skip,
            across(recycle_bin),
        );
        assert_eq!(
            plan.move_strategy(),
            Some(files::MoveStrategy::CrossVolume {
                recycle_originals: recycle_bin
            })
        );
        let mut review = PlanReview::new(&plan);
        let (action, text, _) = review_frame(
            &egui::Context::default(),
            &plan,
            &mut review,
            &mut true,
            vec![],
        );
        assert!(action.is_none());
        for expected in [
            "Move 1 item",
            "Sources are moved. No overwrite.",
            shown,
            "Details",
        ] {
            assert!(text.contains(expected), "{expected:?} missing in {text}");
        }
        assert!(!text.contains(hidden), "{text}");
        assert!(text.lines().any(|line| line == "Move"), "{text}");
        assert!(review.ready(&plan));
    }

    let replace = fixture.plan_with(
        3,
        &fixture.sources,
        Operation::Move,
        ConflictPolicy::ReplaceAll,
        across(true),
    );
    let mut review = PlanReview::new(&replace);
    let (_, text, _) = review_frame(
        &egui::Context::default(),
        &replace,
        &mut review,
        &mut true,
        vec![],
    );
    for expected in [
        "Sources are moved; emptied merged folders are removed.",
        ACROSS_RECYCLE,
        "Move + replace 1",
        "1 replaced; originals go to the Recycle Bin",
    ] {
        assert!(text.contains(expected), "{expected:?} missing in {text}");
    }

    let same = fixture.plan_with(
        4,
        clean,
        Operation::Move,
        ConflictPolicy::Skip,
        files::PlanOptions::default(),
    );
    assert_eq!(same.move_strategy(), Some(files::MoveStrategy::SameVolume));
    let mut review = PlanReview::new(&same);
    let (_, text, _) = review_frame(
        &egui::Context::default(),
        &same,
        &mut review,
        &mut true,
        vec![],
    );
    assert!(text.contains("Sources are moved. No overwrite."), "{text}");
    for absent in ["Across drives", "Recycle Bin", "Details"] {
        assert!(!text.contains(absent), "{absent:?} in {text}");
    }
    assert_eq!(cross_volume_text(files::MoveStrategy::SameVolume), None);
}

#[test]
fn cross_volume_review_details_show_the_strategy() {
    let fixture = ReviewFixture::new("review-across-details");
    let plan = fixture.plan_with(
        1,
        &fixture.sources[1..],
        Operation::Move,
        ConflictPolicy::Skip,
        across(true),
    );
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut review = PlanReview::new(&plan);
    let mut focus = true;
    let (_, text, output) = review_frame(&ctx, &plan, &mut review, &mut focus, vec![]);
    assert!(
        !text.contains("Strategy:"),
        "Details start collapsed: {text}"
    );
    let details = output
        .accesskit_update
        .unwrap()
        .nodes
        .into_iter()
        .find(|(_, node)| node.label() == Some("Details"))
        .unwrap()
        .0;
    let click = egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::Click,
        target_tree: egui::accesskit::TreeId::ROOT,
        target_node: details,
        data: None,
    });
    let (action, _, _) = review_frame(&ctx, &plan, &mut review, &mut focus, vec![click]);
    assert!(action.is_none(), "Opening Details never approves.");
    let mut text = String::new();
    for _ in 0..30 {
        text = review_frame(&ctx, &plan, &mut review, &mut focus, vec![]).1;
        if text.contains("Strategy:") {
            break;
        }
    }
    assert!(
        text.contains("Strategy: across drives / copy each file, verify its CRC-32"),
        "{text}"
    );
    assert!(
        text.contains("then send the originals to the Recycle Bin"),
        "{text}"
    );
}

#[test]
fn operations_rows_offer_undo_only_when_undoable_and_show_short_reasons() {
    use tomas_commander::undo::{Entry, Status};
    let entry = |key: &str, label: &str, status: Status| Entry {
        key: key.into(),
        label: label.into(),
        age: Some(Duration::from_secs(120)),
        status,
    };
    let entries = vec![
        entry("4-1-0", "Move: 3 items to D:\\x", Status::Undoable),
        entry(
            "3-1-0",
            "Copy into ZIP",
            Status::Refused("Not undoable: ZIP insert".into()),
        ),
        entry("2-1-0", "Rename: a.txt to b.txt", Status::Undone),
        entry(
            "1-1-0",
            "Copy: 1 item to D:\\x",
            Status::Refused("Changed since: D:\\x\\a.txt".into()),
        ),
    ];
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let frame = |events: Vec<egui::Event>, enabled: bool| {
        let mut chosen = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| chosen = history_rows(ui, &entries, enabled),
        );
        output.textures_delta.clear();
        let texts: Vec<String> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    Some(text.galley.job.text.clone())
                } else {
                    None
                }
            })
            .collect();
        (chosen, texts, output.platform_output)
    };
    let (chosen, texts, output) = frame(vec![], true);
    assert!(chosen.is_none());
    for expected in [
        "Move: 3 items to D:\\x / 2 min ago",
        "Not undoable: ZIP insert",
        "Undone",
        "Changed since: D:\\x\\a.txt",
    ] {
        assert!(
            texts.iter().any(|text| text == expected),
            "{expected:?} in {texts:?}"
        );
    }
    assert_eq!(
        texts.iter().filter(|text| *text == "Undo").count(),
        1,
        "{texts:?}"
    );
    let undo = output
        .accesskit_update
        .unwrap()
        .nodes
        .into_iter()
        .find(|(_, node)| node.label() == Some("Undo"))
        .unwrap()
        .0;
    let click = || {
        vec![egui::Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: undo,
                data: None,
            },
        )]
    };
    assert_eq!(frame(click(), true).0.as_deref(), Some("4-1-0"));
    assert_eq!(frame(click(), false).0, None, "Busy work disables Undo.");
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        assert!(history_rows(ui, &[], true).is_none());
    });
    output.textures_delta.clear();
}
