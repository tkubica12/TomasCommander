use super::*;

fn path(value: &str) -> PathBuf {
    PathBuf::from(value)
}

fn location(value: &str) -> Location {
    Location {
        path: path(value),
        focused: None,
    }
}

fn listed(pane: &mut Pane, names: &[&str]) {
    pane.entries = names.iter().map(|name| entry(name)).collect();
    pane.busy = false;
    pane.rebuild();
}

fn back_paths(history: &History) -> Vec<PathBuf> {
    history
        .back
        .iter()
        .map(|location| location.path.clone())
        .collect()
}

fn forward_paths(history: &History) -> Vec<PathBuf> {
    history
        .forward
        .iter()
        .map(|location| location.path.clone())
        .collect()
}

fn key_event(key: Key, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn new_tab_opens_the_current_folder_right_of_the_active_tab() {
    let mut pane = Pane::new(path(r"C:\one"));
    listed(&mut pane, &["alpha", "beta"]);
    pane.filter = "b".into();
    pane.rebuild();
    pane.focused = Some(path("beta"));
    pane.selected.insert(path("alpha"));
    pane.history.record(location(r"C:\zero"));
    assert!(pane.open_tab());
    assert_eq!((pane.tabs.len(), pane.tab), (2, 1));
    assert_eq!(pane.path, path(r"C:\one"));
    assert_eq!(pane.tab_path(0), path(r"C:\one"));
    assert!(pane.filter.is_empty());
    assert_eq!(pane.visible.len(), 2, "the shared listing needs no rescan");
    assert!(!pane.busy);
    assert_eq!(pane.focused, Some(path("beta")));
    assert!(pane.selected.is_empty());
    assert!(pane.history.back.is_empty());
    pane.begin_navigation(path(r"C:\two"));
    assert!(pane.select_tab(0));
    assert_eq!(pane.filter, "b");
    assert_eq!(pane.selected, BTreeSet::from([path("alpha")]));
    assert_eq!(back_paths(&pane.history), vec![path(r"C:\zero")]);
    assert!(pane.open_tab());
    assert_eq!(pane.tab, 1);
    assert_eq!(
        (0..3).map(|tab| pane.tab_path(tab)).collect::<Vec<_>>(),
        vec![path(r"C:\one"), path(r"C:\one"), path(r"C:\two")]
    );
}

#[test]
fn tabs_are_limited_per_pane() {
    let mut pane = Pane::new(path(r"C:\one"));
    while pane.open_tab() {}
    assert_eq!(pane.tabs.len(), tomas_commander::preferences::PaneTabs::MAX);
    assert_eq!(pane.tab, pane.tabs.len() - 1);
}

#[test]
fn closing_tabs_keeps_the_last_one_and_activates_the_right_neighbour() {
    let folders: Vec<_> = ["a", "b", "c", "d"]
        .iter()
        .map(|name| path(&format!(r"C:\{name}")))
        .collect();
    let mut pane = Pane::with_tabs(folders.clone(), 1);
    assert_eq!(pane.path, folders[1]);
    assert_eq!(pane.close_tab(0), Some(false));
    assert_eq!((pane.tab, pane.path.clone()), (0, folders[1].clone()));
    assert_eq!(pane.close_tab(2), Some(false));
    assert_eq!((pane.tab, pane.tabs.len()), (0, 2));
    assert_eq!(pane.close_tab(9), None);
    assert_eq!(pane.close_tab(0), Some(true));
    assert_eq!((pane.tab, pane.path.clone()), (0, folders[2].clone()));
    assert!(pane.busy);
    assert_eq!(pane.close_tab(0), None, "the last tab cannot be closed");
    assert_eq!(pane.tabs.len(), 1);
    let mut pane = Pane::with_tabs(folders.clone(), 3);
    assert_eq!(pane.close_tab(3), Some(true));
    assert_eq!((pane.tab, pane.path.clone()), (2, folders[2].clone()));
}

#[test]
fn next_and_previous_tabs_wrap_around() {
    let folders: Vec<_> = ["a", "b", "c"]
        .iter()
        .map(|name| path(&format!(r"C:\{name}")))
        .collect();
    let mut pane = Pane::with_tabs(folders.clone(), 0);
    assert!(pane.cycle_tab(false));
    assert_eq!((pane.tab, pane.path.clone()), (2, folders[2].clone()));
    assert!(pane.cycle_tab(true));
    assert_eq!((pane.tab, pane.path.clone()), (0, folders[0].clone()));
    assert!(pane.cycle_tab(true));
    assert_eq!(pane.tab, 1);
    assert!(!pane.select_tab(1));
    assert!(!pane.select_tab(3));
    let mut single = Pane::new(folders[0].clone());
    assert!(!single.cycle_tab(true));
    assert!(!single.cycle_tab(false));
    assert_eq!(single.tab, 0);
}

#[test]
fn tabs_keep_folder_sort_filter_focus_selection_and_history_separately() {
    let mut pane = Pane::new(path(r"C:\left"));
    listed(&mut pane, &["alpha", "beta", "gamma"]);
    pane.sort = Sort::Size;
    pane.descending = true;
    pane.filter = "gam".into();
    pane.rebuild();
    pane.focused = Some(path("gamma"));
    pane.selected.insert(path("beta"));
    pane.scroll_offset = 58.0;
    pane.history.record(location(r"C:\earlier"));
    pane.parent_returns = pane.remember_parent_return();
    assert!(pane.open_tab());
    assert_eq!((pane.sort, pane.descending), (Sort::Size, true));
    pane.begin_navigation(path(r"C:\right"));
    listed(&mut pane, &["delta", "epsilon"]);
    pane.cycle_sort();
    pane.filter = "e".into();
    pane.rebuild();
    pane.focused = Some(path("epsilon"));
    pane.history.record(location(r"C:\left"));
    assert!(pane.select_tab(0));
    assert_eq!(pane.path, path(r"C:\left"));
    assert_eq!(pane.path_text, files::display_path(&path(r"C:\left")));
    assert_eq!((pane.sort, pane.descending), (Sort::Size, true));
    assert_eq!(pane.filter, "gam");
    assert_eq!(pane.focused, Some(path("gamma")));
    assert_eq!(pane.selected, BTreeSet::from([path("beta")]));
    assert_eq!(back_paths(&pane.history), vec![path(r"C:\earlier")]);
    assert_eq!(pane.parent_returns.len(), 1);
    assert_eq!(pane.restore_scroll, Some(58.0));
    assert!(
        pane.busy && pane.entries.is_empty(),
        "the listing is reloaded"
    );
    listed(&mut pane, &["alpha", "beta", "gamma"]);
    assert_eq!(pane.visible.len(), 1);
    assert_eq!(pane.focused, Some(path("gamma")), "restored focus survives");
    assert!(pane.select_tab(1));
    assert_eq!(pane.path, path(r"C:\right"));
    assert_eq!((pane.sort, pane.descending), (Sort::Name, false));
    assert_eq!(pane.filter, "e");
    assert_eq!(pane.focused, Some(path("epsilon")));
    assert!(pane.selected.is_empty());
    assert_eq!(back_paths(&pane.history), vec![path(r"C:\left")]);
    assert!(pane.parent_returns.is_empty());
    assert_eq!(
        pane.tab_preferences(),
        Some(tomas_commander::preferences::PaneTabs {
            folders: vec![path(r"C:\left"), path(r"C:\right")],
            active: 1,
        })
    );
}

#[test]
fn history_records_left_folders_clears_forward_and_skips_duplicates() {
    let mut history = History::default();
    history.record(location("A"));
    history.record(location("B"));
    history.record(Location {
        path: path("B"),
        focused: Some(path(r"B\file.txt")),
    });
    assert_eq!(back_paths(&history), vec![path("A"), path("B")]);
    assert_eq!(
        history.step(false, location("C")),
        Some(Location {
            path: path("B"),
            focused: Some(path(r"B\file.txt")),
        })
    );
    assert_eq!(back_paths(&history), vec![path("A")]);
    assert_eq!(forward_paths(&history), vec![path("C")]);
    assert_eq!(history.step(false, location("B")), Some(location("A")));
    assert_eq!(forward_paths(&history), vec![path("C"), path("B")]);
    assert_eq!(history.step(true, location("A")), Some(location("B")));
    assert_eq!(back_paths(&history), vec![path("A")]);
    assert_eq!(forward_paths(&history), vec![path("C")]);
    history.record(location("B"));
    assert!(
        history.forward.is_empty(),
        "a new navigation clears Forward"
    );
    assert_eq!(back_paths(&history), vec![path("A"), path("B")]);
    let mut empty = History::default();
    assert_eq!(empty.step(true, location("A")), None);
    assert!(empty.back.is_empty() && empty.forward.is_empty());
    let mut same = History::default();
    same.record(location("A"));
    same.record(location("B"));
    assert_eq!(
        same.step(false, location("B")),
        Some(location("A")),
        "the current folder is skipped"
    );
    same.record(location("D"));
    same.forget(&path("D"));
    assert_eq!(back_paths(&same), Vec::<PathBuf>::new());
}

#[test]
fn history_is_bounded_per_direction() {
    let mut history = History::default();
    for index in 0..70 {
        history.record(location(&format!("folder-{index}")));
    }
    assert_eq!(history.back.len(), HISTORY_LIMIT);
    assert_eq!(history.back[0].path, path("folder-20"));
    for index in 0..HISTORY_LIMIT + 5 {
        history.step(false, location(&format!("current-{index}")));
    }
    assert!(history.back.is_empty());
    assert_eq!(history.forward.len(), HISTORY_LIMIT);
}

fn scan_worker() -> ScanWorker {
    let (sender, _) = mpsc::sync_channel(1);
    ScanWorker {
        sender,
        generation: Arc::new(AtomicU64::new(0)),
        pending: None,
        fallback: false,
    }
}

fn listing(folder: &std::path::Path, names: &[&str]) -> Listing {
    Listing {
        path: folder.to_owned(),
        entries: names
            .iter()
            .map(|name| entry(&folder.join(name).to_string_lossy()))
            .collect(),
        warnings: Vec::new(),
    }
}

#[test]
fn history_steps_queue_missing_folders_for_the_worker_fallback_without_probing() {
    // Lexical paths only: a UI-side probe would canonicalize, skip or replace them.
    let current = path(r"\\?\C:\Current");
    let older = path(r"\\?\C:\Older");
    let work = path(r"\\?\C:\Work");
    let missing = work.join("Gone").join("Deeper");
    let mut pane = Pane::new(current.clone());
    listed(&mut pane, &["alpha"]);
    pane.history.record(location(r"\\?\C:\Older"));
    pane.history.record(Location {
        path: missing.clone(),
        focused: Some(missing.join("file.txt")),
    });
    let mut scan = scan_worker();
    assert!(pane.step_history(&mut scan, false));
    let request = scan.pending.as_ref().unwrap();
    assert_eq!(request.path, missing, "queued verbatim");
    assert!(request.fallback);
    assert!(scan.current(request.generation));
    assert_eq!(pane.path, missing);
    assert!(pane.busy && pane.entries.is_empty());
    assert_eq!(pane.focused, Some(missing.join("file.txt")));
    assert_eq!(back_paths(&pane.history), vec![older.clone()]);
    assert_eq!(forward_paths(&pane.history), vec![current.clone()]);

    // The worker found only C:\Work; Gone is now a file there.
    let note = pane.show_listing(listing(&work, &["Gone", "Notes"]), Some(missing.clone()));
    assert_eq!(
        note.as_deref(),
        Some(r"Folder C:\Work\Gone\Deeper is not available; showing C:\Work.")
    );
    assert_eq!(pane.path, work);
    assert_eq!(pane.path_text, r"C:\Work");
    assert_eq!(pane.focused, Some(work.join("Gone")), "focus leads back");
    assert_eq!(pane.visible.len(), 2);
    assert_eq!(
        back_paths(&pane.history),
        vec![older.clone()],
        "no new entry"
    );
    assert_eq!(forward_paths(&pane.history), vec![current.clone()]);
    assert!(pane.step_history(&mut scan, true));
    assert_eq!(pane.path, current);
    assert_eq!(
        back_paths(&pane.history),
        vec![older, work],
        "the fallback replaced the missing entry"
    );
    let mut empty = Pane::new(current);
    assert!(!empty.step_history(&mut scan, false));
}

#[test]
fn tab_activation_queues_saved_folders_and_applies_only_current_fallbacks() {
    let kept = path(r"\\?\D:\Kept");
    let projects = path(r"\\?\D:\Projects");
    let gone = projects.join("Removed");
    let mut pane = Pane::with_tabs(vec![kept.clone(), gone.clone()], 0);
    let mut scan = scan_worker();
    pane.request_load(&mut scan);
    let launch = scan.pending.take().unwrap();
    assert_eq!((launch.path.clone(), launch.fallback), (kept.clone(), true));
    assert_eq!(pane.show_listing(listing(&kept, &["a"]), None), None);
    assert!(pane.select_tab(1));
    pane.request_load(&mut scan);
    let activation = scan.pending.take().unwrap();
    assert_eq!(
        (activation.path.clone(), activation.fallback),
        (gone.clone(), true)
    );
    assert!(!scan.current(launch.generation));
    // Switching back while the fallback scan runs makes its result stale.
    assert!(pane.select_tab(0));
    pane.request_load(&mut scan);
    assert!(!scan.current(activation.generation));
    assert!(scan.current(scan.pending.as_ref().unwrap().generation));
    assert_eq!(pane.tab_path(1), gone, "a stale fallback changes nothing");
    assert!(
        scan.reload_falls_back(&pane, &kept),
        "a refresh while the tab loads keeps its fallback"
    );
    assert!(!scan.reload_falls_back(&pane, &gone));
    scan.request(kept.clone(), false);
    assert!(!scan.reload_falls_back(&pane, &kept), "ordinary navigation");

    assert!(pane.select_tab(1));
    pane.filter = "zzz".into();
    pane.selected.insert(gone.join("old.txt"));
    pane.restore_scroll = Some(120.0);
    pane.request_load(&mut scan);
    let note = pane.show_listing(listing(&projects, &["Other"]), Some(gone.clone()));
    assert_eq!(
        note.as_deref(),
        Some(r"Folder D:\Projects\Removed is not available; showing D:\Projects.")
    );
    assert_eq!(pane.tab_path(1), projects);
    assert!(pane.filter.is_empty() && pane.selected.is_empty());
    assert_eq!((pane.focused.clone(), pane.scroll_offset), (None, 0.0));
    assert_eq!(pane.visible.len(), 1);
    assert!(pane.history.back.is_empty() && pane.history.forward.is_empty());
    assert_eq!(
        pane.tab_preferences(),
        Some(tomas_commander::preferences::PaneTabs {
            folders: vec![kept, projects],
            active: 1,
        })
    );
}

#[test]
fn tabs_persist_except_for_root_runs_and_fixture_runs_on_default_settings() {
    for (root_argument, fixture, settings_explicit, expected) in [
        (false, false, false, true),
        (false, false, true, true),
        (false, true, true, true),
        (false, true, false, false),
        (true, false, false, false),
        (true, false, true, false),
        (true, true, true, false),
        (true, true, false, false),
    ] {
        assert_eq!(
            persist_tabs(root_argument, fixture, settings_explicit),
            expected,
            "root {root_argument}, fixture {fixture}, settings {settings_explicit}"
        );
    }
}

#[test]
fn tab_and_history_chords_consume_only_their_exact_keys() {
    let ctrl_shift = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::NONE
    };
    let windows_ctrl = Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::NONE
    };
    for (key, modifiers, history, expected) in [
        (Key::T, Modifiers::CTRL, true, Some(Command::NewTab)),
        (Key::T, windows_ctrl, false, Some(Command::NewTab)),
        (Key::W, Modifiers::CTRL, true, Some(Command::CloseTab)),
        (Key::Tab, Modifiers::CTRL, true, Some(Command::NextTab)),
        (Key::Tab, ctrl_shift, false, Some(Command::PreviousTab)),
        (Key::ArrowLeft, Modifiers::ALT, true, Some(Command::Back)),
        (
            Key::ArrowRight,
            Modifiers::ALT,
            true,
            Some(Command::Forward),
        ),
        (Key::ArrowLeft, Modifiers::ALT, false, None),
        (Key::ArrowRight, Modifiers::ALT, false, None),
        (Key::Tab, Modifiers::NONE, true, None),
        (Key::Tab, Modifiers::SHIFT, true, None),
        (Key::Tab, Modifiers::CTRL | Modifiers::ALT, true, None),
        (Key::T, ctrl_shift, true, None),
        (Key::T, Modifiers::NONE, true, None),
        (Key::W, Modifiers::CTRL | Modifiers::ALT, true, None),
        (Key::ArrowLeft, Modifiers::CTRL, true, None),
        (Key::ArrowRight, Modifiers::CTRL, true, None),
        (
            Key::ArrowLeft,
            Modifiers::ALT | Modifiers::SHIFT,
            true,
            None,
        ),
        (Key::ArrowLeft, Modifiers::CTRL | Modifiers::ALT, true, None),
        (Key::ArrowUp, Modifiers::ALT, true, None),
        (Key::Space, Modifiers::CTRL, true, None),
        (Key::Enter, Modifiers::CTRL, true, None),
        (Key::F, Modifiers::CTRL, true, None),
        (Key::Num0, Modifiers::CTRL, true, None),
    ] {
        let ctx = egui::Context::default();
        ctx.run_ui(
            egui::RawInput {
                events: vec![key_event(key, modifiers)],
                ..Default::default()
            },
            |_| {
                assert_eq!(
                    consume_tab_command(&ctx, history),
                    expected,
                    "{key:?} {modifiers:?}"
                );
                assert_eq!(consume_tab_command(&ctx, history), None);
                assert_eq!(
                    ctx.input(|input| input.events.len()),
                    usize::from(expected.is_none())
                );
            },
        )
        .textures_delta
        .clear();
    }
    let chord = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::NONE
    };
    for (key, _) in FILE_ACTION_CHORDS.iter().chain(&FILE_PANE_CHORDS) {
        let ctx = egui::Context::default();
        ctx.run_ui(
            egui::RawInput {
                events: vec![key_event(*key, chord)],
                ..Default::default()
            },
            |_| assert_eq!(consume_tab_command(&ctx, true), None),
        )
        .textures_delta
        .clear();
    }
}

#[test]
fn tab_chords_leave_plain_tab_typing_and_file_chords_for_their_handlers() {
    let ctx = egui::Context::default();
    let chord = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::NONE
    };
    ctx.run_ui(
        egui::RawInput {
            events: vec![
                key_event(Key::Tab, Modifiers::CTRL),
                key_event(Key::Tab, Modifiers::NONE),
                egui::Event::Text("ž".into()),
                key_event(Key::E, chord),
                key_event(Key::T, Modifiers::CTRL),
            ],
            ..Default::default()
        },
        |_| {
            assert_eq!(consume_tab_command(&ctx, true), Some(Command::NextTab));
            assert_eq!(consume_tab_command(&ctx, true), Some(Command::NewTab));
            assert_eq!(consume_tab_command(&ctx, true), None);
            assert_eq!(consume_file_action(&ctx), Some(Command::Rename));
            assert_eq!(take_filter_text(&ctx).as_deref(), Some("ž"));
            assert!(ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab)));
            assert!(ctx.input(|input| input.events.is_empty()));
        },
    )
    .textures_delta
    .clear();
}

#[test]
fn ctrl_tab_does_not_move_egui_keyboard_focus() {
    let ctx = egui::Context::default();
    let first = Id::new("first-field");
    let mut text = String::new();
    let mut other = String::new();
    let mut frame = |events: Vec<egui::Event>, request: bool| {
        ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                let response = ui.add(egui::TextEdit::singleline(&mut text).id(first));
                if request {
                    response.request_focus();
                }
                ui.add(egui::TextEdit::singleline(&mut other));
            },
        )
        .textures_delta
        .clear();
    };
    frame(Vec::new(), true);
    frame(vec![key_event(Key::Tab, Modifiers::CTRL)], false);
    frame(Vec::new(), false);
    assert!(ctx.memory(|memory| memory.has_focus(first)));
    frame(vec![key_event(Key::Tab, Modifiers::NONE)], false);
    frame(Vec::new(), false);
    assert!(
        !ctx.memory(|memory| memory.has_focus(first)),
        "plain Tab still traverses focus, which the pane switch cancels"
    );
}

#[test]
fn tab_shortcuts_are_listed_with_slash_commands_and_free_chords() {
    for (command, slash, shortcut) in [
        (Command::NewTab, "/new-tab", "Ctrl T"),
        (Command::CloseTab, "/close-tab", "Ctrl W"),
        (Command::NextTab, "/next-tab", "Ctrl Tab"),
        (Command::PreviousTab, "/previous-tab", "Ctrl Shift Tab"),
        (Command::Back, "/back", "Alt Left / Mouse back"),
        (Command::Forward, "/forward", "Alt Right / Mouse forward"),
    ] {
        assert_eq!(command.slash(), slash);
        assert!(
            COMMANDS
                .iter()
                .any(|(action, _, key)| *action == command && *key == shortcut),
            "{slash} missing from the help list"
        );
        assert_eq!(
            palette_submission(slash, None).unwrap(),
            (command, String::new())
        );
        assert!(palette_submission(&format!("{slash} extra"), Some(command)).is_err());
    }
    let chords: Vec<(Modifiers, Key)> = TAB_CHORDS
        .iter()
        .chain(&HISTORY_CHORDS)
        .map(|(modifiers, key, _)| (*modifiers, *key))
        .collect();
    for (modifiers, key) in &chords {
        assert!(!matches!(key, Key::Z | Key::Y));
        assert_eq!(
            chords
                .iter()
                .filter(|chord| *chord == &(*modifiers, *key))
                .count(),
            1
        );
    }
}

#[test]
fn mouse_side_buttons_step_history_only_over_the_pane() {
    let rect = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), Vec2::new(300.0, 400.0));
    for (button, position, expected) in [
        (egui::PointerButton::Extra1, rect.center(), Some(false)),
        (egui::PointerButton::Extra2, rect.center(), Some(true)),
        (egui::PointerButton::Extra1, egui::pos2(20.0, 20.0), None),
        (egui::PointerButton::Primary, rect.center(), None),
        (egui::PointerButton::Middle, rect.center(), None),
    ] {
        let ctx = egui::Context::default();
        ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |_| assert_eq!(side_button_history(&ctx, rect), expected, "{button:?}"),
        )
        .textures_delta
        .clear();
    }
}

fn strip_labels(count: usize) -> Vec<TabLabel> {
    (0..count)
        .map(|index| {
            TabLabel::folder(&path(&format!(
                r"C:\projects\a rather long folder name for tab number {index}"
            )))
        })
        .collect()
}

#[test]
fn tab_strip_is_hidden_with_one_tab_and_shown_with_two() {
    let ctx = egui::Context::default();
    ctx.run_ui(egui::RawInput::default(), |ui| {
        let cursor = ui.cursor().min;
        assert!(tab_strip(ui, 0, &strip_labels(1), 0, Color32::BLUE).is_none());
        assert_eq!(ui.cursor().min, cursor, "a single tab draws nothing");
        let strip = tab_strip(ui, 0, &strip_labels(2), 1, Color32::BLUE).unwrap();
        assert_eq!(strip.tabs.len(), 2);
        assert!(strip.action.is_none());
        assert!(strip.rect.height() > 0.0 && strip.rect.height() < 30.0);
        assert!(strip.tabs[0].right() <= strip.tabs[1].left());
    })
    .textures_delta
    .clear();
    assert_eq!(
        TabLabel::folder(&path(r"\\?\D:\")).name,
        r"D:\",
        "drive roots show the drive"
    );
    assert_eq!(
        TabLabel::folder(&path(r"\\?\D:\Work\Projekty")).name,
        "Projekty"
    );
}

#[test]
fn tab_strip_never_expands_narrow_resizable_panes() {
    for width in [100.0, 140.0, 230.0, 400.0, 800.0] {
        for count in [2, 5, 32] {
            let ctx = egui::Context::default();
            let labels = strip_labels(count);
            for _ in 0..6 {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(1400.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let panel = egui::Panel::left("tab-panel")
                            .default_size(width)
                            .frame(egui::Frame::NONE)
                            .show(ui, |ui| {
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(12, 9))
                                    .show(ui, |ui| {
                                        let bounds = ui.max_rect();
                                        // Like the real pane's column header, fill the width.
                                        ui.allocate_exact_size(
                                            Vec2::new(ui.available_width(), 1.0),
                                            egui::Sense::hover(),
                                        );
                                        let strip =
                                            tab_strip(ui, 0, &labels, count - 1, Color32::BLUE)
                                                .unwrap();
                                        assert!(strip.rect.left() >= bounds.left() - 0.1);
                                        assert!(
                                            strip.rect.right() <= bounds.right() + 0.1,
                                            "{width}/{count}: {:?} outside {bounds:?}",
                                            strip.rect
                                        );
                                        assert!(strip.rect.height() < 30.0, "one row only");
                                    });
                            });
                        assert!(
                            (panel.response.rect.width() - width).abs() < 1.0,
                            "{count} tabs grew the panel from {width} to {}",
                            panel.response.rect.width()
                        );
                    },
                )
                .textures_delta
                .clear();
            }
        }
    }
}

#[test]
fn tab_strip_clicks_activate_close_and_add_tabs() {
    let labels = strip_labels(3);
    let ctx = egui::Context::default();
    let mut time = 0.0;
    let mut frame = |events: Vec<egui::Event>| {
        time += 0.1;
        let mut strip = None;
        ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| strip = tab_strip(ui, 0, &labels, 0, Color32::BLUE),
        )
        .textures_delta
        .clear();
        strip.unwrap()
    };
    let layout = frame(Vec::new());
    let plus = egui::pos2(layout.rect.right() - 4.0, layout.rect.center().y);
    for (position, button, expected) in [
        (
            layout.tabs[1].center(),
            egui::PointerButton::Primary,
            TabAction::Activate(1),
        ),
        (
            layout.tabs[2].center(),
            egui::PointerButton::Middle,
            TabAction::Close(2),
        ),
        (
            egui::pos2(layout.tabs[1].right() - 4.0, layout.tabs[1].center().y),
            egui::PointerButton::Primary,
            TabAction::Close(1),
        ),
        (plus, egui::PointerButton::Primary, TabAction::New),
    ] {
        let press = |pressed| {
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]
        };
        assert_eq!(frame(press(true)).action, None);
        assert_eq!(frame(press(false)).action, Some(expected));
        assert_eq!(frame(Vec::new()).action, None);
    }
}
