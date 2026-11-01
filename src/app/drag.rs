use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DragItems {
    Files(Vec<PathBuf>),
    Zip {
        archive: PathBuf,
        prefix: String,
        selected: Vec<String>,
    },
}

impl DragItems {
    fn count(&self) -> usize {
        match self {
            Self::Files(paths) => paths.len(),
            Self::Zip { selected, .. } => selected.len(),
        }
    }

    pub(super) fn label(&self) -> String {
        match self {
            Self::Files(paths) if paths.len() == 1 => {
                let path = &paths[0];
                files::display_path(path.file_name().map(std::path::Path::new).unwrap_or(path))
            }
            Self::Zip { selected, .. } if selected.len() == 1 => selected[0].clone(),
            _ => format!("{} items", self.count()),
        }
    }
}

pub(super) fn start_file_drag(ctx: &egui::Context, paths: Vec<PathBuf>) {
    remember_drag_reveal(ctx);
    egui::DragAndDrop::set_payload(ctx, DragItems::Files(paths));
}

pub(crate) fn remember_drag_reveal(ctx: &egui::Context) {
    let reveal_target = ctx.input(|input| input.modifiers.alt);
    ctx.data_mut(|data| data.insert_temp(Id::new("file-drag-reveal-target"), reveal_target));
}

pub(super) fn drag_reveal_requested(ctx: &egui::Context) -> bool {
    ctx.data(|data| {
        data.get_temp::<bool>(Id::new("file-drag-reveal-target"))
            .unwrap_or(false)
    }) || ctx.input(|input| input.modifiers.alt && input.pointer.primary_down())
}

#[derive(Clone, Debug)]
pub(crate) enum DropLocation {
    Folder(PathBuf),
    Zip { archive: PathBuf, prefix: String },
}

#[derive(Clone)]
pub(super) struct DropHover {
    pub(super) location: DropLocation,
    priority: u8,
    rect: egui::Rect,
}

pub(crate) fn register_drop_zone(
    ctx: &egui::Context,
    rect: egui::Rect,
    location: DropLocation,
    priority: u8,
) {
    if egui::DragAndDrop::has_payload_of_type::<DragItems>(ctx)
        && ctx.input(|input| {
            input
                .pointer
                .interact_pos()
                .is_some_and(|pos| rect.contains(pos))
        })
    {
        ctx.data_mut(|data| {
            let id = Id::new("file-drop-hover");
            if data
                .get_temp::<DropHover>(id)
                .is_none_or(|hover| priority > hover.priority)
            {
                data.insert_temp(
                    id,
                    DropHover {
                        location,
                        priority,
                        rect,
                    },
                );
            }
        });
    }
}

pub(super) struct ZipDrag {
    pub(super) id: u64,
    items: DragItems,
    cancel: Arc<AtomicBool>,
    pub(super) export: Option<tomas_commander::archive_export::Export>,
}

pub(super) fn drop_work(
    id: u64,
    items: DragItems,
    target: DropLocation,
    copy: bool,
) -> FileResult<Option<Work>> {
    use tomas_commander::archive_changes::Request;
    Ok(Some(match (items, target) {
        (DragItems::Files(sources), DropLocation::Folder(destination)) => {
            if sources.is_empty() {
                return Err("Drag has no source files.".into());
            }
            if sources
                .iter()
                .any(|path| path.parent() == Some(destination.as_path()) || path == &destination)
            {
                return Ok(None);
            }
            Work::Prepare {
                id,
                operation: if copy {
                    Operation::Copy
                } else {
                    Operation::Move
                },
                sources,
                destination,
                policy: ConflictPolicy::Skip,
            }
        }
        (DragItems::Files(sources), DropLocation::Zip { archive, prefix }) => {
            Work::PrepareArchive {
                id,
                request: Request::Insert {
                    archive,
                    prefix,
                    sources,
                    move_sources: !copy,
                },
            }
        }
        (
            DragItems::Zip {
                archive,
                prefix,
                selected,
            },
            DropLocation::Folder(destination),
        ) => Work::PrepareArchive {
            id,
            request: Request::Extract {
                archive,
                prefix,
                selected,
                destination,
                move_entries: !copy,
            },
        },
        (DragItems::Zip { .. }, DropLocation::Zip { .. }) => {
            return Err(
                "ZIP-to-ZIP dragging is unsupported; extract to a real folder first.".into(),
            );
        }
    }))
}

impl Ledger {
    pub(super) fn finish_drag(&mut self, ctx: &egui::Context) {
        let Some(payload) = egui::DragAndDrop::payload::<DragItems>(ctx) else {
            self.clear_zip_drag();
            ctx.data_mut(|data| data.remove::<bool>(Id::new("file-drag-reveal-target")));
            return;
        };
        #[cfg(windows)]
        if self.drag_allowed() {
            if let DragItems::Zip {
                archive, selected, ..
            } = &*payload
                && self
                    .zip_drag
                    .as_ref()
                    .is_none_or(|drag| drag.items != *payload)
            {
                self.clear_zip_drag();
                let id = self.next_id;
                self.next_id += 1;
                let cancel = Arc::new(AtomicBool::new(false));
                match self.work.try_send(Work::ExportDrag {
                    id,
                    archive: archive.clone(),
                    selected: selected.clone(),
                    cancel: cancel.clone(),
                }) {
                    Ok(()) => {
                        self.zip_drag = Some(ZipDrag {
                            id,
                            items: (*payload).clone(),
                            cancel,
                            export: None,
                        })
                    }
                    Err(error) => {
                        egui::DragAndDrop::clear_payload(ctx);
                        self.fail(format!("Could not prepare external ZIP drag: {error}"));
                        return;
                    }
                }
            }
            let reveal_target = drag_reveal_requested(ctx);
            let external = if reveal_target {
                Ok(true)
            } else {
                platform::external_drag::pointer_outside_source()
            };
            match external {
                Ok(true) => {
                    let paths = match &*payload {
                        DragItems::Files(paths) => paths
                            .iter()
                            .map(|path| self.scope.resolve(path))
                            .collect::<FileResult<Vec<_>>>(),
                        DragItems::Zip { .. } => {
                            let Some(export) =
                                self.zip_drag.as_ref().and_then(|drag| drag.export.as_ref())
                            else {
                                ctx.set_cursor_icon(egui::CursorIcon::Progress);
                                ctx.request_repaint_after(Duration::from_millis(16));
                                return;
                            };
                            Ok(export.paths.clone())
                        }
                    };
                    egui::DragAndDrop::clear_payload(ctx);
                    ctx.data_mut(|data| data.remove::<bool>(Id::new("file-drag-reveal-target")));
                    let items = payload.count();
                    let zip = matches!(&*payload, DragItems::Zip { .. });
                    let result = paths.and_then(|paths| {
                        platform::external_drag::copy_files(&paths, reveal_target)
                    });
                    let outcome = match &result {
                        Ok(true) => "accepted",
                        Ok(false) => "cancelled",
                        Err(_) => "error",
                    };
                    self.usage.log.record(|| {
                        Event::new("external_drag")
                            .with("items", items)
                            .with("zip", zip)
                            .with("alt", reveal_target)
                            .with("result", outcome)
                    });
                    ctx.input_mut(|input| input.pointer = Default::default());
                    ctx.request_repaint();
                    match result {
                        Ok(true) => {
                            if let Some(drag) = self.zip_drag.take()
                                && let Some(export) = drag.export
                            {
                                export.retain();
                            }
                            self.log("Files handed to the target application as Copy; sources and ZIP unchanged.");
                        }
                        Ok(false) => self.clear_zip_drag(),
                        Err(error) => {
                            self.clear_zip_drag();
                            self.fail(error);
                        }
                    }
                    return;
                }
                Ok(false) => {}
                Err(error) => {
                    egui::DragAndDrop::clear_payload(ctx);
                    self.fail(error);
                    return;
                }
            }
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        let hover = ctx.data(|data| data.get_temp::<DropHover>(Id::new("file-drop-hover")));
        let copy = ctx.input(|input| input.modifiers.ctrl);
        let supported = !matches!(
            (&*payload, hover.as_ref().map(|hover| &hover.location)),
            (DragItems::Zip { .. }, Some(DropLocation::Zip { .. }))
        );
        let valid = self.drag_allowed() && hover.is_some() && supported;
        if let Some(hover) = &hover {
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                Id::new("file-drop-target"),
            ))
            .rect_stroke(
                hover.rect.shrink(1.0),
                2.0,
                Stroke::new(
                    2.0,
                    if valid {
                        self.accent()
                    } else {
                        Color32::from_rgb(220, 70, 70)
                    },
                ),
                egui::StrokeKind::Inside,
            );
        }
        ctx.set_cursor_icon(if !valid {
            egui::CursorIcon::NotAllowed
        } else if copy {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::Move
        });
        if let Some(pos) = ctx.input(|input| input.pointer.interact_pos()) {
            egui::Area::new(Id::new("file-drag-label"))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .fixed_pos(pos + Vec2::new(18.0, 18.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(320.0);
                        ui.add(egui::Label::new(payload.label()).truncate());
                    });
                });
        }
        if ctx.input(|input| input.pointer.button_released(egui::PointerButton::Primary)) {
            egui::DragAndDrop::clear_payload(ctx);
            if let Some(hover) = hover {
                if !valid {
                    self.fail(if supported {
                        "Finish current work before dropping files."
                    } else {
                        "ZIP-to-ZIP dragging is unsupported; extract to a real folder first."
                    });
                    return;
                }
                self.prepare_drop((*payload).clone(), hover.location, copy);
            }
        }
    }

    fn clear_zip_drag(&mut self) {
        if let Some(drag) = self.zip_drag.take() {
            drag.cancel.store(true, Ordering::Release);
            if let Some(export) = drag.export
                && let Err(error) = export.cleanup()
            {
                self.fail(error);
            }
        }
    }

    fn prepare_drop(&mut self, items: DragItems, target: DropLocation, copy: bool) {
        if !self.drag_allowed() {
            self.fail("Finish current work before dropping files.");
            return;
        }
        match drop_work(self.next_id, items, target, copy) {
            Ok(Some(job)) => {
                let command = if copy { Command::Copy } else { Command::Move };
                self.usage_action(command.slash(), Via::Drag, shortcut_hint(command));
                self.reader.close();
                self.cancellation.store(false, Ordering::Release);
                match self.work.try_send(job) {
                    Ok(()) => {
                        self.next_id += 1;
                        self.operation_busy = true;
                        self.log("Preparing drag-and-drop plan / no effects before approval.");
                    }
                    Err(error) => self.fail(format!("Could not queue drop plan: {error}")),
                }
            }
            Ok(None) => self.log("Drop cancelled: source is already here."),
            Err(error) => self.fail(error),
        }
    }
}
