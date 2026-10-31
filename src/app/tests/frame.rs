use super::*;

#[test]
fn header_controls_stay_right_aligned_at_supported_widths() {
    for width in [880.0, 1000.0, 1240.0, 1600.0] {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, 900.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            egui::Panel::top("test-header")
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(18, 14)))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("[t_]").monospace().size(23.0));
                        let title = ui.heading("Tomas Commander");
                        let right = ui.max_rect().right();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let commands = action_button(ui, "Ask", "Ctrl Space", true);
                            let notifications = action_button(ui, "    ", "", true);
                            let theme = action_button(ui, "Light mode", "", true);
                            let accent = action_button(ui, "Accent: Green", "", true);
                            assert!(commands.rect.right() <= right);
                            assert!(notifications.rect.right() <= commands.rect.left());
                            assert!(theme.rect.right() <= notifications.rect.left());
                            assert!((commands.rect.top() - notifications.rect.top()).abs() < 1.0);
                            assert!(accent.rect.right() <= theme.rect.left());
                            assert!(accent.rect.left() > title.rect.right());
                            assert!((commands.rect.top() - theme.rect.top()).abs() < 1.0);
                            assert!((theme.rect.top() - accent.rect.top()).abs() < 1.0);
                        });
                    });
                });
        });
        output.textures_delta.clear();
    }
}

#[test]
fn action_columns_share_a_baseline_and_do_not_wrap_labels() {
    for width in [400.0, 880.0, 1000.0, 1240.0, 1600.0] {
        let ctx = egui::Context::default();
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.spacing.item_spacing = Vec2::new(8.0, 6.0);
            style.spacing.button_padding = Vec2::new(10.0, 5.0);
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(12.0));
        });
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, 900.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            let mut previous: Option<egui::Rect> = None;
            ui.horizontal_wrapped(|ui| {
                let bounds = ui.max_rect();
                for (label, hint) in [
                    ("Copy", "Ctrl Shift C"),
                    ("Move", "Ctrl Shift M"),
                    ("Delete", "Ctrl Shift D"),
                    ("Refresh", "Ctrl Shift R"),
                    ("Search", "Ctrl Shift F"),
                    ("Run VS Code", ""),
                    ("Run GitHub Copilot CLI", ""),
                    ("Run GitHub Copilot App", ""),
                ] {
                    let response = action_button(ui, label, hint, true);
                    assert!(
                        response.rect.left() >= bounds.left()
                            && response.rect.right() <= bounds.right(),
                        "{width}: {label} outside strip: {:?} in {:?}",
                        response.rect,
                        bounds
                    );
                    if let Some(previous) = previous {
                        if response.rect.left() < previous.right() {
                            assert!(
                                response.rect.top() >= previous.bottom() + 12.0,
                                "{width}: {label} overlaps the previous row's hints"
                            );
                        } else {
                            assert!(
                                (response.rect.top() - previous.top()).abs() < 1.0,
                                "{width}: {label} has a different baseline"
                            );
                        }
                    }
                    assert!(
                        response.rect.height() <= 28.0,
                        "{label} unexpectedly wrapped"
                    );
                    previous = Some(response.rect);
                }
            });
        });
        output.textures_delta.clear();
    }
}
