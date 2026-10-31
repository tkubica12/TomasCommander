use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Rail {
    Favorites,
    Recent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecentAction {
    Open,
    Preview,
    Locate,
    OpenWith,
    CopyPath,
}

impl Ledger {
    pub(super) fn rail_keys(&mut self, ctx: &egui::Context, clipboard_copy: bool) -> bool {
        let Some(mut index) = self.rail_index else {
            return false;
        };
        if self.rail == Rail::Recent
            && let Some(action) = consume_recent_action(ctx, clipboard_copy)
        {
            if let Some(item) = self.recent_snapshot.items.get(index) {
                self.recent_action(ctx, item.path.clone(), action);
            }
            return true;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            index = index.saturating_sub(1);
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            index += 1;
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        }
        let length = if self.rail == Rail::Recent {
            self.recent_snapshot.items.len()
        } else {
            self.preferences.favorites.len()
        };
        index = index.min(length.saturating_sub(1));
        self.rail_index = Some(index);
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter)) {
            let path = if self.rail == Rail::Recent {
                self.recent_snapshot
                    .items
                    .get(index)
                    .map(|item| item.path.clone())
            } else {
                self.preferences.favorites.get(index).cloned()
            };
            self.rail_index = None;
            self.file_focus_requested = true;
            if let Some(path) = path {
                if self.rail == Rail::Recent {
                    self.recent_action(ctx, path, RecentAction::Open);
                } else {
                    self.navigate(self.active, path);
                }
            }
        }
        if ctx.input_mut(|input| {
            input.consume_key(Modifiers::NONE, Key::Escape)
                || input.consume_key(Modifiers::NONE, Key::Tab)
        }) {
            self.rail_index = None;
            self.file_focus_requested = true;
        }
        true
    }

    fn recent_action(&mut self, ctx: &egui::Context, path: PathBuf, action: RecentAction) {
        if action == RecentAction::CopyPath {
            ctx.copy_text(files::display_path(&path));
            return;
        }
        if !self.file_actions_allowed() {
            self.fail("Finish the current operation or editor before opening a Recent file.");
            return;
        }
        self.rail_index = None;
        ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        let pane = if self.reader.target == Some(self.active) {
            1 - self.active
        } else {
            self.active
        };
        let job = match action {
            RecentAction::Open => Work::Open(path),
            RecentAction::OpenWith => Work::OpenFileWith(path),
            RecentAction::Locate => {
                if self.archive.target == Some(pane) {
                    self.archive.close();
                    if self.reader.is_virtual() {
                        self.reader.close();
                    }
                }
                Work::LocateRecent { pane, path }
            }
            RecentAction::Preview => {
                if self.archive.target == Some(1 - pane) {
                    self.archive.close();
                }
                if let Err(error) = self.reader.start(pane, path) {
                    self.fail(error);
                }
                return;
            }
            RecentAction::CopyPath => unreachable!(),
        };
        self.file_focus_requested = true;
        if let Err(error) = self.work.try_send(job) {
            self.fail(format!("Recent action was not queued: {error}"));
        }
    }

    pub(super) fn context_rail(&mut self, ui: &mut egui::Ui) {
        if self.rail_open {
            egui::Panel::left("context-rail")
                .resizable(true)
                .default_size(175.0)
                .size_range(145.0..=300.0)
                .frame(
                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .inner_margin(egui::Margin::symmetric(12, 16)),
                )
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("<").on_hover_text("Collapse workspace").clicked() {
                            self.rail_open = false;
                            self.rail_index = None;
                            self.file_focus_requested = true;
                            ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                        }
                        ui.label(
                            RichText::new("WORKSPACE / LEDGER")
                                .monospace()
                                .size(10.0)
                                .weak(),
                        );
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().button_padding.x = 4.0;
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (rail, label) in [(Rail::Favorites, "Places"), (Rail::Recent, "Recent")]
                        {
                            if ui
                                .selectable_value(
                                    &mut self.rail,
                                    rail,
                                    RichText::new(label).size(11.0),
                                )
                                .clicked()
                            {
                                self.rail_index = None;
                            }
                        }
                    });
                    ui.separator();
                    match self.rail {
                        Rail::Recent => self.recent_rail(ui),
                        Rail::Favorites => self.favorites_rail(ui),
                    }
                });
        } else {
            egui::Panel::left("context-rail-collapsed")
                .resizable(false)
                .exact_size(32.0)
                .frame(
                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .inner_margin(egui::Margin::symmetric(4, 16)),
                )
                .show(ui, |ui| {
                    if ui.button(">").on_hover_text("Expand workspace").clicked() {
                        self.rail_open = true;
                        self.rail = Rail::Recent;
                        self.rail_index = None;
                    }
                });
        }
    }

    fn recent_rail(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(
            RichText::new("RECENT ACTIVITY")
                .monospace()
                .size(10.0)
                .weak(),
        );
        if self.recent_snapshot.scanning {
            ui.label("Checking recent changes...");
        }
        let allowed = self.drag_allowed();
        let mut chosen = None;
        egui::ScrollArea::vertical()
            .id_salt("recent-activity")
            .max_height((ui.available_height() - 60.0).max(60.0))
            .animated(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (index, item) in self.recent_snapshot.items.iter().enumerate() {
                    let (response, action) =
                        recent_row(ui, item, self.rail_index == Some(index), allowed);
                    if self.rail_index == Some(index) {
                        response.request_focus();
                        response.scroll_to_me(None);
                    }
                    if let Some(action) = action {
                        chosen = Some((item.path.clone(), action));
                    }
                }
                if self.recent_snapshot.items.is_empty() {
                    ui.label("No recent documents yet.");
                }
            });
        if let Some((path, action)) = chosen {
            self.recent_action(ui.ctx(), path, action);
        }
        ui.label(RichText::new("Ctrl Shift H").small().weak());
    }

    fn favorites_rail(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(
            RichText::new("PINNED LOCATIONS")
                .monospace()
                .size(10.0)
                .weak(),
        );
        ui.add_space(6.0);
        let mut target = None;
        let mut remove = None;
        let mut edit = None;
        egui::ScrollArea::vertical()
            .id_salt("pinned-locations")
            .max_height((ui.available_height() - 170.0).max(60.0))
            .animated(false)
            .show(ui, |ui| {
                let paths = &self.preferences.favorites;
                for (index, path) in paths.iter().enumerate() {
                    let response = {
                        let name = self.preferences.favorite_name(path);
                        let (response, rename, delete) =
                            saved_place_row(ui, &name, path, self.rail_index == Some(index));
                        if rename {
                            edit = Some((path.clone(), name));
                        }
                        if delete {
                            remove = Some(path.clone());
                        }
                        response
                    };
                    if self.rail_index == Some(index) {
                        response.request_focus();
                        response.scroll_to_me(None);
                    }
                    if response.clicked() {
                        target = Some(path.clone());
                    }
                }
            });
        if let Some(path) = remove {
            self.preferences.remove_favorite(&path);
            self.rail_index = None;
            self.save_pending = true;
        }
        if edit.is_some() {
            self.name_dialog = None;
            self.inline_pending = None;
            self.place_edit = edit;
        }
        if let Some(target) = target {
            self.rail_index = None;
            self.navigate(self.active, target);
        }

        ui.add_space(25.0);
        if self.rail == Rail::Favorites && ui.button("+ Pin active folder").clicked() {
            self.dispatch(Command::Pin);
        }
    }
}

pub(super) fn toggle_recent_rail(open: &mut bool, rail: &mut Rail, index: &mut Option<usize>) {
    *open = !*open;
    if *open {
        *rail = Rail::Recent;
    }
    *index = (*open).then_some(0);
}

pub(super) fn consume_recent_action(
    ctx: &egui::Context,
    clipboard_copy: bool,
) -> Option<RecentAction> {
    if clipboard_copy {
        return Some(RecentAction::CopyPath);
    }
    for (modifiers, key, action) in [
        (Modifiers::CTRL, Key::Enter, RecentAction::Preview),
        (Modifiers::SHIFT, Key::Enter, RecentAction::OpenWith),
        (
            Modifiers::CTRL | Modifiers::SHIFT,
            Key::C,
            RecentAction::CopyPath,
        ),
        (Modifiers::ALT, Key::Enter, RecentAction::Locate),
    ] {
        if ctx.input_mut(|input| input.consume_key(modifiers, key)) {
            return Some(action);
        }
    }
    None
}

pub(super) fn remap_recent_cursor(
    old: &[recent::Item],
    new: &[recent::Item],
    index: Option<usize>,
) -> Option<usize> {
    if old.is_empty() && index.is_some() {
        return (!new.is_empty()).then_some(0);
    }
    let path = &old.get(index?)?.path;
    new.iter().position(|item| &item.path == path)
}

pub(super) fn recent_row(
    ui: &mut egui::Ui,
    item: &recent::Item,
    selected: bool,
    enabled: bool,
) -> (egui::Response, Option<RecentAction>) {
    let name = item
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| files::display_path(&item.path));
    let response = ui.push_id(&item.path, |ui| {
        ui.add(egui::Button::new(RichText::new(name).font(FILE_NAME_FONT)).min_size(Vec2::new(ui.available_width(), ROW_HEIGHT))
            .frame(false).truncate().selected(selected)
            .sense(if enabled { egui::Sense::click_and_drag() } else { egui::Sense::click() }))
    }).inner.on_hover_text(format!("{}\n{} / {}\nClick: open containing folder / Double-click: open file\nEnter: open / Ctrl Enter: preview / Shift Enter: Open with\nAlt Enter: open containing folder / Ctrl Shift C: copy path",
        files::display_path(&item.path), item.activity.label(), recent_age(item.at_ms)));
    let mut action = if response.double_clicked_by(egui::PointerButton::Primary) {
        Some(RecentAction::Open)
    } else {
        response.clicked().then_some(RecentAction::Locate)
    };
    response.context_menu(|ui| {
        for (label, candidate) in [
            ("Open", RecentAction::Open),
            ("Preview", RecentAction::Preview),
            ("Open containing folder", RecentAction::Locate),
            ("Open with...", RecentAction::OpenWith),
            ("Copy path", RecentAction::CopyPath),
        ] {
            if ui
                .add_enabled(
                    enabled || candidate == RecentAction::CopyPath,
                    egui::Button::new(label),
                )
                .clicked()
            {
                action = Some(candidate);
                ui.close();
            }
        }
    });
    if enabled && response.drag_started() {
        start_file_drag(ui.ctx(), vec![item.path.clone()]);
        action = None;
    }
    (response, action)
}

fn recent_age(at_ms: u64) -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let minutes = now.saturating_sub(u128::from(at_ms)) / 60_000;
    match minutes {
        0 => "just now".into(),
        1..60 => format!("{minutes} min ago"),
        60..1440 => format!("{} h ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    }
}

pub(super) fn saved_place_row(
    ui: &mut egui::Ui,
    name: &str,
    path: &std::path::Path,
    selected: bool,
) -> (egui::Response, bool, bool) {
    let mut edit = false;
    let mut delete = false;
    if ui.available_width() < 240.0 {
        let response = ui
            .add_sized(
                Vec2::new(ui.available_width(), 24.0),
                egui::Button::new(name).selected(selected).truncate(),
            )
            .on_hover_text(files::display_path(path));
        ui.horizontal(|ui| {
            edit = ui.small_button("Edit").clicked();
            delete = ui
                .small_button("Delete")
                .on_hover_text("Remove bookmark only; keep folder")
                .clicked();
        });
        return (response, edit, delete);
    }
    let response = ui
        .horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                delete = ui
                    .small_button("Delete")
                    .on_hover_text("Remove bookmark only; keep folder")
                    .clicked();
                edit = ui.small_button("Edit").clicked();
                ui.add_sized(
                    Vec2::new(ui.available_width(), 24.0),
                    egui::Button::new(name).selected(selected).truncate(),
                )
                .on_hover_text(files::display_path(path))
            })
            .inner
        })
        .inner;
    (response, edit, delete)
}
