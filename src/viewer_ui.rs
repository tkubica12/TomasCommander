use eframe::egui::{self, RichText, Vec2};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
};
use tomas_commander::{
    files::{self, FileResult, Scope},
    viewer::{self, Content, Preview},
};

struct Request {
    generation: u64,
    path: PathBuf,
    page: u32,
    quarter_turns: u8,
    edit: bool,
    virtual_entry: Option<String>,
}

pub struct ReaderPanel {
    jobs: SyncSender<Request>,
    results: Receiver<(u64, FileResult<Preview>)>,
    generation: Arc<AtomicU64>,
    pub target: Option<usize>,
    pub source: usize,
    path: PathBuf,
    preview: Option<Preview>,
    error: Option<String>,
    texture: Option<egui::TextureHandle>,
    zoom: f32,
    offset: f32,
    prefix: String,
    filter: String,
    archive_cursor: usize,
    archive_notice: Option<String>,
    markdown_cache: egui_commonmark::CommonMarkCache,
    inspected_link: Option<String>,
    quarter_turns: u8,
    pub edit_requested: bool,
    pub save_requested: bool,
    pub recent_activity: Option<PathBuf>,
    close_prompt: bool,
    close_prompt_focus: bool,
    virtual_entry: Option<String>,
    pub open_entry: Option<String>,
    selected_archive: std::collections::BTreeSet<String>,
}

impl ReaderPanel {
    pub fn new(scope: Scope, context: egui::Context) -> Self {
        let (jobs, requests) = mpsc::sync_channel::<Request>(1);
        let (sender, results) = mpsc::sync_channel(1);
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        std::thread::spawn(move || {
            while let Ok(request) = requests.recv() {
                let cancelled = || current.load(Ordering::Acquire) != request.generation;
                if cancelled() {
                    continue;
                }
                let result = if let Some(entry) = &request.virtual_entry {
                    viewer::load_archive_entry(
                        &scope,
                        &request.path,
                        entry,
                        request.page,
                        request.quarter_turns,
                        cancelled,
                    )
                } else if request.edit {
                    tomas_commander::changes::Editor::open(&scope, &request.path).map(|editor| Preview {
                        path: request.path.clone(),
                        content: Content::Editable(editor),
                        notice: "Local UTF-8 source editing / no upload / Save requires exact-target approval",
                    })
                } else {
                    viewer::load_transformed(
                        &scope,
                        &request.path,
                        request.page,
                        request.quarter_turns,
                        cancelled,
                    )
                };
                if cancelled() {
                    continue;
                }
                context.request_repaint();
                if sender.send((request.generation, result)).is_err() {
                    break;
                }
                context.request_repaint();
            }
        });
        Self {
            jobs,
            results,
            generation,
            target: None,
            source: 0,
            path: PathBuf::new(),
            preview: None,
            error: None,
            texture: None,
            zoom: 1.0,
            offset: 0.0,
            prefix: String::new(),
            filter: String::new(),
            archive_cursor: 0,
            archive_notice: None,
            markdown_cache: Default::default(),
            inspected_link: None,
            quarter_turns: 0,
            edit_requested: false,
            save_requested: false,
            recent_activity: None,
            close_prompt: false,
            close_prompt_focus: false,
            virtual_entry: None,
            open_entry: None,
            selected_archive: Default::default(),
        }
    }

    pub fn start(&mut self, source: usize, path: PathBuf) -> FileResult<()> {
        if self.dirty() {
            return Err(
                "Save or explicitly discard current edits before loading another document.".into(),
            );
        }
        self.virtual_entry = None;
        self.start_page(source, path, 0, 0, false)
    }

    pub fn toggle(&mut self, source: usize, path: PathBuf) -> FileResult<()> {
        if self.target.is_some()
            && self.source == source
            && self.path == path
            && self.virtual_entry.is_none()
        {
            self.close();
            Ok(())
        } else {
            self.start(source, path)
        }
    }

    fn start_page(
        &mut self,
        source: usize,
        path: PathBuf,
        page: u32,
        quarter_turns: u8,
        edit: bool,
    ) -> FileResult<()> {
        if self.dirty() {
            return Err(
                "Save or explicitly discard the current edits before loading another document."
                    .into(),
            );
        }
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Err(error) = self.jobs.try_send(Request {
            generation,
            path: path.clone(),
            page,
            quarter_turns,
            edit,
            virtual_entry: self.virtual_entry.clone(),
        }) {
            self.close();
            return Err(format!(
                "Could not queue preview; previous read cancelled: {error}"
            ));
        }
        self.source = source;
        self.target = Some(1 - source);
        self.path = path;
        self.preview = None;
        self.error = None;
        self.texture = None;
        self.zoom = 1.0;
        self.offset = 0.0;
        self.prefix.clear();
        self.filter.clear();
        self.archive_cursor = 0;
        self.archive_notice = None;
        self.markdown_cache = Default::default();
        self.inspected_link = None;
        self.quarter_turns = quarter_turns;
        self.close_prompt = false;
        self.selected_archive.clear();
        self.open_entry = None;
        Ok(())
    }

    pub fn step_pdf(&mut self, delta: i32) -> FileResult<()> {
        let Some(Preview {
            content: Content::Pdf { page, count, .. },
            ..
        }) = &self.preview
        else {
            return Err("No loaded PDF page is available.".into());
        };
        let next = page.saturating_add_signed(delta).min(count - 1);
        if next == *page {
            return Ok(());
        }
        self.start_page(
            self.source,
            self.path.clone(),
            next,
            self.quarter_turns,
            false,
        )
    }

    pub fn rotate(&mut self, clockwise: bool) -> FileResult<()> {
        let page = match self.preview.as_ref().map(|preview| &preview.content) {
            Some(Content::Image { .. }) => 0,
            Some(Content::Pdf { page, .. }) => *page,
            _ => return Err("Load an image or PDF page before rotating.".into()),
        };
        let turns = (self.quarter_turns + if clockwise { 1 } else { 3 }) % 4;
        self.start_page(self.source, self.path.clone(), page, turns, false)
    }

    pub fn start_edit(&mut self, source: usize, path: PathBuf) -> FileResult<()> {
        if self.virtual_entry.is_some() {
            return Err("Archive entry editing requires explicit extraction first.".into());
        }
        self.start_page(source, path, 0, 0, true)
    }

    pub fn start_virtual(&mut self, source: usize, path: PathBuf, name: String) -> FileResult<()> {
        if self.dirty() {
            return Err("Save or discard current edits before opening an archive entry.".into());
        }
        self.virtual_entry = Some(name);
        self.start_page(source, path, 0, 0, false)
    }

    pub fn archive_location(&self) -> (PathBuf, String) {
        (self.path.clone(), self.prefix.clone())
    }
    pub fn request_archive_preview(&mut self) {
        self.archive_enter();
    }
    pub fn is_virtual(&self) -> bool {
        self.virtual_entry.is_some()
    }

    pub fn archive_targets(&self) -> Vec<String> {
        if !self.selected_archive.is_empty() {
            return self.selected_archive.iter().cloned().collect();
        }
        self.archive_rows()
            .get(self.archive_cursor.saturating_sub(1))
            .filter(|_| self.archive_cursor != 0)
            .map(|entry| vec![entry.path.clone()])
            .unwrap_or_default()
    }

    pub fn refresh_archive(&mut self) -> FileResult<()> {
        let (path, _) = self.archive_location();
        self.open_archive(self.source, path)
    }

    pub fn begin_edit(&mut self) -> FileResult<()> {
        if self.target.is_none() {
            return Err("Open a text reading pane before editing.".into());
        }
        self.start_edit(self.source, self.path.clone())
    }

    pub fn dirty(&self) -> bool {
        matches!(&self.preview, Some(Preview { content: Content::Editable(editor), .. }) if editor.dirty())
    }

    pub fn save_plan(&self, id: u64) -> FileResult<tomas_commander::changes::Plan> {
        match &self.preview {
            Some(Preview {
                content: Content::Editable(editor),
                ..
            }) => editor.plan(id),
            _ => Err("Open edit mode before requesting Save.".into()),
        }
    }

    pub fn saved(&mut self) -> FileResult<()> {
        self.preview = None;
        self.start_edit(self.source, self.path.clone())
    }

    pub fn open_archive(&mut self, source: usize, path: PathBuf) -> FileResult<()> {
        self.start(source, path)?;
        self.target = Some(source);
        Ok(())
    }

    pub fn is_archive(&self) -> bool {
        self.target.is_some()
            && self.virtual_entry.is_none()
            && self
                .path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    }

    fn archive_rows(&self) -> Vec<&tomas_commander::archives::ArchiveEntry> {
        let Some(Preview {
            content: Content::Archive(inventory),
            ..
        }) = &self.preview
        else {
            return Vec::new();
        };
        let query = tomas_commander::search::fold(&self.filter);
        let mut rows: Vec<_> = inventory
            .entries
            .iter()
            .filter(|entry| {
                let Some(relative) = entry.path.strip_prefix(&self.prefix) else {
                    return false;
                };
                if query.is_empty() {
                    !relative.contains('/') && !relative.is_empty()
                } else {
                    tomas_commander::search::fold(relative).contains(&query)
                }
            })
            .collect();
        rows.sort_by(|left, right| {
            right
                .directory
                .cmp(&left.directory)
                .then(left.path.cmp(&right.path))
        });
        rows
    }

    fn archive_parent(&mut self) -> bool {
        if self.prefix.is_empty() {
            return true;
        }
        let path = self.prefix.trim_end_matches('/');
        self.prefix = path
            .rsplit_once('/')
            .map_or_else(String::new, |(parent, _)| format!("{parent}/"));
        self.filter.clear();
        self.selected_archive.clear();
        self.archive_cursor = 0;
        self.offset = 0.0;
        false
    }

    fn archive_enter(&mut self) -> bool {
        if self.archive_cursor == 0 {
            return self.archive_parent();
        }
        let choice = self
            .archive_rows()
            .get(self.archive_cursor - 1)
            .map(|entry| (entry.path.clone(), entry.directory));
        if let Some((path, directory)) = choice {
            if directory {
                self.prefix = format!("{path}/");
                self.filter.clear();
                self.selected_archive.clear();
                self.archive_cursor = 0;
                self.offset = 0.0;
                self.archive_notice = None;
            } else {
                self.open_entry = Some(path);
            }
        }
        false
    }

    pub fn archive_keys(&mut self, context: &egui::Context) -> bool {
        if context.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::A)) {
            self.selected_archive = self
                .archive_rows()
                .iter()
                .map(|entry| entry.path.clone())
                .collect();
        }
        for (key, down) in [(egui::Key::ArrowUp, false), (egui::Key::ArrowDown, true)] {
            if context.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, key)) {
                self.archive_cursor = if down {
                    (self.archive_cursor + 10).min(self.archive_rows().len())
                } else {
                    self.archive_cursor.saturating_sub(10)
                };
                self.offset = self.archive_cursor as f32 * 26.0;
            }
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::ArrowLeft))
        {
            return self.archive_parent();
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Space))
            && let Some(entry) = self
                .archive_rows()
                .get(self.archive_cursor.saturating_sub(1))
                .filter(|_| self.archive_cursor != 0)
        {
            let path = entry.path.clone();
            if !self.selected_archive.remove(&path) {
                self.selected_archive.insert(path);
            }
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)) {
            return self.archive_enter();
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown))
        {
            self.archive_cursor = (self.archive_cursor + 1).min(self.archive_rows().len());
            self.offset = self.archive_cursor as f32 * 26.0;
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
            self.archive_cursor = self.archive_cursor.saturating_sub(1);
            self.offset = self.archive_cursor as f32 * 26.0;
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
        {
            return self.archive_parent();
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
            return self.archive_enter();
        }
        false
    }

    pub fn close(&mut self) {
        if self.dirty() {
            if !self.close_prompt {
                self.close_prompt_focus = true;
            }
            self.close_prompt = true;
            return;
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.target = None;
        self.preview = None;
        self.texture = None;
        self.error = None;
        self.path.clear();
        self.markdown_cache = Default::default();
        self.inspected_link = None;
        self.close_prompt = false;
        self.virtual_entry = None;
        self.selected_archive.clear();
        self.open_entry = None;
    }

    pub fn scroll(&mut self, delta: f32) {
        self.offset = (self.offset + delta).max(0.0);
    }

    pub fn poll(&mut self) {
        loop {
            let (generation, result) = match self.results.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.target.is_some() && self.preview.is_none() && self.error.is_none() {
                        self.error = Some(
                            "Reader worker stopped. Close preview and restart the app.".into(),
                        );
                    }
                    break;
                }
            };
            if generation != self.generation.load(Ordering::Acquire) || self.target.is_none() {
                continue;
            }
            match result {
                Ok(preview) => {
                    if self.virtual_entry.is_none() {
                        self.recent_activity = Some(preview.path.clone());
                    }
                    if let Content::Markdown { links, .. } = &preview.content {
                        for link in links {
                            self.markdown_cache.add_link_hook(link);
                        }
                    }
                    self.preview = Some(preview);
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> bool {
        let mut close = false;
        let mut page_delta = 0;
        let mut rotate = false;
        ui.horizontal(|ui| {
            ui.heading(if self.is_archive() {
                "ZIP / virtual folder"
            } else {
                "Reading pane"
            });
            close = ui.button("Close").on_hover_text("Escape").clicked();
        });
        ui.label(files::display_path(&self.path));
        if let Some(entry) = &self.virtual_entry {
            ui.label(format!("ZIP entry: {entry}"));
        }
        ui.label("Original file-pane state retained / Tab switches focus");
        if self.close_prompt {
            let mut discard = false;
            let mut keep = false;
            let mut save = false;
            let modal =
                egui::Modal::new(egui::Id::new("reader-dirty-close")).show(ui.ctx(), |ui| {
                    ui.set_width(460.0);
                    ui.heading("Unsaved edits");
                    ui.label(files::display_path(&self.path));
                    ui.label("Save changes before closing?");
                    ui.horizontal(|ui| {
                        let button = ui.button("Discard");
                        discard = button.clicked()
                            || (button.has_focus()
                                && crate::app::consume_prompt_key(
                                    ui.ctx(),
                                    egui::Modifiers::NONE,
                                    egui::Key::Enter,
                                ));
                        (save, keep) = crate::app::confirmation_buttons(
                            ui,
                            "Save",
                            &mut self.close_prompt_focus,
                        );
                    });
                });
            if keep || modal.should_close() {
                self.close_prompt = false;
            } else if discard {
                self.preview = None;
                self.close();
                return true;
            } else if save {
                self.close_prompt = false;
                self.save_requested = true;
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return close;
        }
        if let Some(Preview {
            content: Content::Editable(editor),
            notice,
            ..
        }) = &mut self.preview
        {
            ui.label(*notice);
            ui.label(if editor.dirty() {
                "UNSAVED CHANGES"
            } else {
                "Original source / unchanged"
            });
            self.save_requested |= ui.button("Save").on_hover_text("Ctrl S").clicked();
            let id = egui::Id::new("reader-editor");
            if ui.is_enabled() {
                crate::app::accessible_text_value(ui.ctx(), id, &mut editor.text);
            }
            egui::ScrollArea::both()
                .id_salt("reader-source-editor")
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut editor.text)
                            .id(id)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .char_limit(tomas_commander::changes::TEXT_LIMIT),
                    );
                });
            return close;
        }
        if self.preview.is_some() && tomas_commander::changes::editable(&self.path) {
            self.edit_requested |= ui.button("Edit").on_hover_text("Ctrl E").clicked();
        }
        if self.is_archive() && self.preview.is_some() {
            ui.label(format!(
                "{} selected / Space toggles / Ctrl A selects visible / Enter previews files",
                self.selected_archive.len()
            ));
            ui.label(format!("Virtual folder: /{}", self.prefix));
            ui.label("Filter / recursive filename search:");
            let id = egui::Id::new("archive-filter");
            let accessible_changed =
                crate::app::accessible_text_value(ui.ctx(), id, &mut self.filter);
            let edited = ui
                .add(egui::TextEdit::singleline(&mut self.filter).id(id))
                .changed();
            if accessible_changed || edited {
                self.archive_cursor = 0;
                self.offset = 0.0;
            }
        }
        let Some(preview) = &self.preview else {
            ui.spinner();
            ui.label("Reading locally...");
            return close;
        };
        ui.label(preview.notice);
        if let Content::Pdf { page, count, .. } = &preview.content {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Page {} / {count} / Ctrl PageUp/PageDown",
                    page + 1
                ));
                if ui
                    .add_enabled(*page > 0, egui::Button::new("Previous page"))
                    .clicked()
                {
                    page_delta = -1;
                }
                if ui
                    .add_enabled(page + 1 < *count, egui::Button::new("Next page"))
                    .clicked()
                {
                    page_delta = 1;
                }
            });
        }
        match &preview.content {
            Content::Editable(_) => unreachable!("editor handled above"),
            Content::Markdown { text, links } => {
                let area = egui::ScrollArea::both()
                    .id_salt("reader-markdown")
                    .vertical_scroll_offset(self.offset)
                    .show(ui, |ui| {
                        egui_commonmark::CommonMarkViewer::new()
                            .explicit_image_uri_scheme(true)
                            .show(ui, &mut self.markdown_cache, text);
                    });
                self.offset = area.state.offset.y;
                for link in links {
                    if self.markdown_cache.get_link_hook(link) == Some(true) {
                        self.inspected_link = Some(link.clone());
                    }
                }
                if let Some(link) = &self.inspected_link {
                    ui.label(format!("Inspected link (not opened): {link}"));
                }
            }
            Content::Archive(_) => {
                let rows: Vec<_> = self.archive_rows().into_iter().cloned().collect();
                let mut chosen = None;
                let area = egui::ScrollArea::both()
                    .id_salt("archive-rows")
                    .vertical_scroll_offset(self.offset)
                    .show_rows(ui, 26.0, rows.len() + 1, |ui, range| {
                        for index in range {
                            let label = if index == 0 {
                                if self.prefix.is_empty() {
                                    ".. / exit ZIP".to_owned()
                                } else {
                                    ".. / parent".to_owned()
                                }
                            } else {
                                let entry = &rows[index - 1];
                                format!(
                                    "{} {}{}",
                                    if entry.directory { "[DIR]" } else { "[ZIP]" },
                                    entry.path.strip_prefix(&self.prefix).unwrap_or(&entry.path),
                                    if entry.directory {
                                        String::new()
                                    } else {
                                        format!(" / {} B", entry.size)
                                    }
                                )
                            };
                            let selected =
                                index != 0 && self.selected_archive.contains(&rows[index - 1].path);
                            let response = ui.add(
                                egui::Button::selectable(
                                    self.archive_cursor == index || selected,
                                    label,
                                )
                                .sense(if index == 0 {
                                    egui::Sense::click()
                                } else {
                                    egui::Sense::click_and_drag()
                                }),
                            );
                            if index != 0 && response.drag_started_by(egui::PointerButton::Primary)
                            {
                                let path = rows[index - 1].path.clone();
                                let selected = if self.selected_archive.contains(&path) {
                                    self.selected_archive.iter().cloned().collect()
                                } else {
                                    vec![path]
                                };
                                egui::DragAndDrop::set_payload(
                                    ui.ctx(),
                                    crate::app::DragItems::Zip {
                                        archive: self.path.clone(),
                                        prefix: self.prefix.clone(),
                                        selected,
                                    },
                                );
                            }
                            if index != 0 && rows[index - 1].directory {
                                crate::app::register_drop_zone(
                                    ui.ctx(),
                                    response.rect.intersect(ui.clip_rect()),
                                    crate::app::DropLocation::Zip {
                                        archive: self.path.clone(),
                                        prefix: rows[index - 1].path.clone(),
                                    },
                                    2,
                                );
                            }
                            if response.clicked() {
                                chosen = Some(index);
                            }
                        }
                    });
                self.offset = area.state.offset.y;
                if let Some(index) = chosen {
                    self.archive_cursor = index;
                    close |= self.archive_enter();
                }
                if let Some(notice) = &self.archive_notice {
                    ui.colored_label(ui.visuals().error_fg_color, notice);
                }
            }
            Content::Text(lines) => {
                let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 4.0;
                let area = egui::ScrollArea::both()
                    .id_salt("reader-text")
                    .vertical_scroll_offset(self.offset)
                    .show_rows(ui, row_height, lines.len(), |ui, range| {
                        for index in range {
                            ui.label(
                                RichText::new(if lines[index].is_empty() {
                                    " "
                                } else {
                                    &lines[index]
                                })
                                .monospace(),
                            );
                        }
                    });
                self.offset = area.state.offset.y;
            }
            Content::Table { rows, columns } => {
                ui.label(format!("{} rows / {columns} columns", rows.len()));
                let height = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
                let area = egui::ScrollArea::both()
                    .id_salt("reader-table")
                    .vertical_scroll_offset(self.offset)
                    .show_rows(ui, height, rows.len(), |ui, range| {
                        for index in range {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{}", index + 1)).weak());
                                for column in 0..*columns {
                                    let cell = rows[index].get(column).map_or("", String::as_str);
                                    ui.add_sized(
                                        [150.0, height],
                                        egui::Label::new(cell.replace(['\r', '\n'], " "))
                                            .truncate(),
                                    )
                                    .on_hover_text(cell);
                                }
                            });
                        }
                    });
                self.offset = area.state.offset.y;
            }
            Content::Image {
                width,
                height,
                rgba,
            }
            | Content::Pdf {
                width,
                height,
                rgba,
                ..
            } => {
                ui.horizontal(|ui| {
                    ui.label(format!("{width} x {height}"));
                    if ui.button("-").clicked() {
                        self.zoom = (self.zoom / 1.25).max(0.1);
                    }
                    if ui.button("+").clicked() {
                        self.zoom = (self.zoom * 1.25).min(4.0);
                    }
                    if ui.button("Fit").clicked() {
                        self.zoom = 1.0;
                    }
                    rotate = ui.button("Rotate / Ctrl Right").clicked();
                });
                if self.texture.is_none() {
                    self.texture = Some(ui.ctx().load_texture(
                        "reading-pane-image",
                        egui::ColorImage::from_rgba_unmultiplied(
                            [*width as usize, *height as usize],
                            rgba,
                        ),
                        egui::TextureOptions::LINEAR,
                    ));
                }
                if let Some(texture) = &self.texture {
                    let fit = (ui.available_width() / *width as f32)
                        .min(ui.available_height() / *height as f32)
                        .min(1.0);
                    let size = Vec2::new(*width as f32, *height as f32) * fit * self.zoom;
                    let area = egui::ScrollArea::both()
                        .id_salt("reader-image")
                        .vertical_scroll_offset(self.offset)
                        .show(ui, |ui| {
                            ui.image((texture.id(), size));
                        });
                    self.offset = area.state.offset.y;
                }
            }
        }
        if page_delta != 0
            && let Err(error) = self.step_pdf(page_delta)
        {
            self.error = Some(error);
        }
        if rotate && let Err(error) = self.rotate(true) {
            self.error = Some(error);
        }
        close
    }
}

impl Drop for ReaderPanel {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_preview_activity_requires_a_successful_current_physical_result() {
        let mut panel = ReaderPanel::new(Scope::unrestricted(), egui::Context::default());
        let (sender, results) = mpsc::sync_channel(4);
        panel.results = results;
        panel.generation.store(3, Ordering::Release);
        panel.target = Some(1);
        let path = PathBuf::from(r"D:\synthetic\preview.md");
        let preview = || Preview {
            path: path.clone(),
            content: Content::Text(vec!["synthetic".into()]),
            notice: "TEST ONLY",
        };
        sender.send((2, Ok(preview()))).unwrap();
        panel.poll();
        assert!(panel.recent_activity.is_none());
        sender
            .send((3, Err("Synthetic failed preview".into())))
            .unwrap();
        panel.poll();
        assert!(panel.recent_activity.is_none());
        sender.send((3, Ok(preview()))).unwrap();
        panel.poll();
        assert_eq!(panel.recent_activity.take(), Some(path.clone()));
        panel.virtual_entry = Some("member.md".into());
        sender.send((3, Ok(preview()))).unwrap();
        panel.poll();
        assert!(panel.recent_activity.is_none());
        panel.target = None;
        sender.send((3, Ok(preview()))).unwrap();
        panel.poll();
        assert!(panel.recent_activity.is_none());
    }

    #[test]
    fn preview_toggle_closes_same_document_and_replaces_different_document() {
        let mut panel = ReaderPanel::new(Scope::unrestricted(), egui::Context::default());
        let first = PathBuf::from(r"C:\synthetic-preview\first.txt");
        let second = PathBuf::from(r"C:\synthetic-preview\second.txt");
        panel.path = first.clone();
        panel.source = 0;
        panel.target = Some(1);
        let generation = panel.generation.load(Ordering::Acquire);
        panel.toggle(0, first).unwrap();
        assert_eq!(panel.target, None);
        assert!(panel.generation.load(Ordering::Acquire) > generation);
        panel.toggle(0, second.clone()).unwrap();
        assert_eq!(panel.target, Some(1));
        assert_eq!(panel.source, 0);
        assert_eq!(panel.path, second.clone());
        panel.toggle(0, second).unwrap();
        assert_eq!(panel.target, None);
    }

    #[test]
    fn dirty_close_and_superseding_requests_preserve_exact_buffer_and_origin() {
        let root = std::env::temp_dir().join(format!(
            "tc-reader-edit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("sample.md");
        std::fs::write(&path, b"# Original\n").unwrap();
        let scope = Scope::fixture(&root).unwrap();
        let mut editor = tomas_commander::changes::Editor::open(&scope, &path).unwrap();
        editor.text.push_str("owned unsaved edit\n");
        let mut panel = ReaderPanel::new(scope, egui::Context::default());
        panel.path = path.clone();
        panel.target = Some(1);
        panel.preview = Some(Preview {
            path: path.clone(),
            content: Content::Editable(editor),
            notice: "TEST ONLY",
        });
        panel.close();
        assert!(panel.close_prompt && panel.dirty());
        assert_eq!(panel.target, Some(1));
        assert!(panel.start(1, root.join("other.txt")).is_err());
        assert!(
            panel
                .start_virtual(1, root.join("other.zip"), "member.md".into())
                .is_err()
        );
        assert!(panel.begin_edit().is_err());
        assert_eq!(panel.path, path);
        assert_eq!(panel.source, 0);
        assert!(panel.virtual_entry.is_none());
        panel.toggle(0, path.clone()).unwrap();
        assert!(panel.close_prompt && panel.dirty());
        assert_eq!(panel.target, Some(1));
        assert!(panel.toggle(0, root.join("other.txt")).is_err());
        let context = egui::Context::default();
        context
            .run_ui(egui::RawInput::default(), |ui| {
                assert!(!panel.show(ui));
            })
            .textures_delta
            .clear();
        for key in [egui::Key::Escape, egui::Key::Enter] {
            if key == egui::Key::Enter {
                panel.close();
                context
                    .run_ui(egui::RawInput::default(), |ui| {
                        assert!(!panel.show(ui));
                    })
                    .textures_delta
                    .clear();
            }
            context
                .run_ui(
                    egui::RawInput {
                        events: vec![egui::Event::Key {
                            key,
                            physical_key: Some(key),
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }],
                        ..Default::default()
                    },
                    |ui| {
                        assert!(!panel.show(ui));
                    },
                )
                .textures_delta
                .clear();
            assert!(!panel.close_prompt);
            assert!(panel.dirty());
            assert_eq!(panel.save_requested, key == egui::Key::Enter);
            assert_eq!(panel.target, Some(1));
        }
        assert!(panel.dirty());
        assert!(panel.save_plan(1).is_ok());
        assert_eq!(std::fs::read(&path).unwrap(), b"# Original\n");
        panel.preview = None;
        panel.close();
        assert!(panel.target.is_none());
        drop(panel);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn virtual_archive_navigation_filter_and_parent_never_become_filesystem_paths() {
        let (jobs, _requests) = mpsc::sync_channel(1);
        let (_sender, results) = mpsc::sync_channel(1);
        let mut panel = ReaderPanel {
            jobs,
            results,
            generation: Arc::new(AtomicU64::new(1)),
            target: Some(0),
            source: 0,
            path: PathBuf::from("owned.zip"),
            preview: Some(Preview {
                path: PathBuf::from("owned.zip"),
                content: Content::Archive(tomas_commander::archives::Inventory {
                    entries: vec![
                        tomas_commander::archives::ArchiveEntry {
                            path: "Příliš".into(),
                            directory: true,
                            size: 0,
                        },
                        tomas_commander::archives::ArchiveEntry {
                            path: "Příliš/file.txt".into(),
                            directory: false,
                            size: 7,
                        },
                        tomas_commander::archives::ArchiveEntry {
                            path: "root.txt".into(),
                            directory: false,
                            size: 7,
                        },
                    ],
                }),
                notice: "TEST ONLY",
            }),
            error: None,
            texture: None,
            zoom: 1.0,
            offset: 0.0,
            prefix: String::new(),
            filter: String::new(),
            archive_cursor: 1,
            archive_notice: None,
            markdown_cache: Default::default(),
            inspected_link: None,
            quarter_turns: 0,
            edit_requested: false,
            save_requested: false,
            close_prompt: false,
            close_prompt_focus: false,
            recent_activity: None,
            virtual_entry: None,
            open_entry: None,
            selected_archive: Default::default(),
        };
        assert_eq!(panel.archive_rows().len(), 2);
        assert!(!panel.archive_enter());
        assert_eq!(panel.prefix, "Příliš/");
        assert_eq!(panel.archive_rows().len(), 1);
        panel.archive_cursor = 1;
        assert!(!panel.archive_enter());
        assert_eq!(panel.open_entry.as_deref(), Some("Příliš/file.txt"));
        assert_eq!(panel.path, PathBuf::from("owned.zip"));
        assert!(!panel.archive_parent());
        for query in ["prilis", "PŘÍLIŠ", "Pr\u{030c}i\u{0301}lis\u{030c}"] {
            panel.filter = query.into();
            assert_eq!(panel.archive_rows().len(), 2);
        }
        panel.filter = "FILE".into();
        assert_eq!(panel.archive_rows().len(), 1);
        let context = egui::Context::default();
        context
            .run_ui(egui::RawInput::default(), |ui| {
                assert!(!panel.show(ui));
            })
            .textures_delta
            .clear();
        let input = egui::RawInput {
            events: vec![egui::Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::SetValue,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    target_node: egui::Id::new("archive-filter").accesskit_id(),
                    data: Some(egui::accesskit::ActionData::Value("root.txt".into())),
                },
            )],
            ..Default::default()
        };
        context
            .run_ui(input, |ui| {
                assert!(!panel.show(ui));
            })
            .textures_delta
            .clear();
        assert_eq!(panel.filter, "root.txt");
        assert_eq!(panel.archive_rows().len(), 1);
        panel.filter.clear();
        assert!(panel.archive_parent());
    }

    #[test]
    fn closed_or_superseded_preview_discards_late_result_and_surfaces_worker_loss() {
        let (jobs, _requests) = mpsc::sync_channel(1);
        let (sender, results) = mpsc::sync_channel(2);
        let mut panel = ReaderPanel {
            jobs,
            results,
            generation: Arc::new(AtomicU64::new(2)),
            target: Some(1),
            source: 0,
            path: PathBuf::from("synthetic.txt"),
            preview: None,
            error: None,
            texture: None,
            zoom: 1.0,
            offset: 0.0,
            prefix: String::new(),
            filter: String::new(),
            archive_cursor: 0,
            archive_notice: None,
            markdown_cache: Default::default(),
            inspected_link: None,
            quarter_turns: 0,
            edit_requested: false,
            save_requested: false,
            close_prompt: false,
            close_prompt_focus: false,
            recent_activity: None,
            virtual_entry: None,
            open_entry: None,
            selected_archive: Default::default(),
        };
        sender.send((1, Err("stale failure".into()))).unwrap();
        panel.poll();
        assert!(panel.error.is_none());
        sender.send((2, Err("current failure".into()))).unwrap();
        panel.poll();
        assert_eq!(panel.error.as_deref(), Some("current failure"));
        panel.close();
        sender.send((3, Err("closed failure".into()))).unwrap();
        panel.poll();
        assert!(panel.target.is_none() && panel.error.is_none());
        panel.target = Some(1);
        drop(sender);
        panel.poll();
        assert!(panel.error.as_deref().unwrap().contains("worker stopped"));
    }

    #[test]
    fn native_widget_renders_text_table_and_image_without_touching_source_pane() {
        let (jobs, _requests) = mpsc::sync_channel(1);
        let (_sender, results) = mpsc::sync_channel(1);
        let context = egui::Context::default();
        let mut panel = ReaderPanel {
            jobs,
            results,
            generation: Arc::new(AtomicU64::new(1)),
            target: Some(1),
            source: 0,
            path: PathBuf::from("synthetic.txt"),
            preview: None,
            error: None,
            texture: None,
            zoom: 1.0,
            offset: 0.0,
            prefix: String::new(),
            filter: String::new(),
            archive_cursor: 0,
            archive_notice: None,
            markdown_cache: Default::default(),
            inspected_link: None,
            quarter_turns: 0,
            edit_requested: false,
            save_requested: false,
            close_prompt: false,
            close_prompt_focus: false,
            recent_activity: None,
            virtual_entry: None,
            open_entry: None,
            selected_archive: Default::default(),
        };
        for content in [
            Content::Pdf {
                page: 0,
                count: 2,
                width: 2,
                height: 3,
                rgba: vec![255; 24],
            },
            Content::Markdown {
                text: "# Heading\n\n- **item**\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nlet value = 1;\n```\n\n[unsafe](javascript:alert)".into(),
                links: vec!["javascript:alert".into()],
            },
            Content::Text(vec!["Příliš".into(), String::new()]),
            Content::Table {
                rows: vec![vec!["a\nb".into(), "c".into()]],
                columns: 2,
            },
            Content::Image {
                width: 2,
                height: 3,
                rgba: vec![255; 24],
            },
        ] {
            if let Content::Markdown { links, .. } = &content {
                for link in links {
                    panel.markdown_cache.add_link_hook(link);
                }
            }
            panel.preview = Some(Preview {
                path: panel.path.clone(),
                content,
                notice: "TEST ONLY",
            });
            context
                .run_ui(egui::RawInput::default(), |ui| {
                    assert!(!panel.show(ui));
                })
                .textures_delta
                .clear();
            assert_eq!(panel.target, Some(1));
            assert_eq!(panel.source, 0);
        }
        assert!(panel.texture.is_some());
        panel.close();
        assert!(panel.texture.is_none());
    }

    #[test]
    fn pdf_page_requests_are_bounded_and_keep_the_opposite_slot() {
        let (jobs, requests) = mpsc::sync_channel(1);
        let (_sender, results) = mpsc::sync_channel(1);
        let mut panel = ReaderPanel {
            jobs,
            results,
            generation: Arc::new(AtomicU64::new(1)),
            target: Some(1),
            source: 0,
            path: PathBuf::from("synthetic.pdf"),
            preview: Some(Preview {
                path: PathBuf::from("synthetic.pdf"),
                content: Content::Pdf {
                    page: 0,
                    count: 2,
                    width: 1,
                    height: 1,
                    rgba: vec![255; 4],
                },
                notice: "TEST ONLY",
            }),
            error: None,
            texture: None,
            zoom: 1.0,
            offset: 0.0,
            prefix: String::new(),
            filter: String::new(),
            archive_cursor: 0,
            archive_notice: None,
            markdown_cache: Default::default(),
            inspected_link: None,
            quarter_turns: 0,
            edit_requested: false,
            save_requested: false,
            close_prompt: false,
            close_prompt_focus: false,
            recent_activity: None,
            virtual_entry: None,
            open_entry: None,
            selected_archive: Default::default(),
        };
        panel.step_pdf(-1).unwrap();
        assert!(requests.try_recv().is_err());
        panel.step_pdf(1).unwrap();
        let request = requests.try_recv().unwrap();
        assert_eq!(request.page, 1);
        assert_eq!(request.quarter_turns, 0);
        assert_eq!(request.path, PathBuf::from("synthetic.pdf"));
        assert_eq!(panel.target, Some(1));
        assert!(panel.preview.is_none());
        assert!(panel.step_pdf(1).is_err());
        panel.preview = Some(Preview {
            path: panel.path.clone(),
            content: Content::Pdf {
                page: 1,
                count: 2,
                width: 1,
                height: 1,
                rgba: vec![255; 4],
            },
            notice: "TEST ONLY",
        });
        panel.rotate(false).unwrap();
        let request = requests.try_recv().unwrap();
        assert_eq!((request.page, request.quarter_turns), (1, 3));
        assert_eq!(request.path, PathBuf::from("synthetic.pdf"));
        assert_eq!(panel.target, Some(1));
        panel.start(0, PathBuf::from("next.png")).unwrap();
        let request = requests.try_recv().unwrap();
        assert_eq!((request.page, request.quarter_turns), (0, 0));
    }
}
