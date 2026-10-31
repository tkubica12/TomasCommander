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
