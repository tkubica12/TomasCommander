use super::*;

pub(super) struct Pane {
    pub(super) path: PathBuf,
    pub(super) path_text: String,
    pub(super) entries: Vec<Entry>,
    pub(super) pending_aggregates: BTreeSet<PathBuf>,
    pub(super) visible: Vec<usize>,
    pub(super) filter: String,
    pub(super) sort: Sort,
    pub(super) descending: bool,
    pub(super) focused: Option<PathBuf>,
    pub(super) selected: BTreeSet<PathBuf>,
    pub(super) anchor: usize,
    pub(super) scroll_to_focus: bool,
    pub(super) scroll_offset: f32,
    pub(super) busy: bool,
    pub(super) warning: Option<String>,
    pub(super) parent_returns: Vec<ReturnLocation>,
}

pub(super) struct ReturnLocation {
    pub(super) path: PathBuf,
    pub(super) focused: Option<PathBuf>,
    pub(super) filter: String,
    pub(super) scroll_offset: f32,
}

impl Pane {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path_text: files::display_path(&path),
            path,
            entries: Vec::new(),
            pending_aggregates: BTreeSet::new(),
            visible: Vec::new(),
            filter: String::new(),
            sort: Sort::Name,
            descending: false,
            focused: None,
            selected: BTreeSet::new(),
            anchor: 0,
            scroll_to_focus: false,
            scroll_offset: 0.0,
            busy: true,
            warning: None,
            parent_returns: Vec::new(),
        }
    }

    pub(super) fn cycle_sort(&mut self) {
        (self.sort, self.descending) = match self.sort {
            Sort::Name => (Sort::Modified, true),
            Sort::Modified => (Sort::Size, true),
            Sort::Size => (Sort::Name, false),
        };
        files::sort_entries_ordered(&mut self.entries, self.sort, self.descending);
        self.rebuild();
        self.anchor = self.focus_position();
        self.scroll_to_focus = true;
    }

    pub(super) fn rebuild(&mut self) {
        let query = search::fold(&self.filter);
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| search::fold(&entry.name).contains(&query))
            .map(|(index, _)| index)
            .collect();
        if self.focused.is_some()
            && !self
                .visible
                .iter()
                .any(|index| Some(&self.entries[*index].path) == self.focused.as_ref())
        {
            self.focused = self
                .visible
                .first()
                .map(|index| self.entries[*index].path.clone());
            self.anchor = usize::from(!self.visible.is_empty());
            self.scroll_to_focus = true;
        }
        self.anchor = self.anchor.min(self.visible.len());
    }

    pub(super) fn focus_position(&self) -> usize {
        self.visible
            .iter()
            .position(|index| Some(&self.entries[*index].path) == self.focused.as_ref())
            .map(|position| position + 1)
            .unwrap_or(0)
    }

    pub(super) fn targets(&self) -> Vec<PathBuf> {
        if self.selected.is_empty() {
            self.focused
                .iter()
                .filter(|path| self.entries.iter().any(|entry| &entry.path == *path))
                .cloned()
                .collect()
        } else {
            self.selected.iter().cloned().collect()
        }
    }

    pub(super) fn drag_targets(&self, path: &PathBuf) -> Vec<PathBuf> {
        if self.selected.contains(path) {
            self.selected.iter().cloned().collect()
        } else {
            vec![path.clone()]
        }
    }

    pub(super) fn row_path(&self, position: usize) -> Option<PathBuf> {
        position
            .checked_sub(1)
            .and_then(|position| self.visible.get(position))
            .map(|index| self.entries[*index].path.clone())
    }

    pub(super) fn jump(&mut self, down: bool) {
        let previous = self.focus_position();
        let next = if down {
            previous.saturating_add(10).min(self.visible.len())
        } else {
            previous.saturating_sub(10)
        };
        self.focused = self.row_path(next);
        self.anchor = next;
        self.scroll_to_focus = true;
    }

    pub(super) fn begin_navigation(&mut self, path: PathBuf) {
        let refresh = self.path == path;
        let child = (self.path.parent() == Some(path.as_path())).then(|| self.path.clone());
        self.path_text = files::display_path(&path);
        self.path = path;
        if !refresh {
            self.parent_returns.clear();
            self.filter.clear();
            self.entries.clear();
            self.pending_aggregates.clear();
            self.visible.clear();
            self.selected.clear();
            self.focused = child;
            self.scroll_offset = 0.0;
            self.scroll_to_focus = true;
        }
        if refresh {
            self.pending_aggregates = self
                .entries
                .iter()
                .filter(|entry| entry.directory && !entry.link)
                .map(|entry| entry.path.clone())
                .collect();
        }
        self.busy = true;
        self.warning = None;
    }

    pub(super) fn replace_entries(&mut self, mut entries: Vec<Entry>) {
        let previous: BTreeMap<_, _> = self
            .entries
            .iter()
            .filter(|entry| entry.directory && !entry.link)
            .filter_map(|entry| match &entry.aggregate {
                Some(Ok(stats)) => Some((&entry.path, stats)),
                _ => None,
            })
            .collect();
        self.pending_aggregates = entries
            .iter()
            .filter(|entry| entry.directory && !entry.link && entry.aggregate.is_none())
            .map(|entry| entry.path.clone())
            .collect();
        for entry in &mut entries {
            if self.pending_aggregates.contains(&entry.path)
                && let Some(stats) = previous.get(&entry.path)
            {
                entry.aggregate = Some(Ok((*stats).clone()));
            }
        }
        files::sort_entries_ordered(&mut entries, self.sort, self.descending);
        self.entries = entries;
    }

    pub(super) fn apply_aggregate(
        &mut self,
        path: &std::path::Path,
        stats: FileResult<files::DirectoryStats>,
    ) -> bool {
        self.pending_aggregates.remove(path);
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.path == path && entry.directory && !entry.link)
        {
            entry.aggregate = Some(stats);
            true
        } else {
            false
        }
    }

    pub(super) fn remember_parent_return(&mut self) -> Vec<ReturnLocation> {
        let location = ReturnLocation {
            path: self.path.clone(),
            focused: self.focused.clone(),
            filter: self.filter.clone(),
            scroll_offset: self.scroll_offset,
        };
        let mut history = std::mem::take(&mut self.parent_returns);
        if history.len() == 64 {
            history.remove(0);
        }
        history.push(location);
        history
    }
}

impl Ledger {
    pub(super) fn navigate(&mut self, index: usize, path: PathBuf) {
        self.inline_pending = None;
        if self
            .name_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.inline)
        {
            self.name_dialog = None;
        }
        let worker = &mut self.scans[index];
        let generation = worker.generation.fetch_add(1, Ordering::Relaxed) + 1;
        worker.pending = Some(ScanRequest {
            path: path.clone(),
            generation,
        });
        self.panes[index].begin_navigation(path);
        self.file_focus_requested = true;
    }

    pub(super) fn parent(&mut self, index: usize) {
        let path = self.panes[index].path.clone();
        if self.scope.root() == Some(path.as_path()) {
            self.log("Parent unavailable at fixture boundary.");
        } else if path.parent().is_none() {
            match platform::logical_drives() {
                Ok(drives) => {
                    self.inline_pending = None;
                    self.rail_index = None;
                    self.drives_popup = Some(DrivePicker::new(index, drives, &path));
                }
                Err(error) => self.fail(error),
            }
        } else if let Some(parent) = path.parent() {
            let history = self.panes[index].remember_parent_return();
            self.navigate(index, parent.to_owned());
            self.panes[index].parent_returns = history;
        }
    }

    pub(super) fn return_inside(&mut self, index: usize) {
        let Some(location) = self.panes[index].parent_returns.last() else {
            return;
        };
        let path = match self.scope.resolve(&location.path) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => {
                self.fail("The saved return folder is no longer a directory.");
                return;
            }
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let mut history = std::mem::take(&mut self.panes[index].parent_returns);
        let location = history.pop().expect("Return history was checked above");
        self.navigate(index, path);
        let pane = &mut self.panes[index];
        pane.parent_returns = history;
        pane.focused = location.focused;
        pane.filter = location.filter;
        pane.scroll_offset = location.scroll_offset;
        pane.scroll_to_focus = false;
    }

    pub(super) fn open_focused(&mut self) {
        let pane = &self.panes[self.active];
        if pane.focused.is_none() {
            self.parent(self.active);
            return;
        }
        if let Some(path) = &pane.focused {
            let entry = pane.entries.iter().find(|entry| &entry.path == path);
            if let Some(entry) = entry {
                if entry.directory {
                    self.navigate(self.active, path.clone());
                } else if path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
                {
                    if let Err(error) = self.archive.open_archive(self.active, path.clone()) {
                        self.fail(error);
                    }
                } else {
                    if let Err(error) = self.work.try_send(Work::Open(path.clone())) {
                        self.fail(format!("Could not queue file opening: {error}"));
                    }
                }
            }
        }
    }
}
