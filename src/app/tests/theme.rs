use super::*;

#[test]
fn ctrl_wheel_uses_event_modifiers_and_subpercent_deltas_without_rounding_loss() {
    let ctx = egui::Context::default();
    let mut zoom = 1.0;
    for unit in [
        egui::MouseWheelUnit::Point,
        egui::MouseWheelUnit::Line,
        egui::MouseWheelUnit::Page,
    ] {
        ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::MouseWheel {
                        unit,
                        delta: Vec2::new(0.0, 1.0),
                        phase: egui::TouchPhase::Move,
                        modifiers: Modifiers::CTRL,
                    },
                    egui::Event::ModifiersChanged(Modifiers::NONE),
                ],
                ..Default::default()
            },
            |_| {
                zoom = consume_ui_zoom(&ctx, 1.0);
                assert!(zoom > 1.0);
                assert!(!ctx.input(|i| {
                    i.events
                        .iter()
                        .any(|e| matches!(e, egui::Event::MouseWheel { .. }))
                }));
                assert_eq!(ctx.input(|i| i.smooth_scroll_delta), Vec2::ZERO);
            },
        )
        .textures_delta
        .clear();
    }
    ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: Vec2::new(0.0, -1.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::CTRL,
            }],
            ..Default::default()
        },
        |_| assert!(consume_ui_zoom(&ctx, 1.0) < 1.0),
    )
    .textures_delta
    .clear();
    ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: Vec2::new(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        },
        |_| {
            assert_eq!(consume_ui_zoom(&ctx, 1.0), 1.0);
            assert!(ctx.input(|i| {
                i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::MouseWheel { .. }))
            }));
        },
    )
    .textures_delta
    .clear();
}

#[test]
fn zoom_shortcuts_are_global_bounded_and_leave_unrelated_text_alone() {
    let ctx = egui::Context::default();
    let input = |key, modifiers| egui::RawInput {
        events: vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }],
        ..Default::default()
    };
    let mut zoom = 1.0;
    ctx.run_ui(input(Key::Plus, Modifiers::CTRL | Modifiers::SHIFT), |_| {
        zoom = consume_ui_zoom(&ctx, zoom);
        assert!(ctx.input(|input| input.events.is_empty()));
    })
    .textures_delta
    .clear();
    assert!((zoom - 1.1).abs() < 0.001);
    ctx.run_ui(input(Key::Equals, Modifiers::CTRL), |_| {
        zoom = consume_ui_zoom(&ctx, Preferences::MAX_ZOOM);
    })
    .textures_delta
    .clear();
    assert_eq!(zoom, Preferences::MAX_ZOOM);
    ctx.run_ui(input(Key::Minus, Modifiers::CTRL), |_| {
        zoom = consume_ui_zoom(&ctx, Preferences::MIN_ZOOM);
    })
    .textures_delta
    .clear();
    assert_eq!(zoom, Preferences::MIN_ZOOM);
    ctx.run_ui(input(Key::Num0, Modifiers::CTRL), |_| {
        zoom = consume_ui_zoom(&ctx, zoom);
    })
    .textures_delta
    .clear();
    assert_eq!(zoom, 1.0);
    ctx.run_ui(input(Key::Plus, Modifiers::NONE), |_| {
        assert_eq!(consume_ui_zoom(&ctx, 1.0), 1.0);
        assert_eq!(ctx.input(|input| input.events.len()), 1);
    })
    .textures_delta
    .clear();
}

#[test]
fn ctrl_wheel_scales_entire_ui_instead_of_scrolling() {
    let ctx = egui::Context::default();
    let mut zoom = 1.0;
    ctx.run_ui(
        egui::RawInput {
            events: vec![
                egui::Event::ModifiersChanged(Modifiers::CTRL),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: Vec2::new(0.0, 1.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Modifiers::CTRL,
                },
            ],
            ..Default::default()
        },
        |_| {
            zoom = consume_ui_zoom(&ctx, 1.0);
            ctx.set_zoom_factor(zoom);
            assert_eq!(ctx.input(|input| input.smooth_scroll_delta), Vec2::ZERO);
        },
    )
    .textures_delta
    .clear();
    ctx.run_ui(egui::RawInput::default(), |_| {
        assert!((ctx.zoom_factor() - zoom).abs() < 0.001)
    })
    .textures_delta
    .clear();
    assert!(zoom > 1.0);
}

#[test]
fn undersized_native_viewports_recover_without_resizing_valid_windows() {
    for (size, expected) in [
        (Vec2::new(381.0, 970.0), Some(Vec2::new(880.0, 970.0))),
        (Vec2::new(400.0, 300.0), Some(Vec2::new(880.0, 560.0))),
        (Vec2::new(880.0, 560.0), None),
        (Vec2::new(1240.0, 800.0), None),
    ] {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .inner_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size));
        let mut output = ctx.run_ui(raw, |_| enforce_minimum_window_size(&ctx));
        let requested = output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .find_map(|command| match command {
                egui::ViewportCommand::InnerSize(size) => Some(*size),
                _ => None,
            });
        assert_eq!(requested, expected);
        output.textures_delta.clear();
    }
}

#[test]
fn global_zoom_does_not_enlarge_an_already_valid_native_window() {
    let ctx = egui::Context::default();
    let mut previous_zoom = 1.0;
    for (requested, size, next_zoom) in [
        (2.0, Vec2::new(1240.0, 800.0), Some(2.0)),
        (2.0, Vec2::new(1240.0, 800.0), None),
        (2.0, Vec2::new(620.0, 400.0), None),
        (1.0, Vec2::new(620.0, 400.0), Some(1.0)),
        (1.0, Vec2::new(620.0, 400.0), None),
        (1.0, Vec2::new(1240.0, 800.0), None),
    ] {
        let mut raw = egui::RawInput::default();
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .inner_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size));
        let mut output = ctx.run_ui(raw, |_| {
            enforce_stable_minimum_window_size(&ctx, &mut previous_zoom, requested);
            if let Some(zoom) = next_zoom {
                ctx.set_zoom_factor(zoom);
            }
        });
        assert!(
            !output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::InnerSize(_)))
        );
        output.textures_delta.clear();
    }
}
