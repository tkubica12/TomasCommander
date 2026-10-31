use super::*;

#[test]
fn breadcrumbs_keep_exact_paths_and_never_offer_fixture_ancestors() {
    let root = std::path::Path::new(r"C:\owned");
    let path = root.join("Folder").join("Nested");
    assert_eq!(
        breadcrumb_paths(&path, Some(root)),
        vec![root.to_owned(), root.join("Folder"), path.clone()]
    );
    assert_eq!(breadcrumb_paths(&path, None).last(), Some(&path));
}

#[test]
fn compact_breadcrumbs_render_within_two_rows_even_for_long_components() {
    let path = PathBuf::from(
        r"C:\very long ancestor name\another ancestor\947dfde0-a1eb-40dc-b348-628e8df8e692\files\nested\current folder",
    );
    for width in [140.0, 200.0, 400.0, 800.0] {
        let ctx = egui::Context::default();
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                let bounds = ui.max_rect();
                compact_breadcrumbs(ui, &breadcrumb_paths(&path, None));
                assert!(ui.min_rect().height() > 0.0);
                assert!(
                    ui.min_rect().height() <= 2.0 * ui.spacing().interact_size.y.max(18.0) + 4.0
                );
                assert!(ui.min_rect().right() <= bounds.right() + 0.1);
            },
        )
        .textures_delta
        .clear();
    }
}

#[test]
fn filename_first_click_only_focuses_slow_second_click_renames_fast_double_opens() {
    for (second_press, second_release, expected_renames, expected_opens) in
        [(0.8, 0.85, 1, 0), (0.2, 0.25, 0, 1)]
    {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(80.0, 60.0), Vec2::new(300.0, 29.0));
        let mut current = false;
        let mut renames = 0;
        let mut opens = 0;
        for (time, pressed) in [
            (0.0, None),
            (0.1, Some(true)),
            (0.15, Some(false)),
            (second_press, Some(true)),
            (second_release, Some(false)),
        ] {
            let events = pressed
                .map(|pressed| {
                    vec![
                        egui::Event::PointerMoved(rect.center()),
                        egui::Event::PointerButton {
                            pos: rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Modifiers::NONE,
                        },
                    ]
                })
                .unwrap_or_default();
            ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let row = ui.interact(
                        rect.expand2(Vec2::new(30.0, 0.0)),
                        ui.id().with("file"),
                        egui::Sense::click_and_drag(),
                    );
                    let label = file_name_label(ui, rect, "alpha.txt");
                    let rename = inline_rename_click(&label, current);
                    if time == 0.15 {
                        assert!(label.clicked());
                        assert!(rename.is_none(), "First click must never schedule rename");
                    }
                    renames += usize::from(rename.is_some());
                    let response = row.union(label);
                    if response.clicked() {
                        current = true;
                    }
                    opens += usize::from(response.double_clicked());
                },
            )
            .textures_delta
            .clear();
        }
        assert!(current);
        assert_eq!(renames, expected_renames);
        assert_eq!(opens, expected_opens);
    }
}

#[test]
fn dragging_current_filename_never_requests_inline_rename() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(80.0, 60.0), Vec2::new(300.0, 29.0));
    let start = rect.center();
    let end = start + Vec2::new(60.0, 0.0);
    let mut dragged = false;
    for (frame, events) in [
        vec![],
        vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
        vec![egui::Event::PointerMoved(end)],
        vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    ]
    .into_iter()
    .enumerate()
    {
        ctx.run_ui(
            egui::RawInput {
                time: Some(frame as f64 * 0.1),
                events,
                ..Default::default()
            },
            |ui| {
                let label = file_name_label(ui, rect, "alpha.txt");
                dragged |= label.drag_started_by(egui::PointerButton::Primary);
                assert!(inline_rename_click(&label, true).is_none());
            },
        )
        .textures_delta
        .clear();
    }
    assert!(dragged);
}

#[test]
fn inline_filename_enter_submits_when_singleline_editor_loses_focus() {
    let ctx = egui::Context::default();
    let mut name = String::from("renamed-alpha.txt");
    let mut submitted = false;
    for frame in 0..3 {
        let events = if frame == 2 {
            vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: Some(Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]
        } else {
            Vec::new()
        };
        ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                let edit = ui.add(egui::TextEdit::singleline(&mut name).id(Id::new("inline-test")));
                if frame == 0 {
                    edit.request_focus();
                }
                submitted |= inline_name_submitted(ui, &edit);
            },
        )
        .textures_delta
        .clear();
    }
    assert!(submitted);
    assert_eq!(name, "renamed-alpha.txt");
}

#[test]
fn filename_click_and_selection_keep_exact_text_position() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(80.0, 60.0), Vec2::new(300.0, 29.0));
    let mut positions = Vec::new();
    let mut clicks = 0;
    for (frame, pressed) in [None, Some(true), Some(false)].into_iter().enumerate() {
        let events = pressed
            .map(|pressed| {
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ]
            })
            .unwrap_or_default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(frame as f64 * 0.05),
                events,
                ..Default::default()
            },
            |ui| {
                if frame > 0 {
                    ui.painter().rect_filled(rect, 0.0, Color32::GREEN);
                }
                let row = ui.interact(
                    rect.expand2(Vec2::new(30.0, 0.0)),
                    ui.id().with("file"),
                    egui::Sense::click_and_drag(),
                );
                let label = file_name_label(ui, rect, "alpha.txt");
                let response = row.union(label.clone());
                assert_eq!(response.clicked(), label.clicked());
                assert_eq!(label.rect, rect);
                clicks += usize::from(response.clicked());
            },
        );
        for shape in &output.shapes {
            if let egui::epaint::Shape::Text(text) = &shape.shape
                && text.galley.job.text == "alpha.txt"
            {
                positions.push(text.pos);
            }
        }
        output.textures_delta.clear();
    }
    assert_eq!(clicks, 1);
    assert_eq!(positions.len(), 3);
    assert!(positions.iter().all(|position| position == &positions[0]));
    assert_eq!(positions[0].x, rect.left());
}

#[test]
fn filename_accessibility_click_uses_the_inline_rename_hit_target() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(80.0, 60.0), Vec2::new(300.0, 29.0));
    let mut id = Id::NULL;
    ctx.run_ui(egui::RawInput::default(), |ui| {
        id = file_name_label(ui, rect, "alpha.txt").id;
    })
    .textures_delta
    .clear();
    ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Click,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    target_node: id.accesskit_id(),
                    data: None,
                },
            )],
            ..Default::default()
        },
        |ui| {
            let label = file_name_label(ui, rect, "alpha.txt");
            assert!(label.clicked());
        },
    )
    .textures_delta
    .clear();
}

#[test]
fn copy_path_menu_copies_the_exact_clicked_path_including_filename() {
    for (path, expected) in [
        (
            r"D:\Presentations\PPF intro.html",
            r"D:\Presentations\PPF intro.html",
        ),
        (
            r"\\?\D:\Presentations\PPF intro.html",
            r"D:\Presentations\PPF intro.html",
        ),
        (
            r"\\?\UNC\server\share\PPF intro.html",
            r"\\server\share\PPF intro.html",
        ),
    ] {
        let pane = Pane::new(PathBuf::from(r"D:\Presentations"));
        let path = PathBuf::from(path);
        let ctx = egui::Context::default();
        let mut id = Id::NULL;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            id = copy_path_menu_item(ui, &pane, &path).id;
        })
        .textures_delta
        .clear();
        let output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::AccessKitActionRequest(
                    egui::accesskit::ActionRequest {
                        action: egui::accesskit::Action::Click,
                        target_tree: egui::accesskit::TreeId::ROOT,
                        target_node: id.accesskit_id(),
                        data: None,
                    },
                )],
                ..Default::default()
            },
            |ui| {
                copy_path_menu_item(ui, &pane, &path);
            },
        );
        let copied: Vec<_> = output
            .platform_output
            .commands
            .iter()
            .filter_map(|command| {
                if let egui::OutputCommand::CopyText(text) = command {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(copied, vec![expected]);
    }
}

#[test]
fn copy_path_menu_copies_selected_group_on_separate_lines_or_unselected_row_alone() {
    let mut pane = Pane::new(PathBuf::from(r"D:\Presentations"));
    let first = PathBuf::from(r"D:\Presentations\Alpha intro.html");
    let second = PathBuf::from(r"D:\Presentations\Bravo.pptx");
    let unselected = PathBuf::from(r"D:\Presentations\Unselected.pdf");
    pane.selected = BTreeSet::from([second.clone(), first.clone()]);
    pane.filter = "Bravo".into();
    for (clicked, expected) in [
        (
            &second,
            "D:\\Presentations\\Alpha intro.html\r\nD:\\Presentations\\Bravo.pptx",
        ),
        (&unselected, r"D:\Presentations\Unselected.pdf"),
    ] {
        let ctx = egui::Context::default();
        let mut id = Id::NULL;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            id = copy_path_menu_item(ui, &pane, clicked).id;
        })
        .textures_delta
        .clear();
        let output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::AccessKitActionRequest(
                    egui::accesskit::ActionRequest {
                        action: egui::accesskit::Action::Click,
                        target_tree: egui::accesskit::TreeId::ROOT,
                        target_node: id.accesskit_id(),
                        data: None,
                    },
                )],
                ..Default::default()
            },
            |ui| {
                copy_path_menu_item(ui, &pane, clicked);
            },
        );
        let copied: Vec<_> = output
            .platform_output
            .commands
            .iter()
            .filter_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(copied, vec![expected]);
        assert_eq!(
            pane.selected,
            BTreeSet::from([first.clone(), second.clone()])
        );
    }
}

#[test]
fn filename_right_click_reaches_the_shared_row_context_menu() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(80.0, 60.0), Vec2::new(300.0, 29.0));
    let mut menu_opened = false;
    for pressed in [None, Some(true), Some(false)] {
        let events = pressed
            .map(|pressed| {
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ]
            })
            .unwrap_or_default();
        ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                let row = ui.interact(
                    rect.expand2(Vec2::new(30.0, 0.0)),
                    ui.id().with("file"),
                    egui::Sense::click_and_drag(),
                );
                let label = file_name_label(ui, rect, "alpha.txt");
                row.union(label).context_menu(|ui| {
                    menu_opened = true;
                    copy_path_menu_item(
                        ui,
                        &Pane::new(PathBuf::from(r"D:\")),
                        &PathBuf::from(r"D:\alpha.txt"),
                    );
                    ui.label("Open with");
                });
            },
        )
        .textures_delta
        .clear();
    }
    assert!(menu_opened);
}

#[test]
fn modified_double_click_matches_enter_actions_without_renaming() {
    assert!(double_click_command(false, Modifiers::NONE).is_none());
    assert!(matches!(
        double_click_command(false, Modifiers::CTRL),
        Some(Command::Preview)
    ));
    assert!(matches!(
        double_click_command(false, Modifiers::SHIFT),
        Some(Command::OpenWith)
    ));
    assert!(double_click_command(true, Modifiers::CTRL).is_none());
    assert!(double_click_command(true, Modifiers::SHIFT).is_none());
}

#[test]
fn egui_pointer_double_click_dispatches_modifiers_with_drag_sense() {
    for modifiers in [Modifiers::NONE, Modifiers::CTRL, Modifiers::SHIFT] {
        let ctx = egui::Context::default();
        let mut position = egui::Pos2::ZERO;
        let mut double_clicks = 0;
        for (frame, pressed) in [None, Some(true), Some(false), Some(true), Some(false)]
            .into_iter()
            .enumerate()
        {
            let mut events = vec![egui::Event::ModifiersChanged(modifiers)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerMoved(position));
                events.push(egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                });
            }
            ctx.run_ui(
                egui::RawInput {
                    time: Some(frame as f64 * 0.05),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(400.0, ROW_HEIGHT), egui::Sense::hover());
                    let row = egui::Rect::from_min_max(
                        egui::pos2(rect.left() + 30.0, rect.top()),
                        rect.max,
                    );
                    let response = file_name_label(ui, row, "alpha.txt");
                    position = response.rect.center();
                    if response.double_clicked() {
                        double_clicks += 1;
                        let command =
                            double_click_command(false, ui.input(|input| input.modifiers));
                        assert!(match modifiers {
                            Modifiers::CTRL => matches!(command, Some(Command::Preview)),
                            Modifiers::SHIFT => matches!(command, Some(Command::OpenWith)),
                            _ => command.is_none(),
                        });
                    }
                },
            )
            .textures_delta
            .clear();
        }
        assert_eq!(double_clicks, 1);
    }
}

#[test]
fn pane_text_controls_never_expand_the_resizable_panel() {
    for width in [200.0, 400.0, 800.0] {
        for path_edit in [false, true] {
            let ctx = egui::Context::default();
            let mut text = r"C:\long folder\another folder".to_owned();
            for _ in 0..12 {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(1400.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let panel = egui::Panel::left("field-panel")
                            .default_size(width)
                            .frame(egui::Frame::NONE)
                            .show(ui, |ui| {
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(12, 10))
                                    .show(ui, |ui| {
                                        let bounds = ui.max_rect();
                                        let response = trailing_field(
                                            ui,
                                            egui::TextEdit::singleline(&mut text)
                                                .margin(Vec2::new(7.0, 6.0)),
                                            |ui| {
                                                if path_edit {
                                                    let _ = ui.small_button("Done");
                                                    let _ = ui.button("Go");
                                                } else {
                                                    let _ = ui.button("x");
                                                }
                                            },
                                        );
                                        assert!(response.rect.left() >= bounds.left() - 0.1);
                                        assert!(response.rect.right() <= bounds.right() + 0.1);
                                        assert!(response.rect.height() < 40.0);
                                        assert!(ui.min_rect().height() < 60.0);
                                    });
                            });
                        assert!(
                            (panel.response.rect.width() - width).abs() < 1.0,
                            "panel grew from {width} to {}",
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
fn wrapped_selection_footer_reserves_space_above_the_action_bar() {
    for width in [180.0, 300.0, 800.0] {
        let ctx = egui::Context::default();
        let mut pane = Pane::new(PathBuf::from("fixture"));
        pane.selected.insert(PathBuf::from("hidden.txt"));
        let mut checked = 0;
        for _ in 0..3 {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(width, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let bounds = ui.max_rect();
                    let footer = selection_footer(
                        ui,
                        0,
                        &mut pane,
                        true,
                        Color32::BLUE,
                        Color32::GRAY,
                        egui::Stroke::NONE,
                    );
                    if ctx.will_discard() {
                        return;
                    }
                    checked += 1;
                    assert!(
                        footer.response.rect.bottom() <= bounds.bottom() + 0.1,
                        "footer {:?} outside {bounds:?}",
                        footer.response.rect
                    );
                    assert!(footer.response.rect.width() <= bounds.width() + 0.1);
                    assert!(
                        ui.available_rect_before_wrap().bottom()
                            <= footer.response.rect.top() + 0.1
                    );
                    if width < 400.0 {
                        assert!(footer.response.rect.height() > 42.0);
                    }
                },
            )
            .textures_delta
            .clear();
        }
        assert!(checked >= 3, "footer never reached a publishable layout");
    }
}

#[test]
fn visible_keyboard_navigation_does_not_scroll() {
    for position in 0..10 {
        assert_eq!(reveal_row(0.0, position, ROW_HEIGHT * 10.0), 0.0);
    }
    assert_eq!(
        reveal_row(ROW_HEIGHT * 5.0, 8, ROW_HEIGHT * 10.0),
        ROW_HEIGHT * 5.0
    );
}

#[test]
fn keyboard_navigation_reveals_only_the_hidden_row() {
    assert_eq!(reveal_row(0.0, 10, ROW_HEIGHT * 10.0), ROW_HEIGHT);
    assert_eq!(
        reveal_row(ROW_HEIGHT * 5.0, 4, ROW_HEIGHT * 10.0),
        ROW_HEIGHT * 4.0
    );
    assert_eq!(reveal_row(0.0, 239, ROW_HEIGHT * 10.0), ROW_HEIGHT * 230.0);
    assert_eq!(reveal_row(ROW_HEIGHT * 230.0, 0, ROW_HEIGHT * 10.0), 0.0);
}

#[test]
fn file_columns_fit_narrow_panes_without_overlapping() {
    for width in [230.0, 350.0, 600.0] {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, ROW_HEIGHT));
        let columns = RowColumns::new(rect);
        assert_eq!(columns.name.left(), 56.0);
        assert!(columns.name.width() >= 0.0);
        assert!(columns.name.right() < columns.size.left());
        assert!(columns.size.right() < columns.age.left());
        assert_eq!(columns.age.right(), width - 10.0);
    }
    for width in [100.0, 150.0, 200.0] {
        let columns = RowColumns::new(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(width, ROW_HEIGHT),
        ));
        assert!(columns.name.width() > 0.0);
        assert_eq!(columns.name.right(), width - 10.0);
        assert_eq!(columns.size.width(), 0.0);
        assert_eq!(columns.age.width(), 0.0);
    }
}

#[test]
fn modified_dates_are_utc_and_handle_gregorian_leap_days() {
    assert_eq!(format_modified(None), "-");
    assert_eq!(
        format_modified(Some(SystemTime::UNIX_EPOCH)),
        "1970-01-01 00:00"
    );
    let leap = SystemTime::UNIX_EPOCH + Duration::from_secs(951_827_696);
    assert_eq!(format_modified(Some(leap)), "2000-02-29 12:34");
    let century = SystemTime::UNIX_EPOCH + Duration::from_secs(4_107_542_400);
    assert_eq!(format_modified(Some(century)), "2100-03-01 00:00");
}
