use super::*;

#[test]
fn space_in_file_pane_remains_a_selection_key_not_a_filter() {
    let ctx = egui::Context::default();
    ctx.run_ui(
        egui::RawInput {
            events: vec![
                egui::Event::Key {
                    key: Key::Space,
                    physical_key: Some(Key::Space),
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                },
                egui::Event::Text(" ".into()),
            ],
            ..Default::default()
        },
        |_| {
            assert_eq!(take_filter_text(&ctx), None);
            assert!(ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Space)));
        },
    )
    .textures_delta
    .clear();
}

#[test]
fn ask_shortcut_uses_ctrl_space_without_consuming_selection_or_modified_variants() {
    let ctx = egui::Context::default();
    let palette = Modifiers::CTRL | Modifiers::SHIFT;
    for (key, modifiers, opens) in [
        (Key::Space, Modifiers::CTRL, Some("Ctrl Space")),
        (Key::Space, Modifiers::NONE, None),
        (Key::Space, Modifiers::SHIFT, None),
        (Key::Space, palette, None),
        (Key::Space, Modifiers::CTRL | Modifiers::ALT, None),
        // Ctrl+Shift+P opens Ask too, as the command palette does elsewhere.
        (Key::P, palette, Some("Ctrl Shift P")),
        (Key::P, Modifiers::CTRL, None),
        (Key::P, palette | Modifiers::ALT, None),
        (Key::P, Modifiers::NONE, None),
    ] {
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
                assert_eq!(consume_ask_key(&ctx), opens, "{key:?} {modifiers:?}");
                assert_eq!(
                    ctx.input(|input| input.events.len()),
                    usize::from(opens.is_none())
                );
            },
        )
        .textures_delta
        .clear();
    }
}

#[test]
fn ctrl_shift_a_creates_a_zip_and_never_selects_all() {
    let ctx = egui::Context::default();
    let zip = Modifiers::CTRL | Modifiers::SHIFT;
    ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::A,
                physical_key: Some(Key::A),
                pressed: true,
                repeat: false,
                modifiers: zip,
            }],
            ..Default::default()
        },
        |_| {
            assert!(
                !consume_prompt_key(&ctx, Modifiers::CTRL, Key::A),
                "Select all matches Ctrl+A exactly."
            );
            assert_eq!(consume_file_action(&ctx), Some(Command::CreateZip));
        },
    )
    .textures_delta
    .clear();
}

#[test]
fn typing_to_filter_consumes_unicode_text_once_and_preserves_other_events() {
    let ctx = egui::Context::default();
    ctx.run_ui(
        egui::RawInput {
            events: vec![
                egui::Event::Text("Příliš ".into()),
                egui::Event::Text("žluťoučký".into()),
                egui::Event::Paste("not automatic filter input".into()),
            ],
            ..Default::default()
        },
        |_| {
            assert_eq!(take_filter_text(&ctx).as_deref(), Some("Příliš žluťoučký"));
            assert_eq!(take_filter_text(&ctx), None);
            assert!(ctx.input(|input| matches!(input.events.as_slice(), [egui::Event::Paste(_)])));
        },
    )
    .textures_delta
    .clear();
}

#[test]
fn typing_to_filter_preserves_shortcuts_and_accepts_altgr_text() {
    for (modifiers, expected) in [
        (Modifiers::CTRL, None),
        (Modifiers::ALT, None),
        (
            Modifiers {
                ctrl: true,
                alt: true,
                ..Default::default()
            },
            Some("@"),
        ),
        (Modifiers::SHIFT, Some("@")),
    ] {
        let ctx = egui::Context::default();
        ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::Text("@".into()),
                ],
                ..Default::default()
            },
            |_| assert_eq!(take_filter_text(&ctx).as_deref(), expected),
        )
        .textures_delta
        .clear();
    }
}

#[test]
fn down_leaves_only_the_active_filter_and_consumes_the_first_step() {
    let ctx = egui::Context::default();
    let filter = Id::new(("filter", 1usize));
    ctx.run_ui(egui::RawInput::default(), |ui| {
        let mut value = String::from("alpha");
        ui.add(egui::TextEdit::singleline(&mut value).id(filter))
            .request_focus();
    })
    .textures_delta
    .clear();
    ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::ArrowDown,
                physical_key: Some(Key::ArrowDown),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        },
        |_| {
            assert!(!leave_filter_with_down(&ctx, 0));
            assert!(leave_filter_with_down(&ctx, 1));
            assert!(!ctx.memory(|memory| memory.has_focus(filter)));
            assert!(!ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown)));
        },
    )
    .textures_delta
    .clear();
}

#[test]
fn file_action_shortcuts_require_ctrl_shift_and_have_distinct_commands() {
    let chord = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::NONE
    };
    for (key, command, label) in [
        (Key::E, Command::Rename, "Ctrl Shift E"),
        (Key::N, Command::NewFolder, "Ctrl Shift N"),
        (Key::R, Command::Refresh, "Ctrl Shift R"),
        (Key::A, Command::CreateZip, "Ctrl Shift A"),
    ] {
        assert!(
            COMMANDS
                .iter()
                .any(|(action, _, shortcut)| *action == command && *shortcut == label)
        );
        for modifiers in [
            chord,
            Modifiers::CTRL,
            Modifiers::SHIFT,
            Modifiers::NONE,
            Modifiers { alt: true, ..chord },
        ] {
            let ctx = egui::Context::default();
            let expected = (modifiers == chord).then_some(command);
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
                    assert_eq!(consume_file_action(&ctx), expected);
                    assert_eq!(consume_file_action(&ctx), None);
                    assert_eq!(
                        ctx.input(|input| input.events.len()),
                        usize::from(expected.is_none())
                    );
                },
            )
            .textures_delta
            .clear();
        }
    }
}

#[test]
fn file_page_keys_match_control_arrows_and_do_not_capture_modified_page_keys() {
    for (key, modifiers, expected) in [
        (Key::PageUp, Modifiers::NONE, Some(false)),
        (Key::PageDown, Modifiers::NONE, Some(true)),
        (Key::ArrowUp, Modifiers::CTRL, Some(false)),
        (Key::ArrowDown, Modifiers::CTRL, Some(true)),
        (Key::PageUp, Modifiers::CTRL, None),
        (Key::PageDown, Modifiers::SHIFT, None),
        (Key::PageUp, Modifiers::ALT, None),
        (Key::ArrowDown, Modifiers::NONE, None),
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
                assert_eq!(consume_file_jump(&ctx), expected);
                assert_eq!(
                    ctx.input(|input| input.events.len()),
                    usize::from(expected.is_none())
                );
            },
        )
        .textures_delta
        .clear();
    }
    let mut pane = Pane::new(PathBuf::from("fixture"));
    pane.entries = (1..=25)
        .map(|index| entry(&format!("file-{index:02}")))
        .collect();
    pane.rebuild();
    pane.selected.insert(PathBuf::from("file-03"));
    for (down, expected) in [
        (true, 10),
        (true, 20),
        (true, 25),
        (false, 15),
        (false, 5),
        (false, 0),
    ] {
        pane.jump(down);
        assert_eq!(pane.focus_position(), expected);
        assert_eq!(pane.anchor, expected);
        assert!(pane.scroll_to_focus);
        assert_eq!(pane.selected, BTreeSet::from([PathBuf::from("file-03")]));
    }
}

#[test]
fn help_uses_text_character_instead_of_a_layout_specific_physical_key() {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![
            egui::Event::Text("?".into()),
            egui::Event::Text("unchanged".into()),
        ],
        ..Default::default()
    };
    ctx.run_ui(raw, |_| {
        assert!(consume_help(&ctx));
        assert!(!consume_help(&ctx));
        assert!(ctx.input(|input| {
            input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Text(text) if text == "unchanged"))
        }));
    })
    .textures_delta
    .clear();
}

#[test]
fn accessibility_editor_replaces_only_the_target_value_once() {
    let ctx = egui::Context::default();
    let id = Id::new(("path", 0));
    let request = egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::SetValue,
        target_tree: egui::accesskit::TreeId::ROOT,
        target_node: id.accesskit_id(),
        data: Some(egui::accesskit::ActionData::Value(
            "C:\\Czech folder".into(),
        )),
    };
    let raw = egui::RawInput {
        events: vec![egui::Event::AccessKitActionRequest(request)],
        ..Default::default()
    };
    let mut value = "old".to_string();
    let mut unrelated = "preserve".to_string();
    let mut output = ctx.run_ui(raw, |_| {
        assert!(!accessible_text_value(
            &ctx,
            Id::new(("filter", 0)),
            &mut unrelated
        ));
        assert!(accessible_text_value(&ctx, id, &mut value));
        assert!(!accessible_text_value(&ctx, id, &mut value));
    });
    output.textures_delta.clear();
    assert_eq!(value, "C:\\Czech folder");
    assert_eq!(unrelated, "preserve");
}

#[test]
fn prompt_enter_chords_preserve_newlines_and_alt_variants() {
    let ctx = egui::Context::default();
    for (modifiers, plain, preview) in [
        (Modifiers::NONE, true, false),
        (Modifiers::SHIFT, false, false),
        (Modifiers::CTRL, false, true),
        (Modifiers::CTRL | Modifiers::SHIFT, false, false),
        (Modifiers::ALT, false, false),
        (Modifiers::CTRL | Modifiers::ALT, false, false),
    ] {
        ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |_| {
                assert_eq!(consume_prompt_key(&ctx, Modifiers::NONE, Key::Enter), plain);
                assert_eq!(
                    consume_prompt_key(&ctx, Modifiers::CTRL, Key::Enter),
                    preview
                );
                assert_eq!(
                    ctx.input(|input| input.events.len()),
                    usize::from(!plain && !preview)
                );
            },
        )
        .textures_delta
        .clear();
    }
}

#[test]
fn clipboard_copy_translation_routes_only_the_file_chord() {
    let chord = Modifiers {
        ctrl: true,
        command: true,
        shift: true,
        ..Modifiers::NONE
    };
    for (enabled, shift, expected) in [
        (true, true, true),
        (true, false, false),
        (false, true, false),
    ] {
        let ctx = egui::Context::default();
        let modifiers = Modifiers { shift, ..chord };
        let raw = egui::RawInput {
            events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Copy],
            ..Default::default()
        };
        let mut previous = modifiers;
        let mut output = ctx.run_ui(raw, |_| {
            assert_eq!(consume_file_copy(&ctx, &mut previous, enabled), expected);
            assert_eq!(
                ctx.input(|input| input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Copy))),
                !expected
            );
        });
        output.textures_delta.clear();
    }
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![
            egui::Event::ModifiersChanged(chord),
            egui::Event::Copy,
            egui::Event::ModifiersChanged(Modifiers::NONE),
        ],
        ..Default::default()
    };
    let mut previous = Modifiers::NONE;
    let mut output = ctx.run_ui(raw, |_| {
        assert!(consume_file_copy(&ctx, &mut previous, true));
    });
    output.textures_delta.clear();
}

#[test]
fn range_navigation_uses_key_event_modifiers_after_shift_release() {
    for key in [Key::ArrowUp, Key::ArrowDown, Key::Home, Key::End] {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            events: vec![
                egui::Event::ModifiersChanged(Modifiers::SHIFT),
                egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::SHIFT,
                },
                egui::Event::ModifiersChanged(Modifiers::NONE),
            ],
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |_| {
            assert_eq!(ctx.input(|input| input.modifiers), Modifiers::NONE);
            assert_eq!(consume_navigation_key(&ctx, key), Some(Modifiers::SHIFT));
            assert_eq!(consume_navigation_key(&ctx, key), None);
        });
        output.textures_delta.clear();
    }
}

#[test]
fn navigation_does_not_inherit_later_shift_or_consume_other_chords() {
    for modifiers in [Modifiers::NONE, Modifiers::CTRL, Modifiers::ALT] {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            events: vec![
                egui::Event::Key {
                    key: Key::ArrowDown,
                    physical_key: Some(Key::ArrowDown),
                    pressed: true,
                    repeat: false,
                    modifiers,
                },
                egui::Event::ModifiersChanged(Modifiers::SHIFT),
            ],
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |_| {
            assert_eq!(ctx.input(|input| input.modifiers), Modifiers::SHIFT);
            let expected = (modifiers == Modifiers::NONE).then_some(Modifiers::NONE);
            assert_eq!(consume_navigation_key(&ctx, Key::ArrowDown), expected);
            assert_eq!(
                ctx.input(|input| input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))),
                expected.is_none()
            );
        });
        output.textures_delta.clear();
    }
}

#[test]
fn escape_clears_filter_from_field_or_files_without_clearing_selection() {
    for filter_focused in [false, true] {
        let ctx = egui::Context::default();
        let mut pane = Pane::new(PathBuf::from("fixture"));
        pane.entries = vec![entry("alpha.txt"), entry("beta.txt")];
        pane.filter = "alpha".into();
        pane.selected.insert(PathBuf::from("beta.txt"));
        pane.focused = Some(PathBuf::from("alpha.txt"));
        pane.rebuild();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let response = ui
                .add(egui::TextEdit::singleline(&mut pane.filter).id(Id::new(("filter", 0usize))));
            if filter_focused {
                response.request_focus();
            }
        })
        .textures_delta
        .clear();
        ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Escape,
                    physical_key: Some(Key::Escape),
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |_| {
                assert!(clear_filter_on_escape(&ctx, &mut pane, 0));
                assert!(pane.filter.is_empty());
                assert_eq!(pane.visible.len(), 2);
                assert_eq!(pane.focused, Some(PathBuf::from("alpha.txt")));
                assert!(pane.selected.contains(&PathBuf::from("beta.txt")));
                assert!(!ctx.memory(|memory| memory.has_focus(Id::new(("filter", 0usize)))));
                assert!(!ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)));
            },
        )
        .textures_delta
        .clear();
    }
}

#[test]
fn empty_filter_does_not_consume_escape_for_selection_or_dialogs() {
    let ctx = egui::Context::default();
    let mut pane = Pane::new(PathBuf::from("fixture"));
    ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: Some(Key::Escape),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        },
        |_| {
            assert!(!clear_filter_on_escape(&ctx, &mut pane, 0));
            assert!(ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)));
        },
    )
    .textures_delta
    .clear();
}

#[test]
fn undo_shortcut_is_ctrl_shift_u_and_reuses_no_other_chord() {
    let chord = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::NONE
    };
    assert!(
        COMMANDS
            .iter()
            .any(|(command, _, key)| *command == Command::Undo && *key == "Ctrl Shift U")
    );
    for modifiers in [
        chord,
        Modifiers::CTRL,
        Modifiers::SHIFT,
        Modifiers::NONE,
        Modifiers { alt: true, ..chord },
    ] {
        let ctx = egui::Context::default();
        let expected = (modifiers == chord).then_some(Command::Undo);
        ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::U,
                    physical_key: Some(Key::U),
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |_| assert_eq!(consume_file_action(&ctx), expected),
        )
        .textures_delta
        .clear();
    }
    let keys: Vec<&str> = FILE_ACTION_CHORDS
        .iter()
        .chain(&FILE_PANE_CHORDS)
        .map(|(key, _)| key.name())
        // H toggles the rail; P opens Ask.
        .chain(["H", "P"])
        .collect();
    assert_eq!(
        keys.iter().collect::<BTreeSet<_>>().len(),
        keys.len(),
        "{keys:?}"
    );
    assert!(!keys.contains(&"Z") && !keys.contains(&"Y"), "{keys:?}");
    let labels: Vec<&str> = COMMANDS
        .iter()
        .map(|(_, _, key)| *key)
        .filter(|key| !key.is_empty())
        .collect();
    assert_eq!(
        labels.iter().collect::<BTreeSet<_>>().len(),
        labels.len(),
        "{labels:?}"
    );
}
