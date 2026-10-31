use super::*;

#[test]
fn drops_prepare_copy_move_and_zip_requests_without_execution() {
    use tomas_commander::archive_changes::Request;
    let sources = vec![
        PathBuf::from(r"C:\root\alpha.txt"),
        PathBuf::from(r"C:\root\beta.txt"),
    ];
    let destination = PathBuf::from(r"C:\destination");
    let archive = PathBuf::from(r"C:\root\sample.zip");
    for copy in [false, true] {
        let job = drop_work(
            7,
            DragItems::Files(sources.clone()),
            DropLocation::Folder(destination.clone()),
            copy,
        )
        .unwrap()
        .unwrap();
        assert!(
            matches!(job, Work::Prepare { id: 7, operation, sources: paths, destination: target }
            if operation == if copy { Operation::Copy } else { Operation::Move } && paths == sources && target == destination)
        );
        let job = drop_work(
            8,
            DragItems::Files(sources.clone()),
            DropLocation::Zip {
                archive: archive.clone(),
                prefix: "nested/".into(),
            },
            copy,
        )
        .unwrap()
        .unwrap();
        assert!(
            matches!(job, Work::PrepareArchive { id: 8, request: Request::Insert { archive: target, prefix, sources: paths, move_sources } }
            if target == archive && prefix == "nested/" && paths == sources && move_sources == !copy)
        );
        let job = drop_work(
            9,
            DragItems::Zip {
                archive: archive.clone(),
                prefix: "nested/".into(),
                selected: vec!["nested/member.txt".into()],
            },
            DropLocation::Folder(destination.clone()),
            copy,
        )
        .unwrap()
        .unwrap();
        assert!(
            matches!(job, Work::PrepareArchive { id: 9, request: Request::Extract { archive: source, prefix, selected, destination: target, move_entries } }
            if source == archive && prefix == "nested/" && selected == ["nested/member.txt"] && target == destination && move_entries == !copy)
        );
    }
    assert!(
        drop_work(
            10,
            DragItems::Files(sources.clone()),
            DropLocation::Folder(PathBuf::from(r"C:\root")),
            false
        )
        .unwrap()
        .is_none()
    );
    assert!(
        drop_work(
            11,
            DragItems::Files(vec![destination.clone()]),
            DropLocation::Folder(destination),
            false
        )
        .unwrap()
        .is_none()
    );
    assert!(
        drop_work(
            12,
            DragItems::Files(Vec::new()),
            DropLocation::Folder(PathBuf::from(r"C:\other")),
            true
        )
        .is_err()
    );
    assert!(
        drop_work(
            13,
            DragItems::Zip {
                archive: archive.clone(),
                prefix: String::new(),
                selected: vec!["member.txt".into()]
            },
            DropLocation::Zip {
                archive,
                prefix: String::new()
            },
            true
        )
        .is_err()
    );
}

#[test]
fn selected_files_drag_from_name_or_checkbox_preserves_the_entire_group_and_alt_intent() {
    for checkbox in [false, true] {
        for alt in [false, true] {
            let ctx = egui::Context::default();
            let mut pane = Pane::new(PathBuf::from("root"));
            let first = PathBuf::from("first.txt");
            let second = PathBuf::from("second.txt");
            pane.selected = BTreeSet::from([first.clone(), second.clone()]);
            let expected = vec![first.clone(), second.clone()];
            let modifiers = Modifiers {
                alt,
                ..Modifiers::NONE
            };
            let mut position = egui::pos2(30.0, 30.0);
            let mut checked = true;
            for step in 0..3 {
                let mut events = match step {
                    1 => vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers,
                        },
                    ],
                    2 => vec![egui::Event::PointerMoved(position + Vec2::new(90.0, 60.0))],
                    _ => vec![],
                };
                events.insert(0, egui::Event::ModifiersChanged(modifiers));
                ctx.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let response = if checkbox {
                            ui.add(egui::Checkbox::without_text(&mut checked))
                                .interact(egui::Sense::click_and_drag())
                        } else {
                            let (rect, _) = ui.allocate_exact_size(
                                Vec2::new(300.0, ROW_HEIGHT),
                                egui::Sense::hover(),
                            );
                            file_name_label(ui, rect, "first.txt")
                        };
                        position = response.rect.center();
                        if response.drag_started_by(egui::PointerButton::Primary) {
                            start_file_drag(&ctx, pane.drag_targets(&first));
                        }
                    },
                )
                .textures_delta
                .clear();
            }
            let payload = egui::DragAndDrop::payload::<DragItems>(&ctx);
            assert!(
                matches!(payload.as_deref(), Some(DragItems::Files(paths)) if paths == &expected)
            );
            assert!(checked);
            assert_eq!(pane.selected.len(), 2);
            assert_eq!(
                ctx.data(|data| data.get_temp::<bool>(Id::new("file-drag-reveal-target"))),
                Some(alt)
            );
            assert_eq!(drag_reveal_requested(&ctx), alt);
            let pointer = ctx.input(|input| input.pointer.interact_pos());
            for late_alt in [true, false] {
                ctx.run_ui(
                    egui::RawInput {
                        events: vec![egui::Event::ModifiersChanged(Modifiers {
                            alt: late_alt,
                            ..Modifiers::NONE
                        })],
                        ..Default::default()
                    },
                    |_| assert_eq!(drag_reveal_requested(&ctx), alt || late_alt),
                )
                .textures_delta
                .clear();
                assert_eq!(ctx.input(|input| input.pointer.interact_pos()), pointer);
                let payload = egui::DragAndDrop::payload::<DragItems>(&ctx);
                assert!(
                    matches!(payload.as_deref(), Some(DragItems::Files(paths)) if paths == &expected)
                );
            }
            let modifiers = Modifiers {
                alt: true,
                ..Modifiers::NONE
            };
            ctx.run_ui(
                egui::RawInput {
                    events: vec![
                        egui::Event::ModifiersChanged(modifiers),
                        egui::Event::PointerButton {
                            pos: pointer.unwrap(),
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers,
                        },
                    ],
                    ..Default::default()
                },
                |_| assert_eq!(drag_reveal_requested(&ctx), alt),
            )
            .textures_delta
            .clear();
        }
    }
}

#[test]
fn drag_bubble_labels_contain_only_the_dragged_contents() {
    let first = PathBuf::from("folder").join("presentation.txt");
    let second = PathBuf::from("folder").join("notes.txt");
    assert_eq!(
        DragItems::Files(vec![first.clone()]).label(),
        "presentation.txt"
    );
    assert_eq!(DragItems::Files(vec![first, second]).label(), "2 items");
    assert_eq!(
        DragItems::Zip {
            archive: PathBuf::from("bundle.zip"),
            prefix: "nested/".into(),
            selected: vec!["nested/slides.pdf".into()],
        }
        .label(),
        "nested/slides.pdf"
    );
}

#[test]
fn drag_uses_selected_group_only_when_grabbed_item_is_selected() {
    let mut pane = Pane::new(PathBuf::from("root"));
    let a = PathBuf::from("a");
    let b = PathBuf::from("b");
    let c = PathBuf::from("c");
    pane.selected = BTreeSet::from([a.clone(), b.clone()]);
    assert_eq!(pane.drag_targets(&a), vec![a.clone(), b]);
    assert_eq!(pane.drag_targets(&c), vec![c]);
    assert_eq!(pane.selected.len(), 2);
}

#[test]
fn drop_folder_row_wins_over_pane_background_in_either_order() {
    let ctx = egui::Context::default();
    let root = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::splat(400.0));
    let row = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), Vec2::splat(100.0));
    for reverse in [false, true] {
        ctx.run_ui(egui::RawInput {
            events: vec![egui::Event::PointerMoved(egui::pos2(20.0, 20.0))],
            ..Default::default()
        }, |_| {
            ctx.data_mut(|data| data.remove::<DropHover>(Id::new("file-drop-hover")));
            egui::DragAndDrop::set_payload(&ctx, DragItems::Files(vec![PathBuf::from("source")]));
            let parent = || register_drop_zone(&ctx, root, DropLocation::Folder(PathBuf::from("parent")), 0);
            let child = || register_drop_zone(&ctx, row, DropLocation::Folder(PathBuf::from("child")), 2);
            if reverse { child(); parent(); } else { parent(); child(); }
            let hover = ctx.data(|data| data.get_temp::<DropHover>(Id::new("file-drop-hover"))).unwrap();
            assert!(matches!(hover.location, DropLocation::Folder(path) if path == std::path::Path::new("child")));
        }).textures_delta.clear();
    }
}
