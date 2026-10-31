use super::*;

pub(super) const ACCENTS: [(&str, [u8; 3], [u8; 3]); 4] = [
    ("Blue", [0, 109, 160], [0, 164, 239]),
    ("Red-orange", [188, 58, 22], [242, 80, 34]),
    ("Green", [76, 113, 0], [127, 186, 0]),
    ("Yellow", [128, 91, 0], [255, 185, 0]),
];

pub(super) fn tint(surface: Color32, accent: Color32, amount: u8) -> Color32 {
    let blend = |a: u8, b: u8| {
        ((u16::from(a) * u16::from(255 - amount) + u16::from(b) * u16::from(amount)) / 255) as u8
    };
    Color32::from_rgb(
        blend(surface.r(), accent.r()),
        blend(surface.g(), accent.g()),
        blend(surface.b(), accent.b()),
    )
}

impl Ledger {
    pub(super) fn accent(&self) -> Color32 {
        let (_, light, dark) = ACCENTS[self.preferences.accent];
        let [r, g, b] = if self.preferences.dark { dark } else { light };
        Color32::from_rgb(r, g, b)
    }

    pub(super) fn apply_theme(&self, ctx: &egui::Context) {
        let mut visuals = if self.preferences.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let color = self.accent();
        visuals.selection.bg_fill = color.gamma_multiply(0.18);
        visuals.selection.stroke = Stroke::new(1.0, color);
        visuals.hyperlink_color = color;
        visuals.panel_fill = if self.preferences.dark {
            Color32::from_rgb(25, 25, 25)
        } else {
            Color32::WHITE
        };
        visuals.window_fill = visuals.panel_fill;
        let border = if self.preferences.dark {
            Color32::from_gray(54)
        } else {
            Color32::from_gray(222)
        };
        let muted = if self.preferences.dark {
            Color32::from_gray(181)
        } else {
            Color32::from_gray(86)
        };
        visuals.override_text_color = Some(if self.preferences.dark {
            Color32::from_gray(242)
        } else {
            Color32::from_gray(22)
        });
        visuals.weak_text_color = Some(muted);
        visuals.faint_bg_color = if self.preferences.dark {
            Color32::from_gray(31)
        } else {
            Color32::from_gray(249)
        };
        visuals.extreme_bg_color = if self.preferences.dark {
            Color32::from_gray(16)
        } else {
            Color32::from_gray(250)
        };
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border);
        visuals.widgets.inactive.bg_fill = visuals.panel_fill;
        visuals.widgets.inactive.weak_bg_fill = visuals.panel_fill;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, border);
        visuals.widgets.hovered.bg_fill = visuals.faint_bg_color;
        visuals.widgets.hovered.weak_bg_fill = visuals.faint_bg_color;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, muted);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, color);
        let theme = if self.preferences.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        };
        ctx.set_theme(theme);
        ctx.set_visuals(visuals);
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = Vec2::new(8.0, 6.0);
            style.spacing.button_padding = Vec2::new(10.0, 5.0);
            style.spacing.interact_size.y = 28.0;
            style.spacing.scroll.bar_width = 8.0;
            style.spacing.scroll.bar_inner_margin = 2.0;
            style.spacing.scroll.bar_outer_margin = 2.0;
            style.spacing.scroll.floating = false;
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(12.0));
            style
                .text_styles
                .insert(egui::TextStyle::Monospace, egui::FontId::monospace(12.0));
            style
                .text_styles
                .insert(egui::TextStyle::Small, egui::FontId::proportional(11.0));
            style
                .text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(18.0));
        });
    }

    pub(super) fn set_zoom(&mut self, zoom: f32) {
        let zoom = zoom.clamp(Preferences::MIN_ZOOM, Preferences::MAX_ZOOM);
        if Preferences::valid_zoom(zoom) && self.preferences.zoom != zoom {
            self.preferences.zoom = zoom;
            self.save_pending = true;
        }
    }

    pub(super) fn zoom_input(&mut self, ctx: &egui::Context) {
        let requested = consume_ui_zoom(ctx, self.preferences.zoom);
        self.set_zoom(requested);
        if (ctx.zoom_factor() - self.preferences.zoom).abs() > 0.001 {
            ctx.set_zoom_factor(self.preferences.zoom);
        }
    }
}

pub(super) fn consume_ui_zoom(ctx: &egui::Context, current: f32) -> f32 {
    ctx.input_mut(|input| {
        let mut delta = 0.0;
        let mut reset = false;
        let mut wheel = 1.0;
        input.events.retain(|event| {
            if let egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } = event
                && modifiers.ctrl
                && !modifiers.alt
                && !modifiers.mac_cmd
            {
                let speed = match unit {
                    egui::MouseWheelUnit::Line => 0.1,
                    egui::MouseWheelUnit::Point => 0.002,
                    egui::MouseWheelUnit::Page => 0.2,
                };
                wheel *= ((delta.x + delta.y) * speed).exp();
                input.smooth_scroll_delta = Vec2::ZERO;
                return false;
            }
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
                && modifiers.ctrl
                && !modifiers.alt
                && !modifiers.mac_cmd
            {
                match key {
                    Key::Plus | Key::Equals => delta += 0.1,
                    Key::Minus => delta -= 0.1,
                    Key::Num0 => reset = true,
                    _ => return true,
                }
                return false;
            }
            true
        });
        if reset {
            1.0
        } else {
            (current * wheel + delta).clamp(Preferences::MIN_ZOOM, Preferences::MAX_ZOOM)
        }
    })
}

pub(super) fn enforce_minimum_window_size(ctx: &egui::Context) {
    let minimum = Vec2::from(MIN_WINDOW_SIZE) / ctx.zoom_factor();
    if let Some(size) = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.size()))
        && (size.x < minimum.x || size.y < minimum.y)
    {
        // Windows tiling tools can override the native minimum-size hint.
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.max(minimum)));
    }
}

pub(super) fn enforce_stable_minimum_window_size(
    ctx: &egui::Context,
    previous_zoom: &mut f32,
    requested_zoom: f32,
) {
    let zoom = ctx.zoom_factor();
    // Native viewport rectangles can still use the previous scale on the transition pass.
    if zoom == *previous_zoom && zoom == requested_zoom {
        enforce_minimum_window_size(ctx);
    }
    *previous_zoom = zoom;
}
