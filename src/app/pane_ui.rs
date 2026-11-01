use super::keyboard::side_button_history;
use super::*;

pub(crate) fn reveal_row(offset: f32, position: usize, viewport_height: f32) -> f32 {
    let top = position as f32 * ROW_HEIGHT;
    let bottom = top + ROW_HEIGHT;
    if top < offset {
        top
    } else if bottom > offset + viewport_height {
        (bottom - viewport_height).max(0.0)
    } else {
        offset
    }
}

pub(crate) fn paint_file_row(
    ui: &egui::Ui,
    rect: egui::Rect,
    position: usize,
    focused: bool,
    selected: bool,
    active: bool,
    hovered: bool,
) {
    let surface = ui.visuals().panel_fill;
    let accent = ui.visuals().selection.stroke.color;
    let fill = if selected {
        tint(surface, accent, if focused && active { 40 } else { 28 })
    } else if focused || hovered || position % 2 == 1 {
        ui.visuals().faint_bg_color
    } else {
        surface
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    if focused {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            0.0,
            if active {
                Stroke::new(1.0, accent)
            } else {
                ui.visuals().widgets.noninteractive.bg_stroke
            },
            egui::StrokeKind::Inside,
        );
    }
}

pub(super) struct RowColumns {
    pub(super) name: egui::Rect,
    pub(super) size: egui::Rect,
    pub(super) age: egui::Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TabAction {
    Activate(usize),
    Close(usize),
    New,
}

pub(super) struct TabLabel {
    pub(super) name: String,
    pub(super) hover: String,
    /// Folder targeted by drops on this label; `None` while the tab shows a ZIP.
    pub(super) drop: Option<PathBuf>,
}

impl TabLabel {
    pub(super) fn folder(path: &std::path::Path) -> Self {
        Self {
            name: folder_label(path),
            hover: files::display_path(path),
            drop: Some(path.to_owned()),
        }
    }
}

pub(super) struct TabStrip {
    pub(super) action: Option<TabAction>,
    // Geometry is read by the layout tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) rect: egui::Rect,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) tabs: Vec<egui::Rect>,
}

const TAB_HEIGHT: f32 = 20.0;
const TAB_CLOSE_WIDTH: f32 = 16.0;
const TAB_GAP: f32 = 2.0;

/// One compact row of tabs with close buttons and "+". Hidden (`None`) for a single tab.
/// Tabs shrink to fit the pane and scroll horizontally beyond that, so the strip never
/// widens the pane.
pub(super) fn tab_strip(
    ui: &mut egui::Ui,
    pane: usize,
    labels: &[TabLabel],
    active: usize,
    accent: Color32,
) -> Option<TabStrip> {
    if labels.len() < 2 {
        return None;
    }
    let font = egui::TextStyle::Small.resolve(ui.style());
    let width = ui.available_width().max(1.0);
    let plus_width = TAB_HEIGHT + 2.0;
    let strip_width = (width - plus_width - TAB_GAP).max(0.0);
    let slot = (strip_width / labels.len() as f32 - TAB_CLOSE_WIDTH - TAB_GAP).clamp(28.0, 150.0);
    let id = Id::new(("tab-strip", pane));
    let reveal = ui.data_mut(|data| {
        let previous = data.get_temp::<usize>(id);
        data.insert_temp(id, active);
        previous != Some(active)
    });
    let mut action = None;
    let mut tabs = Vec::new();
    let response = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(TAB_GAP, 0.0);
        ui.spacing_mut().scroll.floating = true;
        egui::ScrollArea::horizontal()
            .id_salt(id)
            .max_width(strip_width)
            .min_scrolled_width(0.0)
            .auto_shrink([true, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, label) in labels.iter().enumerate() {
                        let selected = index == active;
                        let natural = ui
                            .painter()
                            .layout_no_wrap(label.name.clone(), font.clone(), Color32::WHITE)
                            .size()
                            .x
                            + 12.0;
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::new(natural.min(slot) + TAB_CLOSE_WIDTH, TAB_HEIGHT),
                            egui::Sense::hover(),
                        );
                        let close_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.right() - TAB_CLOSE_WIDTH, rect.top()),
                            rect.max,
                        );
                        let name_rect = rect.with_max_x(close_rect.left());
                        let tab = ui
                            .interact(name_rect, id.with(("tab", index)), egui::Sense::click())
                            .on_hover_text(label.hover.as_str());
                        let close = ui
                            .interact(close_rect, id.with(("close", index)), egui::Sense::click())
                            .on_hover_text("Close tab / Ctrl W / middle-click");
                        tab.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::Button,
                                ui.is_enabled(),
                                selected,
                                format!("Tab {}", label.name),
                            )
                        });
                        close.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                ui.is_enabled(),
                                format!("Close tab {}", label.name),
                            )
                        });
                        let visuals = ui.visuals();
                        let fill = if selected {
                            tint(visuals.panel_fill, accent, 40)
                        } else if tab.hovered() || close.hovered() {
                            visuals.faint_bg_color
                        } else {
                            visuals.extreme_bg_color
                        };
                        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                        painter.rect_filled(rect, 2.0, fill);
                        if selected {
                            painter.hline(
                                rect.x_range(),
                                rect.bottom() - 1.0,
                                Stroke::new(2.0, accent),
                            );
                        }
                        let color = if selected {
                            accent
                        } else {
                            visuals.text_color()
                        };
                        let mut job = egui::text::LayoutJob::simple_singleline(
                            label.name.clone(),
                            font.clone(),
                            color,
                        );
                        job.wrap = egui::text::TextWrapping::truncate_at_width(
                            (name_rect.width() - 8.0).max(0.0),
                        );
                        let galley = painter.layout_job(job);
                        painter.galley(
                            egui::pos2(
                                name_rect.left() + 6.0,
                                name_rect.center().y - galley.size().y / 2.0,
                            ),
                            galley,
                            color,
                        );
                        painter.text(
                            close_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "\u{d7}",
                            font.clone(),
                            if close.hovered() {
                                visuals.text_color()
                            } else {
                                visuals.weak_text_color()
                            },
                        );
                        if let Some(folder) = &label.drop {
                            register_drop_zone(
                                ui.ctx(),
                                rect.intersect(ui.clip_rect()),
                                DropLocation::Folder(folder.clone()),
                                1,
                            );
                        }
                        if tab.clicked() {
                            action = Some(TabAction::Activate(index));
                        }
                        if close.clicked()
                            || tab.clicked_by(egui::PointerButton::Middle)
                            || close.clicked_by(egui::PointerButton::Middle)
                        {
                            action = Some(TabAction::Close(index));
                        }
                        if selected && reveal {
                            tab.scroll_to_me(None);
                        }
                        tabs.push(rect);
                    }
                });
            });
        let (rect, plus) =
            ui.allocate_exact_size(Vec2::new(plus_width, TAB_HEIGHT), egui::Sense::click());
        let plus = plus.on_hover_text("New tab / Ctrl T");
        plus.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "New tab")
        });
        let visuals = ui.visuals();
        ui.painter().rect_filled(
            rect,
            2.0,
            if plus.hovered() {
                visuals.faint_bg_color
            } else {
                visuals.extreme_bg_color
            },
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "+",
            font.clone(),
            visuals.text_color(),
        );
        if plus.clicked() {
            action = Some(TabAction::New);
        }
    });
    Some(TabStrip {
        action,
        rect: response.response.rect,
        tabs,
    })
}

impl RowColumns {
    pub(super) fn new(rect: egui::Rect) -> Self {
        if rect.width() < 230.0 {
            let right = rect.right() - 10.0;
            return Self {
                name: egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 56.0, rect.top()),
                    egui::pos2(right.max(rect.left() + 56.0), rect.bottom()),
                ),
                size: egui::Rect::from_min_max(
                    egui::pos2(right, rect.top()),
                    egui::pos2(right, rect.bottom()),
                ),
                age: egui::Rect::from_min_max(
                    egui::pos2(right, rect.top()),
                    egui::pos2(right, rect.bottom()),
                ),
            };
        }
        let age_width = (rect.width() * 0.3).clamp(50.0, 132.0);
        let size_width = (rect.width() * 0.2).clamp(45.0, 82.0);
        let right = rect.right() - 10.0;
        Self {
            name: egui::Rect::from_min_max(
                egui::pos2(rect.left() + 56.0, rect.top()),
                egui::pos2(
                    (right - age_width - size_width - 16.0).max(rect.left() + 56.0),
                    rect.bottom(),
                ),
            ),
            size: egui::Rect::from_min_max(
                egui::pos2(right - age_width - size_width - 8.0, rect.top()),
                egui::pos2(right - age_width - 8.0, rect.bottom()),
            ),
            age: egui::Rect::from_min_max(
                egui::pos2(right - age_width, rect.top()),
                egui::pos2(right, rect.bottom()),
            ),
        }
    }
}

impl Ledger {
    pub(super) fn pane_ui(&mut self, ui: &mut egui::Ui, index: usize) {
        if self.archive.target == Some(index) {
            self.archive_pane_ui(ui, index);
            return;
        }
        if self.reader.target == Some(index) {
            self.reader_pane_ui(ui, index);
            return;
        }
        let mut history_step = side_button_history(ui.ctx(), ui.max_rect());
        let mut history_via = Via::MouseButton;
        let mut header_sort = false;
        if ui.input(|input| {
            input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|position| ui.max_rect().contains(position))
        }) {
            self.active = index;
            self.rail_index = None;
            self.file_focus_requested = false;
            ui.memory_mut(|memory| {
                for pane in 0..2 {
                    memory.surrender_focus(Id::new(("path", pane)));
                    memory.surrender_focus(Id::new(("filter", pane)));
                }
            });
        }
        let accent = self.accent();
        let active = self.active == index;
        let muted = ui.visuals().weak_text_color();
        let stripe = ui.visuals().faint_bg_color;
        let border = ui.visuals().widgets.noninteractive.bg_stroke;
        let mut parent = false;
        let mut go = false;
        let mut pin = false;
        let mut breadcrumb = None;
        let mut row_command = None;
        let mut open = false;
        let mut inline_start = false;
        let mut inline_submit = false;
        let mut inline_cancel = false;
        let drag_allowed = self.drag_allowed() && ui.is_enabled();
        ui.spacing_mut().item_spacing.y = 0.0;
        let pane = &mut self.panes[index];
        let editable = self.plan.is_none()
            && self.error.is_none()
            && !self.palette
            && self.command_dialog.is_none()
            && self.proposal_review.is_none();
        if editable {
            accessible_text_value(ui.ctx(), Id::new(("path", index)), &mut pane.path_text);
            if accessible_text_value(ui.ctx(), Id::new(("filter", index)), &mut pane.filter) {
                self.active = index;
                pane.rebuild();
            }
        }
        selection_footer(ui, index, pane, active, accent, muted, border);
        let tab_labels = (pane.tabs.len() > 1).then(|| {
            (0..pane.tabs.len())
                .map(|tab| TabLabel::folder(pane.tab_path(tab)))
                .collect::<Vec<_>>()
        });
        let current_tab = pane.tab;
        let can_back = !pane.history.back.is_empty();
        let can_forward = !pane.history.forward.is_empty();
        let mut tab_action = None;
        let heading = egui::Frame::new()
            .fill(stripe)
            .inner_margin(egui::Margin::symmetric(12, 9))
            .show(ui, |ui| {
                if let Some(labels) = &tab_labels
                    && let Some(strip) = tab_strip(ui, index, labels, current_tab, accent)
                {
                    tab_action = strip.action;
                    ui.add_space(6.0);
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(if index == 0 {
                            "01 / LEFT PANE"
                        } else {
                            "02 / RIGHT PANE"
                        })
                        .monospace()
                        .size(11.0)
                        .color(if active {
                            accent
                        } else {
                            ui.visuals().weak_text_color()
                        }),
                    );
                    let narrow = ui.available_width() < 100.0;
                    let layout = if narrow {
                        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true)
                    } else {
                        egui::Layout::right_to_left(egui::Align::Center)
                    };
                    ui.with_layout(layout, |ui| {
                        pin = ui.small_button("+ pin").clicked();
                        parent = ui
                            .small_button("..")
                            .on_hover_text("Parent folder / choose drive at root / Backspace")
                            .clicked();
                        let mut buttons = [
                            (true, can_forward, "\u{203a}", "Forward / Alt Right"),
                            (false, can_back, "\u{2039}", "Back / Alt Left"),
                        ];
                        if narrow {
                            buttons.reverse();
                        }
                        for (forward, enabled, glyph, label) in buttons {
                            let response = ui
                                .add_enabled(enabled, egui::Button::new(glyph).small())
                                .on_hover_text(label);
                            ui.ctx().accesskit_node_builder(response.id, |node| {
                                node.set_label(label);
                            });
                            if response.clicked() {
                                history_step = Some(forward);
                                history_via = Via::Header;
                            }
                        }
                    });
                })
            });
        if active {
            let rect = heading.response.rect;
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
                0.0,
                accent,
            );
        }
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                if !self.path_edits[index] {
                    let (path, edit) =
                        compact_breadcrumbs(ui, &breadcrumb_paths(&pane.path, self.scope.root()));
                    breadcrumb = path;
                    if edit {
                        self.path_edits[index] = true;
                        self.editor_focus = Some(Id::new(("path", index)));
                    }
                } else {
                    let response = trailing_field(
                        ui,
                        egui::TextEdit::singleline(&mut pane.path_text)
                            .id(Id::new(("path", index)))
                            .font(egui::TextStyle::Monospace)
                            .margin(Vec2::new(7.0, 6.0)),
                        |ui| {
                            if ui.small_button("Done").clicked() {
                                self.path_edits[index] = false;
                            }
                            go = ui.button("Go").clicked();
                        },
                    );
                    if response.has_focus() {
                        self.active = index;
                        self.rail_index = None;
                    }
                    if self.editor_focus == Some(response.id) {
                        response.request_focus();
                        self.editor_focus = None;
                    }
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.add_action(egui::accesskit::Action::SetValue)
                    });
                    go |= response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
                }
                let clear = !pane.filter.is_empty();
                let mut clear_clicked = false;
                let response = trailing_field(
                    ui,
                    egui::TextEdit::singleline(&mut pane.filter)
                        .id(Id::new(("filter", index)))
                        .hint_text("Filter this folder...")
                        .font(egui::TextStyle::Monospace)
                        .margin(Vec2::new(7.0, 6.0)),
                    |ui| {
                        clear_clicked = ui
                            .add_enabled(clear, egui::Button::new("x"))
                            .on_hover_text("Clear filter")
                            .clicked();
                    },
                );
                if response.changed() {
                    pane.rebuild();
                }
                ui.ctx().accesskit_node_builder(response.id, |node| {
                    node.set_label(if index == 0 {
                        "Filter left pane"
                    } else {
                        "Filter right pane"
                    });
                    node.add_action(egui::accesskit::Action::SetValue)
                });
                if response.has_focus() {
                    self.active = index;
                    self.rail_index = None;
                }
                if self.editor_focus == Some(response.id) {
                    response.request_focus();
                    self.editor_focus = None;
                }
                if clear_clicked {
                    pane.filter.clear();
                    pane.rebuild();
                    self.file_focus_requested = true;
                }
            });
        let (head, head_response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), egui::Sense::hover());
        head_response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                ui.is_enabled(),
                "Name / Size / Last modified",
            )
        });
        ui.painter().rect_filled(head, 0.0, stripe);
        ui.painter().hline(head.x_range(), head.top(), border);
        ui.painter().hline(head.x_range(), head.bottom(), border);
        let mut head_content = head;
        head_content.max.x -= SCROLL_GUTTER;
        let columns = RowColumns::new(head_content);
        for (rect, text, sort, align) in [
            (columns.name, "Name", Sort::Name, egui::Align2::LEFT_CENTER),
            (columns.size, "Size", Sort::Size, egui::Align2::RIGHT_CENTER),
            (
                columns.age,
                "Last modified",
                Sort::Modified,
                egui::Align2::RIGHT_CENTER,
            ),
        ] {
            if rect.width() == 0.0 {
                continue;
            }
            let label = if pane.sort == sort {
                format!(
                    "{text} {}",
                    if pane.descending {
                        "\u{25bc}"
                    } else {
                        "\u{25b2}"
                    }
                )
            } else {
                text.to_owned()
            };
            let response =
                ui.interact(rect, Id::new(("column", index, text)), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
            });
            if response.clicked() {
                if pane.sort == sort {
                    pane.descending = !pane.descending;
                } else {
                    pane.sort = sort;
                    pane.descending = false;
                }
                self.active = index;
                header_sort = true;
                files::sort_entries_ordered(&mut pane.entries, pane.sort, pane.descending);
                pane.rebuild();
                pane.scroll_to_focus = true;
            }
            ui.painter().with_clip_rect(rect).text(
                if align == egui::Align2::LEFT_CENTER {
                    rect.left_center()
                } else {
                    rect.right_center()
                },
                align,
                label,
                egui::FontId::monospace(10.0),
                muted,
            );
        }
        if let Some(warning) = &pane.warning {
            ui.label(RichText::new("PARTIAL / ERROR").color(accent));
            ui.label(warning);
        }
        let height = ui.available_height().max(0.0);
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("rows", index))
            .auto_shrink([false, false])
            .animated(false)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .max_height(height);
        if pane.scroll_to_focus {
            scroll = scroll.vertical_scroll_offset(reveal_row(
                pane.scroll_offset,
                pane.focus_position(),
                height,
            ));
            pane.scroll_to_focus = false;
        }
        let output = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                scroll.show_rows(ui, ROW_HEIGHT, pane.visible.len() + 1, |ui, range| {
                    for position in range {
                        let navigation = position == 0;
                        let parent_entry = Entry {
                            path: pane.path.clone(),
                            name: "..".into(),
                            directory: true,
                            link: false,
                            size: 0,
                            modified: None,
                            folded: String::new(),
                            aggregate: None,
                        };
                        let entry = if navigation {
                            parent_entry
                        } else {
                            pane.entries[pane.visible[position - 1]].clone()
                        };
                        let path = entry.path.clone();
                        let focused = if navigation {
                            pane.focused.is_none()
                        } else {
                            pane.focused.as_ref() == Some(&path)
                        };
                        let selected = !navigation && pane.selected.contains(&path);
                        let name = entry.name.clone();
                        let icon = if entry.link {
                            "[link]"
                        } else if entry.directory {
                            "[/]"
                        } else {
                            "[.]"
                        };
                        let available_parent = pane.path.parent().is_some()
                            && self.scope.root() != Some(pane.path.as_path());
                        let label = if navigation {
                            if available_parent {
                                "[/] .. / Parent folder"
                            } else if self.scope.root().is_none() {
                                "[/] .. / Choose drive"
                            } else {
                                "[/] .. / Parent unavailable"
                            }
                            .to_string()
                        } else {
                            format!("{icon} {name}")
                        };
                        let aggregate = entry.aggregate.as_ref().and_then(|result| result.as_ref().ok());
                        let partial = aggregate.is_some_and(files::DirectoryStats::partial);
                        let size = if navigation {
                            if available_parent { "<UP>" } else if self.scope.root().is_none() { "<DRIVES>" } else { "<ROOT>" }.into()
                        } else if entry.directory {
                            match &entry.aggregate {
                                None => "...".into(),
                                Some(Err(_)) => "!".into(),
                                Some(Ok(stats)) => format!("{}{}", format_size(stats.size), if partial { "+" } else { "" }),
                            }
                        } else {
                            format_size(entry.size)
                        };
                        let age = if navigation {
                            String::new()
                        } else if entry.directory {
                            match &entry.aggregate {
                                None => "...".into(),
                                Some(Err(_)) => "!".into(),
                                Some(Ok(stats)) => format!("{}{}", format_modified(stats.modified), if partial { "*" } else { "" }),
                            }
                        } else {
                            format_modified(entry.modified)
                        };
                        ui.push_id(&path, |ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), ROW_HEIGHT),
                                egui::Sense::hover(),
                            );
                            let row = egui::Rect::from_min_max(
                                egui::pos2(rect.left() + 30.0, rect.top()),
                                rect.max,
                            );
                            let mut response = ui
                                .interact(row, ui.id().with("file"), if navigation || !drag_allowed { egui::Sense::click() } else { egui::Sense::click_and_drag() })
                                .on_hover_text(format!("{}{}", files::display_path(&path),
                                    match &entry.aggregate {
                                        Some(Err(error)) => format!("\n{error}"),
                                        Some(Ok(stats)) => format!("\n{} items; {} links excluded{}{}{}; modified times are UTC",
                                            stats.examined, stats.skipped_links,
                                            if stats.truncated { "; calculation limit reached" } else { "" },
                                            if stats.warnings.is_empty() { String::new() } else { format!("\n{}", stats.warnings.join("\n")) },
                                            if pane.pending_aggregates.contains(&path) { "\nRefreshing; showing last known size and modification time" } else { "" }),
                                        None if entry.directory && !navigation => "\nCalculating folder size and latest file modification...".into(),
                                        _ => "\nModified times are UTC".into(),
                                    }));
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::Button,
                                    ui.is_enabled(),
                                    selected,
                                    &label,
                                )
                            });
                            if !navigation && drag_allowed && response.drag_started_by(egui::PointerButton::Primary) {
                                self.inline_pending = None;
                                start_file_drag(ui.ctx(), pane.drag_targets(&path));
                                pane.focused = Some(path.clone());
                                self.active = index;
                                self.rail_index = None;
                            }
                            if !navigation && !entry.link && entry.directory {
                                register_drop_zone(ui.ctx(), response.rect.intersect(ui.clip_rect()), DropLocation::Folder(path.clone()), 2);
                            } else if !navigation && !entry.link && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("zip")) {
                                register_drop_zone(ui.ctx(), response.rect.intersect(ui.clip_rect()), DropLocation::Zip { archive: path.clone(), prefix: String::new() }, 2);
                            }
                            if focused && active && editable && self.file_focus_requested {
                                response.request_focus();
                                self.file_focus_requested = false;
                            }
                            ui.ctx().accesskit_node_builder(response.id, |node| {
                                node.set_description(format!(
                                    "{size}, last modified {age}{}",
                                    if focused && active {
                                        ", current file"
                                    } else {
                                        ""
                                    }
                                ));
                            });
                            paint_file_row(ui, rect, position, focused, selected, active, response.hovered());
                            let mut checked = selected;
                            if !navigation {
                                let checkbox = ui
                                    .place(
                                        egui::Rect::from_center_size(
                                            egui::pos2(rect.left() + 16.0, rect.center().y),
                                            Vec2::splat(28.0),
                                        ),
                                        egui::Checkbox::without_text(&mut checked),
                                    )
                                    .on_hover_text(format!("Select {name}"));
                                let checkbox = if drag_allowed {
                                    checkbox.interact(egui::Sense::click_and_drag())
                                } else {
                                    checkbox
                                };
                                checkbox.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::Checkbox,
                                        ui.is_enabled(),
                                        checked,
                                        format!("Select {name}"),
                                    )
                                });
                                if checkbox.changed() {
                                    if checked {
                                        pane.selected.insert(path.clone());
                                    } else {
                                        pane.selected.remove(&path);
                                    }
                                    self.active = index;
                                    self.rail_index = None;
                                    pane.focused = Some(path.clone());
                                    pane.anchor = position;
                                }
                                if drag_allowed && checkbox.drag_started_by(egui::PointerButton::Primary) {
                                    self.inline_pending = None;
                                    start_file_drag(ui.ctx(), pane.drag_targets(&path));
                                    pane.focused = Some(path.clone());
                                    self.active = index;
                                    self.rail_index = None;
                                }
                            }
                            let columns = RowColumns::new(rect);
                            ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).text(
                                egui::pos2(rect.left() + 32.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                if entry.link { "@" } else { icon },
                                egui::FontId::monospace(10.0),
                                if entry.directory { accent } else { muted },
                            );
                            let editing = self.name_dialog.as_mut().filter(|dialog|
                                dialog.inline && dialog.pane == index && dialog.source.as_ref() == Some(&path));
                            if let Some(dialog) = editing {
                                let id = ui.id().with("inline-name");
                                accessible_text_value(ui.ctx(), id, &mut dialog.name);
                                let edit = ui.place(columns.name, egui::TextEdit::singleline(&mut dialog.name)
                                    .id(id).font(egui::TextStyle::Monospace));
                                if dialog.focus { edit.request_focus(); dialog.focus = false; }
                                inline_submit |= inline_name_submitted(ui, &edit);
                                inline_cancel |= ui.input_mut(|input|
                                    input.consume_key(Modifiers::NONE, Key::Escape));
                                inline_cancel |= edit.lost_focus() && ui.input(|input| input.pointer.any_pressed());
                                if let Some(error) = &dialog.error { edit.on_hover_text(error); }
                            } else {
                                let label = file_name_label(ui, columns.name, &name);
                                if !navigation && drag_allowed && label.drag_started_by(egui::PointerButton::Primary) {
                                    self.inline_pending = None;
                                    start_file_drag(ui.ctx(), pane.drag_targets(&path));
                                    pane.focused = Some(path.clone());
                                    self.active = index;
                                    self.rail_index = None;
                                }
                                if !navigation && label.clicked() && ui.input(|input| input.modifiers.is_none()) {
                                    self.inline_pending = inline_rename_click(&label, focused && active)
                                        .map(|started| (index, path.clone(), started));
                                    label.request_focus();
                                    pane.focused = Some(path.clone());
                                    self.active = index;
                                }
                                response = response.union(label);
                            }
                            if !navigation {
                                response.context_menu(|ui| {
                                    copy_path_menu_item(ui, pane, &path);
                                    if !entry.directory && !entry.link && ui.button("Clone").clicked() {
                                        pane.focused = Some(path.clone()); self.active = index;
                                        row_command = Some((Command::Clone, Via::ContextMenu)); ui.close();
                                    }
                                    for (command, label) in [(Command::OpenWith, "Open with"), (Command::Preview, "Internal preview"), (Command::Rename, "Rename"), (Command::Copy, "Copy"), (Command::Move, "Move"), (Command::Recycle, "Recycle")] {
                                        if ui.button(label).clicked() {
                                            if !pane.selected.contains(&path) {
                                                pane.selected.clear();
                                            }
                                            pane.focused = Some(path.clone()); self.active = index;
                                            row_command = Some((command, Via::ContextMenu)); ui.close();
                                        }
                                    }
                                });
                            }
                            for (rect, text) in [(columns.size, &size), (columns.age, &age)] {
                                if rect.width() == 0.0 { continue; }
                                ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).text(
                                    rect.right_center(),
                                    egui::Align2::RIGHT_CENTER,
                                    text,
                                    egui::FontId::monospace(11.0),
                                    muted,
                                );
                            }
                            if response.clicked() || response.secondary_clicked() {
                                response.request_focus();
                                self.active = index;
                                self.rail_index = None;
                                pane.focused = if navigation { None } else { Some(path.clone()) };
                                let modifiers = ui.input(|input| input.modifiers);
                                if navigation {
                                    pane.anchor = 0;
                                } else if modifiers.ctrl && response.clicked() {
                                    if !pane.selected.remove(&path) {
                                        pane.selected.insert(path.clone());
                                    }
                                } else if modifiers.shift {
                                    pane.selected = (pane.anchor.min(position)
                                        ..=pane.anchor.max(position))
                                        .filter_map(|position| pane.row_path(position))
                                        .collect();
                                } else {
                                    pane.anchor = position;
                                }
                            }
                            if response.double_clicked() {
                                inline_start = false;
                                self.inline_pending = None;
                                self.active = index;
                                pane.focused = if navigation { None } else { Some(path.clone()) };
                                if let Some(command) = double_click_command(
                                    navigation, ui.input(|input| input.modifiers),
                                ) {
                                    row_command = Some((command, Via::DoubleClick));
                                } else {
                                    open = true;
                                }
                            }
                        });
                    }
                })
            })
            .inner;
        pane.scroll_offset = output.state.offset.y;
        if !pane.busy {
            register_drop_zone(
                ui.ctx(),
                ui.max_rect(),
                DropLocation::Folder(pane.path.clone()),
                0,
            );
        }
        if let Some(action) = tab_action {
            self.apply_tab_action(index, action);
            return;
        }
        if let Some(forward) = history_step
            && editable
            && ui.is_enabled()
        {
            self.active = index;
            self.rail_index = None;
            let command = if forward {
                Command::Forward
            } else {
                Command::Back
            };
            self.usage_action(command.slash(), history_via, shortcut_hint(command));
            self.step_history(index, forward);
            return;
        }
        if let Some((pending_pane, path, started)) = &self.inline_pending
            && *pending_pane == index
            && ui.input(|input| input.time) - started
                >= ui
                    .ctx()
                    .options(|options| options.input_options.max_double_click_delay)
                    .max(0.4)
            && !ui.input(|input| input.pointer.any_down())
        {
            inline_start = self.active == index
                && self.panes[index].focused.as_ref() == Some(path)
                && self.drag_allowed();
            self.inline_pending = None;
        }
        if self.inline_pending.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
        if inline_cancel {
            self.name_dialog = None;
            self.usage.outcome("inline_rename", "cancelled");
        }
        if inline_submit {
            self.usage.outcome("inline_rename", "confirmed");
            self.prepare_name_change();
        }
        if header_sort {
            self.usage_action("/sort", Via::Header, shortcut_hint(Command::Sort));
        }
        if inline_start {
            self.dispatch(Via::Click, Command::Rename);
            if let Some(dialog) = &mut self.name_dialog {
                dialog.inline = true;
            }
            ui.ctx().request_repaint();
        }
        if go {
            self.path_edits[index] = false;
            self.active = index;
            self.navigate(
                index,
                PathBuf::from(self.panes[index].path_text.trim()),
                NavVia::TypedPath,
            );
        }
        if let Some(path) = breadcrumb {
            self.active = index;
            self.navigate(index, path, NavVia::Breadcrumb);
        }
        if let Some((command, via)) = row_command {
            self.dispatch(via, command);
        }
        if parent {
            self.active = index;
            self.usage_action("/parent", Via::Header, shortcut_hint(Command::Parent));
            self.parent(index, NavVia::Parent);
        }
        if pin {
            self.active = index;
            self.dispatch(Via::Header, Command::Pin);
        }
        if open {
            self.open_focused(NavVia::DoubleClick);
        }
    }

    fn archive_pane_ui(&mut self, ui: &mut egui::Ui, index: usize) {
        ui.set_min_width(ui.available_width());
        if ui.rect_contains_pointer(ui.max_rect()) && ui.input(|input| input.pointer.any_pressed())
        {
            self.active = index;
            self.rail_index = None;
        }
        let pane = &self.panes[index];
        let tab_labels = (pane.tabs.len() > 1).then(|| {
            let (archive, _) = self.archive.archive_location();
            (0..pane.tabs.len())
                .map(|tab| {
                    if tab == pane.tab {
                        TabLabel {
                            name: folder_label(&archive),
                            hover: files::display_path(&archive),
                            drop: None,
                        }
                    } else {
                        TabLabel::folder(pane.tab_path(tab))
                    }
                })
                .collect::<Vec<_>>()
        });
        let current_tab = pane.tab;
        let accent = self.accent();
        let tab_action = tab_labels.and_then(|labels| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(12, 6))
                .show(ui, |ui| tab_strip(ui, index, &labels, current_tab, accent))
                .inner
                .and_then(|strip| strip.action)
        });
        let (close, extract) = self.archive.show_archive(ui, self.active == index);
        if close {
            self.archive.close();
            if self.reader.is_virtual() {
                self.reader.close();
            }
            self.active = index;
            self.file_focus_requested = true;
        }
        if let Some(all) = extract {
            self.active = index;
            if !self.file_actions_allowed() || self.reader.dirty() {
                self.fail("Finish pending work and Save/discard edits before a ZIP operation.");
            } else {
                match self.archive.extraction_request(
                    self.panes[1 - index].path.clone(),
                    all,
                    false,
                ) {
                    Ok(request) => self.prepare_archive(request),
                    Err(error) => self.fail(error),
                }
            }
        }
        let (archive, prefix) = self.archive.archive_location();
        if self.archive.is_archive() {
            register_drop_zone(
                ui.ctx(),
                ui.max_rect(),
                DropLocation::Zip { archive, prefix },
                0,
            );
        }
        if let Some(action) = tab_action {
            self.apply_tab_action(index, action);
        }
    }

    fn reader_pane_ui(&mut self, ui: &mut egui::Ui, index: usize) {
        ui.set_min_width(ui.available_width());
        if ui.is_enabled()
            && ui.rect_contains_pointer(ui.max_rect())
            && ui.input(|input| input.pointer.any_pressed())
        {
            self.active = index;
        }
        if self.reader.show(ui) {
            self.reader.close();
            if self.reader.target.is_none() {
                self.active = self.reader.source;
                self.file_focus_requested = true;
            }
        }
    }
}

pub(crate) fn format_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1e6)
    } else if bytes >= 1000 {
        format!("{:.1} KB", bytes as f64 / 1000.0)
    } else {
        format!("{bytes} B")
    }
}

pub(super) fn breadcrumb_paths(
    path: &std::path::Path,
    boundary: Option<&std::path::Path>,
) -> Vec<PathBuf> {
    let mut paths: Vec<_> = path
        .ancestors()
        .filter(|ancestor| boundary.is_none_or(|root| ancestor.starts_with(root)))
        .map(std::path::Path::to_owned)
        .collect();
    paths.reverse();
    paths
}

pub(super) fn double_click_command(navigation: bool, modifiers: Modifiers) -> Option<Command> {
    if navigation {
        None
    } else if modifiers.ctrl {
        Some(Command::Preview)
    } else if modifiers.shift {
        Some(Command::OpenWith)
    } else {
        None
    }
}

pub(super) fn inline_rename_click(response: &egui::Response, already_current: bool) -> Option<f64> {
    (already_current
        && response.clicked()
        && !response.double_clicked()
        && response.ctx.input(|input| input.modifiers.is_none()))
    .then(|| response.ctx.input(|input| input.time))
}

pub(super) fn inline_name_submitted(ui: &mut egui::Ui, edit: &egui::Response) -> bool {
    (edit.has_focus() || edit.lost_focus())
        && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter))
}

pub(super) fn copy_path_menu_item(
    ui: &mut egui::Ui,
    pane: &Pane,
    path: &PathBuf,
) -> egui::Response {
    let response = ui.button("Copy path");
    if response.clicked() {
        let text = pane
            .drag_targets(path)
            .iter()
            .map(|path| files::display_path(path))
            .collect::<Vec<_>>()
            .join("\r\n");
        ui.ctx().copy_text(text);
        ui.close();
    }
    response
}

pub(super) fn file_name_label(ui: &mut egui::Ui, rect: egui::Rect, name: &str) -> egui::Response {
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            name,
            FILE_NAME_FONT,
            ui.visuals().text_color(),
        );
    let response = ui.interact(
        rect,
        ui.id().with("filename"),
        egui::Sense::click_and_drag(),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), name));
    response.on_hover_cursor(egui::CursorIcon::Text)
}

pub(super) fn compact_breadcrumbs(ui: &mut egui::Ui, paths: &[PathBuf]) -> (Option<PathBuf>, bool) {
    if paths.is_empty() {
        return (None, false);
    }
    let width = ui.available_width();
    let gap = 4.0;
    let font = egui::TextStyle::Small.resolve(ui.style());
    let measure = |label: &str| {
        ui.painter()
            .layout_no_wrap(label.to_owned(), font.clone(), ui.visuals().text_color())
            .size()
            .x
            + ui.spacing().button_padding.x * 2.0
    };
    let edit_width = measure("Edit path").min(width * 0.6);
    let component_max = (width - edit_width - gap).max(1.0).min(width * 0.55);
    let labels: Vec<_> = paths
        .iter()
        .map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| files::display_path(path))
        })
        .collect();
    let widths: Vec<_> = labels
        .iter()
        .map(|label| measure(label).min(component_max))
        .collect();
    let ellipsis_width = measure("...").min(component_max);
    let mut chosen = Vec::new();
    for first in 0..paths.len() {
        let mut items = vec![Some(0)];
        if first > 1 {
            items.push(None);
        }
        items.extend((first.max(1)..paths.len()).map(Some));
        items.push(Some(paths.len()));
        let mut row = 0;
        let mut used = 0.0;
        let mut layout = Vec::new();
        for item in items {
            let item_width = match item {
                None => ellipsis_width,
                Some(i) if i == paths.len() => edit_width,
                Some(i) => widths[i],
            };
            if used > 0.0 && used + gap + item_width > width {
                row += 1;
                used = 0.0;
            }
            used += if used == 0.0 {
                item_width
            } else {
                gap + item_width
            };
            layout.push((row, item, item_width));
        }
        if row < 2 {
            chosen = layout;
            break;
        }
    }
    let mut clicked = None;
    let mut edit = false;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(gap, 2.0);
        for row in 0..2 {
            if !chosen.iter().any(|(r, _, _)| *r == row) {
                continue;
            }
            ui.horizontal_top(|ui| {
                for (_, item, item_width) in chosen.iter().filter(|(r, _, _)| *r == row) {
                    let label = match item {
                        None => "...",
                        Some(i) if *i == paths.len() => "Edit path",
                        Some(i) => &labels[*i],
                    };
                    let response = ui.add_sized(
                        Vec2::new(*item_width, 18.0),
                        egui::Button::new(RichText::new(label).small()).truncate(),
                    );
                    match item {
                        Some(i) if *i == paths.len() => edit |= response.clicked(),
                        Some(i) => {
                            if response
                                .on_hover_text(files::display_path(&paths[*i]))
                                .clicked()
                            {
                                clicked = Some(paths[*i].clone());
                            }
                        }
                        None => {
                            response.on_hover_text(files::display_path(paths.last().unwrap()));
                        }
                    }
                }
            });
        }
    });
    (clicked, edit)
}

pub(super) fn selection_footer(
    ui: &mut egui::Ui,
    index: usize,
    pane: &mut Pane,
    active: bool,
    accent: Color32,
    muted: Color32,
    border: egui::Stroke,
) -> egui::InnerResponse<()> {
    let bounds = ui.available_rect_before_wrap();
    let footer = egui::Panel::bottom(Id::new(("pane-footer", index)))
        .resizable(false)
        .frame(
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(12, 6))
                .stroke(border),
        )
        .show(ui, |ui| {
            pane_footer(ui, pane, active, accent, muted);
        });
    if footer.response.rect.bottom() > bounds.bottom() + 1.0 {
        ui.ctx()
            .request_discard("Selection footer grew after wrapping");
    }
    footer
}

fn pane_footer(ui: &mut egui::Ui, pane: &mut Pane, active: bool, accent: Color32, muted: Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!(
                "{} visible / {} selected",
                pane.visible.len(),
                pane.selected.len()
            ))
            .monospace()
            .small()
            .weak(),
        );
        let hidden = pane
            .selected
            .iter()
            .filter(|path| {
                !pane
                    .visible
                    .iter()
                    .any(|row| &pane.entries[*row].path == *path)
            })
            .count();
        if hidden > 0 {
            ui.label(format!("{hidden} selected hidden by filter"));
        }
        if ui
            .add_enabled(
                !pane.selected.is_empty(),
                egui::Button::new("Clear selection"),
            )
            .clicked()
        {
            pane.selected.clear();
        }
        ui.label(
            RichText::new(if active { "READY / FOCUSED" } else { "READY" })
                .monospace()
                .size(10.0)
                .color(if active { accent } else { muted }),
        );
    });
}

pub(super) fn trailing_field(
    ui: &mut egui::Ui,
    field: egui::TextEdit<'_>,
    controls: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            controls(ui);
            ui.add_sized(
                Vec2::new(ui.available_width().max(1.0), ui.spacing().interact_size.y),
                field,
            )
        })
        .inner
    })
    .inner
}

pub(super) fn format_modified(modified: Option<SystemTime>) -> String {
    let Some(time) = modified else {
        return "-".into();
    };
    let Ok(duration) = time.duration_since(SystemTime::UNIX_EPOCH) else {
        return "pre-1970".into();
    };
    let mut days = duration.as_secs() / 86_400;
    let mut year = 1970;
    loop {
        let leap = year % 400 == 0 || (year % 4 == 0 && year % 100 != 0);
        let length = if leap { 366 } else { 365 };
        if days < length {
            break;
        }
        days -= length;
        year += 1;
        if year > 9999 {
            return ">9999".into();
        }
    }
    let leap = year % 400 == 0 || (year % 4 == 0 && year % 100 != 0);
    let lengths = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0;
    while days >= lengths[month] {
        days -= lengths[month];
        month += 1;
    }
    format!(
        "{year:04}-{:02}-{:02} {:02}:{:02}",
        month + 1,
        days + 1,
        duration.as_secs() % 86_400 / 3600,
        duration.as_secs() % 3600 / 60
    )
}
