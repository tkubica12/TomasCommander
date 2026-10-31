use eframe::egui::{self, Color32, FontId, RichText, text::LayoutJob};
use tomas_commander::text_preview::{Language, Token};

pub struct TextView {
    pub source: bool,
    wrap: bool,
    numbers: bool,
    autoformat: bool,
    copied: bool,
}

impl Default for TextView {
    fn default() -> Self {
        Self {
            source: false,
            wrap: false,
            numbers: true,
            autoformat: true,
            copied: false,
        }
    }
}

impl TextView {
    pub fn markdown_tools(&mut self, ui: &mut egui::Ui, source: &str) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.source, false, "Rendered");
            ui.selectable_value(&mut self.source, true, "Source");
            if ui.button("Copy source").clicked() {
                ui.ctx().copy_text(source.to_owned());
                self.copied = true;
            }
            if self.copied {
                ui.label(RichText::new("Copied").small().weak());
            }
        });
    }

    pub fn code_tools<'a>(
        &mut self,
        ui: &mut egui::Ui,
        source: &'a str,
        formatted: Option<&'a str>,
        language: Language,
    ) -> &'a str {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(language.label()).small().weak());
            if formatted.is_some() {
                ui.checkbox(&mut self.autoformat, "Autoformat")
                    .on_hover_text("Format JSON for this preview only; the file remains unchanged");
            }
            ui.checkbox(&mut self.wrap, "Wrap");
            ui.add_enabled(
                !self.wrap,
                egui::Checkbox::new(&mut self.numbers, "Line numbers"),
            )
            .on_hover_text("Line numbers are shown when wrapping is off");
            if ui
                .button("Copy")
                .on_hover_text("Copy the displayed text / select text and Ctrl C to copy part")
                .clicked()
            {
                ui.ctx().copy_text(
                    if self.autoformat {
                        formatted.unwrap_or(source)
                    } else {
                        source
                    }
                    .to_owned(),
                );
                self.copied = true;
            }
            if formatted.is_some() && ui.button("Copy original").clicked() {
                ui.ctx().copy_text(source.to_owned());
                self.copied = true;
            }
            if self.copied {
                ui.label(RichText::new("Copied").small().weak());
            }
        });
        if self.autoformat {
            formatted.unwrap_or(source)
        } else {
            source
        }
    }

    pub fn show(&self, ui: &mut egui::Ui, text: &str, language: Language, offset: &mut f32) {
        let area = egui::ScrollArea::both()
            .id_salt(("reader-code", self.source, self.autoformat))
            .auto_shrink([false, false])
            .vertical_scroll_offset(*offset)
            .show(ui, |ui| {
                let font = egui::TextStyle::Monospace.resolve(ui.style());
                let mut layout = highlighted(ui, text, language, &font);
                layout.wrap.max_width = if self.wrap {
                    ui.available_width()
                } else {
                    f32::INFINITY
                };
                let mut layouter = |ui: &egui::Ui, _: &dyn egui::TextBuffer, width: f32| {
                    let mut job = layout.clone();
                    if self.wrap {
                        job.wrap.max_width = width;
                    }
                    ui.fonts_mut(|fonts| fonts.layout_job(job))
                };
                egui::Frame::new()
                    .fill(ui.visuals().extreme_bg_color)
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 12.0;
                            if self.numbers && !self.wrap {
                                let count = text.split('\n').count();
                                let digits = count.to_string().len();
                                let numbers = (1..=count)
                                    .map(|line| format!("{line:>digits$}"))
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(numbers).font(font.clone()).weak(),
                                    )
                                    .selectable(false),
                                );
                            }
                            let mut readonly = text;
                            ui.add(
                                egui::TextEdit::multiline(&mut readonly)
                                    .id(egui::Id::new("reader-source-preview"))
                                    .code_editor()
                                    .font(font)
                                    .frame(egui::Frame::NONE)
                                    .margin(egui::Margin::ZERO)
                                    .cursor_at_end(false)
                                    .desired_width(if self.wrap {
                                        ui.available_width()
                                    } else {
                                        f32::INFINITY
                                    })
                                    .layouter(&mut layouter),
                            );
                        });
                    });
            });
        *offset = area.state.offset.y;
    }
}

#[derive(Default)]
struct Highlighter;

impl egui::cache::ComputerMut<(&str, Language, bool, &FontId), LayoutJob> for Highlighter {
    fn compute(
        &mut self,
        (text, language, dark, font): (&str, Language, bool, &FontId),
    ) -> LayoutJob {
        let mut job = LayoutJob::default();
        for span in tomas_commander::text_preview::spans(text, language) {
            let color = match (span.token, dark) {
                (Token::Comment, true) => Color32::from_rgb(130, 170, 110),
                (Token::Comment, false) => Color32::from_rgb(70, 120, 55),
                (Token::String, true) => Color32::from_rgb(220, 160, 130),
                (Token::String, false) => Color32::from_rgb(155, 60, 30),
                (Token::Number, true) => Color32::from_rgb(180, 210, 165),
                (Token::Number, false) => Color32::from_rgb(35, 110, 75),
                (Token::Keyword, true) => Color32::from_rgb(200, 140, 220),
                (Token::Keyword, false) => Color32::from_rgb(135, 45, 150),
                (Token::Key, true) => Color32::from_rgb(145, 205, 245),
                (Token::Key, false) => Color32::from_rgb(0, 90, 155),
                (Token::Function, true) => Color32::from_rgb(225, 215, 155),
                (Token::Function, false) => Color32::from_rgb(125, 95, 0),
                (_, true) => Color32::from_rgb(215, 220, 225),
                (_, false) => Color32::from_rgb(35, 40, 45),
            };
            job.append(
                &text[span.range],
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color,
                    italics: span.token == Token::Comment,
                    ..Default::default()
                },
            );
        }
        job
    }
}

fn highlighted(ui: &egui::Ui, text: &str, language: Language, font: &FontId) -> LayoutJob {
    ui.ctx().memory_mut(|memory| {
        memory
            .caches
            .cache::<egui::cache::FrameCache<LayoutJob, Highlighter>>()
            .get((text, language, ui.visuals().dark_mode, font))
            .clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::cache::ComputerMut;

    fn frame(
        ctx: &egui::Context,
        view: &mut TextView,
        events: Vec<egui::Event>,
        time: f64,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 420.0),
                )),
                events,
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                let text = view.code_tools(
                    ui,
                    "{\"key\":42}",
                    Some("{\n  \"key\": 42\n}"),
                    Language::Json,
                );
                view.show(ui, text, Language::Json, &mut 0.0);
            },
        );
        output.textures_delta.clear();
        output
    }

    fn pos(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.job.text == label
                {
                    Some(text.pos + text.galley.size() / 2.0)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("Missing code toolbar label {label}"))
    }

    #[test]
    fn preview_toolbar_toggles_format_and_copies_exact_display_or_original() {
        let ctx = egui::Context::default();
        let mut view = TextView::default();
        for (index, (label, expected)) in [
            ("Copy", "{\n  \"key\": 42\n}"),
            ("Copy original", "{\"key\":42}"),
        ]
        .into_iter()
        .enumerate()
        {
            let time = index as f64;
            let output = frame(&ctx, &mut view, vec![], time);
            let point = pos(&output, label);
            frame(
                &ctx,
                &mut view,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                time + 0.1,
            );
            let output = frame(
                &ctx,
                &mut view,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                time + 0.12,
            );
            assert!(output.platform_output.commands.iter().any(
                |command| matches!(command, egui::OutputCommand::CopyText(text) if text == expected)
            ));
        }
        let output = frame(&ctx, &mut view, vec![], 2.0);
        let point = pos(&output, "Autoformat");
        frame(
            &ctx,
            &mut view,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            2.1,
        );
        frame(
            &ctx,
            &mut view,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            2.12,
        );
        assert!(!view.autoformat);
    }

    #[test]
    fn highlighting_preserves_source_and_uses_distinct_accessible_theme_colors() {
        let code = "# comment\ndef hello():\n    return \"číslo\", 42\n";
        for dark in [true, false] {
            let job = Highlighter.compute((code, Language::Python, dark, &FontId::monospace(13.0)));
            assert_eq!(job.text, code);
            let colors: std::collections::HashSet<_> = job
                .sections
                .iter()
                .map(|section| section.format.color)
                .collect();
            assert!(colors.len() >= 5);
        }
    }
}
