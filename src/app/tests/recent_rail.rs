use super::*;

fn recent_item(name: &str, at_ms: u64) -> recent::Item {
    serde_json::from_value(serde_json::json!({
        "path": format!("D:\\synthetic\\{name}"), "at_ms": at_ms,
        "activity": "modified", "modified_ms": 1, "size": 7
    }))
    .unwrap()
}

#[test]
fn recent_cursor_keeps_the_file_identity_across_reordering_and_never_selects_a_replacement() {
    let first = recent_item("first.md", 10);
    let second = recent_item("second.md", 20);
    let old = vec![first.clone(), second.clone()];
    let new = vec![second.clone(), first];
    assert_eq!(remap_recent_cursor(&old, &new, Some(0)), Some(1));
    assert_eq!(remap_recent_cursor(&old, &[second], Some(0)), None);
    assert_eq!(remap_recent_cursor(&old, &new, None), None);
    assert_eq!(remap_recent_cursor(&[], &new, Some(0)), Some(0));
}

#[test]
fn recent_workspace_toggle_depends_on_visibility_not_focus_or_selected_tab() {
    for initial_open in [false, true] {
        for initial_rail in [Rail::Favorites, Rail::Recent] {
            for initial_index in [None, Some(3)] {
                let mut open = initial_open;
                let mut rail = initial_rail;
                let mut index = initial_index;
                for _ in 0..4 {
                    let was_open = open;
                    toggle_recent_rail(&mut open, &mut rail, &mut index);
                    assert_eq!(open, !was_open);
                    assert_eq!(index, open.then_some(0));
                    if open {
                        assert!(rail == Rail::Recent);
                    }
                }
            }
        }
    }
    assert!(
        COMMANDS
            .iter()
            .any(|(command, _, shortcut)| *command == Command::Favorites && shortcut.is_empty())
    );
}

#[test]
fn recent_rows_use_the_same_filename_font_and_height_as_file_panes() {
    let ctx = egui::Context::default();
    let item = recent_item("matching-font.txt", 20);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let (response, _) = recent_row(ui, &item, false, true);
        assert_eq!(response.rect.height(), ROW_HEIGHT);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(300.0, ROW_HEIGHT), egui::Sense::hover());
        file_name_label(ui, rect, "matching-font.txt");
    });
    let mut matching_labels = 0;
    for shape in &output.shapes {
        if let egui::epaint::Shape::Text(text) = &shape.shape
            && text.galley.job.text == "matching-font.txt"
        {
            matching_labels += 1;
            assert!(
                text.galley
                    .job
                    .sections
                    .iter()
                    .all(|section| section.format.font_id == FILE_NAME_FONT)
            );
        }
    }
    assert_eq!(matching_labels, 2);
    output.textures_delta.clear();
}

#[test]
fn recent_keyboard_actions_consume_only_the_matching_chord() {
    for (modifiers, key, expected) in [
        (Modifiers::CTRL, Key::Enter, Some(RecentAction::Preview)),
        (Modifiers::SHIFT, Key::Enter, Some(RecentAction::OpenWith)),
        (
            Modifiers::CTRL | Modifiers::SHIFT,
            Key::C,
            Some(RecentAction::CopyPath),
        ),
        (Modifiers::ALT, Key::Enter, Some(RecentAction::Locate)),
        (Modifiers::NONE, Key::Enter, None),
        (Modifiers::CTRL, Key::C, None),
    ] {
        let ctx = egui::Context::default();
        ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |_| {
                assert_eq!(consume_recent_action(&ctx, false), expected);
                assert_eq!(consume_recent_action(&ctx, false), None);
            },
        )
        .textures_delta
        .clear();
    }
}

#[test]
fn recent_keyboard_actions_handle_windows_copy_translation() {
    for shift in [true, false] {
        let ctx = egui::Context::default();
        let modifiers = Modifiers {
            ctrl: true,
            command: true,
            shift,
            ..Modifiers::NONE
        };
        let mut previous = Modifiers::NONE;
        ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::Copy,
                    egui::Event::ModifiersChanged(Modifiers::NONE),
                ],
                ..Default::default()
            },
            |_| {
                let copy = consume_file_copy(&ctx, &mut previous, true);
                assert_eq!(
                    consume_recent_action(&ctx, copy),
                    shift.then_some(RecentAction::CopyPath)
                );
                assert_eq!(
                    ctx.input(|input| input.events.contains(&egui::Event::Copy)),
                    !shift
                );
            },
        )
        .textures_delta
        .clear();
    }
}

#[test]
fn recent_single_click_locates_double_click_opens_and_slow_clicks_never_open() {
    for (second_click, expected) in [
        (None, vec![RecentAction::Locate]),
        (Some(0.15), vec![RecentAction::Locate, RecentAction::Open]),
        (Some(1.0), vec![RecentAction::Locate, RecentAction::Locate]),
    ] {
        let ctx = egui::Context::default();
        let item = recent_item("presentation.pptx", 20);
        let mut position = egui::pos2(30.0, 30.0);
        let mut actions = Vec::new();
        let mut frames = vec![(0.0, None), (0.05, Some(true)), (0.1, Some(false))];
        if let Some(time) = second_click {
            frames.extend([(time, Some(true)), (time + 0.05, Some(false))]);
        }
        for (time, pressed) in frames {
            let events = pressed
                .map(|pressed| {
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
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
                    let (response, action) = recent_row(ui, &item, false, time < 0.15);
                    position = response.rect.center();
                    actions.extend(action.map(|(action, _)| action));
                },
            )
            .textures_delta
            .clear();
        }
        assert_eq!(actions, expected);
    }
}

#[test]
fn recent_row_context_menu_is_reachable_and_drag_uses_the_exact_physical_file() {
    let item = recent_item("presentation.pptx", 20);
    let ctx = egui::Context::default();
    let mut position = egui::pos2(30.0, 30.0);
    let mut containing_folder = None;
    for pressed in [None, Some(true), Some(false), None] {
        let events = pressed
            .map(|pressed| {
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ]
            })
            .unwrap_or_default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                let (response, action) = recent_row(ui, &item, false, true);
                position = response.rect.center();
                assert!(action.is_none());
            },
        );
        for shape in &output.shapes {
            if let egui::epaint::Shape::Text(text) = &shape.shape
                && text.galley.job.text == "Open containing folder"
            {
                containing_folder = Some(text.pos + text.galley.size() / 2.0);
            }
        }
        output.textures_delta.clear();
    }
    assert!(egui::Popup::is_any_open(&ctx));
    let position = containing_folder.expect("Recent menu must offer Open containing folder");
    let mut chosen = None;
    for pressed in [true, false] {
        ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| {
                let (_, action) = recent_row(ui, &item, false, true);
                if action.is_some() {
                    chosen = action;
                }
            },
        )
        .textures_delta
        .clear();
    }
    assert_eq!(chosen, Some((RecentAction::Locate, Via::ContextMenu)));
    for enabled in [false, true] {
        let ctx = egui::Context::default();
        let mut start = egui::pos2(30.0, 30.0);
        for step in 0..3 {
            let events = match step {
                0 => vec![],
                1 => vec![
                    egui::Event::PointerMoved(start),
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                ],
                _ => vec![egui::Event::PointerMoved(start + Vec2::new(80.0, 80.0))],
            };
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(600.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let (response, action) = recent_row(ui, &item, false, enabled);
                    start = response.rect.center();
                    assert!(action.is_none());
                },
            )
            .textures_delta
            .clear();
        }
        let payload = egui::DragAndDrop::payload::<DragItems>(&ctx);
        if enabled {
            assert!(
                matches!(payload.as_deref(), Some(DragItems::Files(paths)) if paths == &vec![item.path.clone()])
            );
        } else {
            assert!(payload.is_none());
        }
    }
}
