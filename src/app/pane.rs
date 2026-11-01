use super::pane_ui::TabAction;
use super::*;
use tomas_commander::preferences::PaneTabs;

/// Folder history entries kept per tab and direction.
pub(super) const HISTORY_LIMIT: usize = 50;

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
    pub(super) history: History,
    /// Tabs left to right. The fields above hold the live state of `tabs[tab]`, whose
    /// stored copy is only a placeholder until another tab is activated.
    pub(super) tabs: Vec<Tab>,
    pub(super) tab: usize,
    /// Scroll offset to reapply once a restored tab's listing arrives.
    pub(super) restore_scroll: Option<f32>,
}

/// Saved view state of an inactive tab. Inactive tabs keep no listing, scan or watcher.
pub(super) struct Tab {
    pub(super) path: PathBuf,
    pub(super) sort: Sort,
    pub(super) descending: bool,
    pub(super) filter: String,
    pub(super) focused: Option<PathBuf>,
    pub(super) selected: BTreeSet<PathBuf>,
    pub(super) scroll_offset: f32,
    pub(super) history: History,
    pub(super) parent_returns: Vec<ReturnLocation>,
}

impl Tab {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            sort: Sort::Name,
            descending: false,
            filter: String::new(),
            focused: None,
            selected: BTreeSet::new(),
            scroll_offset: 0.0,
            history: History::default(),
            parent_returns: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Location {
    pub(super) path: PathBuf,
    pub(super) focused: Option<PathBuf>,
}

/// Bounded back/forward folder stacks; the newest entry is last.
#[derive(Default, Debug)]
pub(super) struct History {
    pub(super) back: Vec<Location>,
    pub(super) forward: Vec<Location>,
}

fn push_bounded(stack: &mut Vec<Location>, location: Location) {
    if stack.len() == HISTORY_LIMIT {
        stack.remove(0);
    }
    stack.push(location);
}

impl History {
    /// Records the folder being left by a new navigation and clears Forward.
    pub(super) fn record(&mut self, previous: Location) {
        self.forward.clear();
        if let Some(last) = self.back.last_mut()
            && last.path == previous.path
        {
            *last = previous;
        } else {
            push_bounded(&mut self.back, previous);
        }
    }

    /// Moves to the newest entry in the requested direction that differs from `current`, which
    /// moves to the opposite stack. Nothing is checked on disk: a folder that has disappeared is
    /// replaced by its nearest existing ancestor when its listing loads.
    pub(super) fn step(&mut self, forward: bool, current: Location) -> Option<Location> {
        let (from, to) = if forward {
            (&mut self.forward, &mut self.back)
        } else {
            (&mut self.back, &mut self.forward)
        };
        while let Some(location) = from.pop() {
            if location.path != current.path {
                push_bounded(to, current);
                return Some(location);
            }
        }
        None
    }

    /// Drops neighbouring entries equal to the folder just loaded (for example a typed
    /// path that resolved to the folder that was already open).
    pub(super) fn forget(&mut self, path: &std::path::Path) {
        for stack in [&mut self.back, &mut self.forward] {
            if stack.last().is_some_and(|last| last.path == path) {
                stack.pop();
            }
        }
    }
}

/// Short tab caption: the folder name, or the drive/share for roots.
pub(super) fn folder_label(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| files::display_path(path))
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
            tabs: vec![Tab::new(path.clone())],
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
            history: History::default(),
            tab: 0,
            restore_scroll: None,
        }
    }

    /// A pane with restored tab folders; `active` must index `folders`.
    pub(super) fn with_tabs(folders: Vec<PathBuf>, active: usize) -> Self {
        let mut pane = Self::new(folders[active].clone());
        pane.tabs = folders.into_iter().map(Tab::new).collect();
        pane.tab = active;
        pane
    }

    pub(super) fn tab_path(&self, index: usize) -> &std::path::Path {
        if index == self.tab {
            &self.path
        } else {
            &self.tabs[index].path
        }
    }

    pub(super) fn tab_preferences(&self) -> Option<PaneTabs> {
        PaneTabs::persistable(
            (0..self.tabs.len())
                .map(|index| self.tab_path(index).to_owned())
                .collect(),
            self.tab,
        )
    }

    pub(super) fn location(&self) -> Location {
        Location {
            path: self.path.clone(),
            focused: self.focused.clone(),
        }
    }

    fn store_tab(&mut self) {
        let tab = &mut self.tabs[self.tab];
        tab.path = self.path.clone();
        tab.sort = self.sort;
        tab.descending = self.descending;
        tab.filter = std::mem::take(&mut self.filter);
        tab.focused = self.focused.take();
        tab.selected = std::mem::take(&mut self.selected);
        tab.scroll_offset = self.scroll_offset;
        tab.history = std::mem::take(&mut self.history);
        tab.parent_returns = std::mem::take(&mut self.parent_returns);
    }

    /// Makes `index` live. Its listing must then be loaded like a refresh, keeping the
    /// restored filter, focus and selection.
    fn load_tab(&mut self, index: usize) {
        self.tab = index;
        let tab = std::mem::replace(&mut self.tabs[index], Tab::new(PathBuf::new()));
        self.path_text = files::display_path(&tab.path);
        self.path = tab.path;
        self.sort = tab.sort;
        self.descending = tab.descending;
        self.filter = tab.filter;
        self.focused = tab.focused;
        self.selected = tab.selected;
        self.history = tab.history;
        self.parent_returns = tab.parent_returns;
        self.restore_scroll = Some(tab.scroll_offset);
        self.entries.clear();
        self.visible.clear();
        self.pending_aggregates.clear();
        self.anchor = 0;
        self.scroll_to_focus = false;
        self.busy = true;
        self.warning = None;
    }

    /// Opens the current folder in a new tab right of the active one and activates it.
    /// The listing is shared, so no rescan is needed.
    pub(super) fn open_tab(&mut self) -> bool {
        if self.tabs.len() >= PaneTabs::MAX {
            return false;
        }
        let focused = self.focused.clone();
        self.store_tab();
        self.tabs.insert(self.tab + 1, Tab::new(PathBuf::new()));
        self.tab += 1;
        self.focused = focused;
        self.rebuild();
        self.anchor = self.focus_position();
        self.scroll_to_focus = true;
        true
    }

    /// Activates another tab; returns whether its listing must be loaded.
    pub(super) fn select_tab(&mut self, index: usize) -> bool {
        if index == self.tab || index >= self.tabs.len() {
            return false;
        }
        self.store_tab();
        self.load_tab(index);
        true
    }

    /// Next (or previous) tab with wrap-around; returns whether a listing must be loaded.
    pub(super) fn cycle_tab(&mut self, forward: bool) -> bool {
        let count = self.tabs.len();
        if count < 2 {
            return false;
        }
        self.select_tab(if forward {
            (self.tab + 1) % count
        } else {
            (self.tab + count - 1) % count
        })
    }

    /// Closes a tab, never the last one. Closing the active tab activates its right
    /// neighbour (or the new last tab). `Some(true)` means a listing must be loaded.
    pub(super) fn close_tab(&mut self, index: usize) -> Option<bool> {
        if self.tabs.len() < 2 || index >= self.tabs.len() {
            return None;
        }
        self.tabs.remove(index);
        if index < self.tab {
            self.tab -= 1;
            Some(false)
        } else if index > self.tab {
            Some(false)
        } else {
            self.load_tab(index.min(self.tabs.len() - 1));
            Some(true)
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
            self.restore_scroll = None;
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

    /// Queues the active tab's folder (after activation or at launch) without filesystem
    /// access; the scan worker falls back to its nearest existing ancestor if it has
    /// disappeared or cannot be reached.
    pub(super) fn request_load(&self, scan: &mut ScanWorker) {
        scan.request(self.path.clone(), true);
    }

    /// Back (`forward == false`) or Forward in this tab without filesystem access: the target
    /// is queued on `scan` as is and the scan worker falls back to its nearest existing
    /// ancestor if it has disappeared. `false` when there is no folder in that direction.
    pub(super) fn step_history(&mut self, scan: &mut ScanWorker, forward: bool) -> bool {
        let current = self.location();
        let Some(target) = self.history.step(forward, current) else {
            return false;
        };
        scan.request(target.path.clone(), true);
        self.begin_navigation(target.path);
        if target.focused.is_some() {
            self.focused = target.focused;
        }
        true
    }

    /// Shows a finished listing. `missing` is the requested folder the scan worker replaced by
    /// this nearest existing ancestor: the tab moves there with a fresh view and the returned
    /// note says so. History is untouched, so a missing Back/Forward entry is in effect
    /// replaced by its fallback.
    pub(super) fn show_listing(
        &mut self,
        listing: Listing,
        missing: Option<PathBuf>,
    ) -> Option<String> {
        let note = missing.map(|missing| {
            self.filter.clear();
            self.selected.clear();
            self.restore_scroll = None;
            self.scroll_offset = 0.0;
            self.scroll_to_focus = true;
            self.focused = listing
                .entries
                .iter()
                .find(|entry| missing.starts_with(&entry.path))
                .map(|entry| entry.path.clone());
            format!(
                "Folder {} is not available; showing {}.",
                files::display_path(&missing),
                files::display_path(&listing.path)
            )
        });
        self.path = listing.path;
        self.path_text = files::display_path(&self.path);
        self.history.forget(&self.path);
        self.replace_entries(listing.entries);
        let paths: BTreeSet<_> = self
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        self.selected.retain(|path| paths.contains(path));
        self.rebuild();
        self.anchor = self.focus_position();
        if let Some(offset) = self.restore_scroll.take() {
            self.scroll_offset = offset;
            self.scroll_to_focus = true;
        }
        self.warning = if listing.warnings.is_empty() {
            None
        } else {
            Some(listing.warnings.join("\n"))
        };
        note
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
    /// Opens `path` in pane `index`, recording the folder being left in Back history.
    /// Reloading the same folder (refresh, operation results) records nothing; an unavailable
    /// folder is an error unless a remembered folder is reloaded while still loading.
    pub(super) fn navigate(&mut self, index: usize, path: PathBuf, via: NavVia) {
        self.end_inline_edit();
        if self.panes[index].path != path {
            self.usage_navigate(index, &path, via);
            let pane = &mut self.panes[index];
            let previous = pane.location();
            pane.history.record(previous);
        }
        let fallback = self.scans[index].reload_falls_back(&self.panes[index], &path);
        self.scans[index].request(path.clone(), fallback);
        self.panes[index].begin_navigation(path);
        self.file_focus_requested = true;
    }

    fn end_inline_edit(&mut self) {
        self.inline_pending = None;
        if self
            .name_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.inline)
        {
            self.name_dialog = None;
        }
    }

    /// Back (`forward == false`) or Forward in the active tab of pane `index`.
    pub(super) fn step_history(&mut self, index: usize, forward: bool) {
        let from = self.panes[index].path.clone();
        if self.panes[index].step_history(&mut self.scans[index], forward) {
            self.end_inline_edit();
            self.file_focus_requested = true;
            self.usage_arrived(
                index,
                from,
                if forward {
                    NavVia::HistoryForward
                } else {
                    NavVia::HistoryBack
                },
            );
        } else {
            let direction = if forward { "Forward" } else { "Back" };
            self.log(format!("{direction}: no more folders."));
        }
    }

    /// Leaves the active tab of pane `index`: closes its ZIP view (the tab keeps the
    /// folder containing the ZIP, focused on it) and any inline editor.
    fn leave_tab(&mut self, index: usize) {
        if self.archive.target == Some(index) {
            self.archive.close();
            if self.reader.is_virtual() {
                self.reader.close();
            }
        }
        self.inline_pending = None;
        if self
            .name_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.inline && dialog.pane == index)
        {
            self.name_dialog = None;
        }
        self.path_edits[index] = false;
    }

    /// Loads the active tab of pane `index` like a refresh, keeping its restored filter, focus
    /// and selection. Only the scan worker touches the folder; if it has disappeared or cannot
    /// be reached, the worker shows the nearest existing ancestor instead, so an offline share
    /// never blocks the window.
    pub(super) fn load_active_tab(&mut self, index: usize) {
        self.panes[index].request_load(&mut self.scans[index]);
        self.file_focus_requested = true;
    }

    pub(super) fn new_tab(&mut self, index: usize) {
        if self.panes[index].tabs.len() >= PaneTabs::MAX {
            self.log(format!("Tab limit reached ({} per pane).", PaneTabs::MAX));
            return;
        }
        self.leave_tab(index);
        self.panes[index].open_tab();
        self.file_focus_requested = true;
        self.usage_tab(index, "new");
    }

    pub(super) fn activate_tab(&mut self, index: usize, tab: usize) {
        if tab == self.panes[index].tab || tab >= self.panes[index].tabs.len() {
            return;
        }
        let from = self.panes[index].path.clone();
        self.leave_tab(index);
        if self.panes[index].select_tab(tab) {
            self.load_active_tab(index);
        }
        self.usage_tab(index, "switch");
        self.usage_arrived(index, from, NavVia::TabSwitch);
    }

    pub(super) fn cycle_tab(&mut self, index: usize, forward: bool) {
        if self.panes[index].tabs.len() < 2 {
            self.log("Only one tab is open in this pane.");
            return;
        }
        let from = self.panes[index].path.clone();
        self.leave_tab(index);
        if self.panes[index].cycle_tab(forward) {
            self.load_active_tab(index);
        }
        self.usage_tab(index, if forward { "next" } else { "previous" });
        self.usage_arrived(index, from, NavVia::TabSwitch);
    }

    pub(super) fn close_tab(&mut self, index: usize, tab: usize) {
        if self.panes[index].tabs.len() < 2 {
            self.log("The last tab cannot be closed.");
            return;
        }
        let from = self.panes[index].path.clone();
        if tab == self.panes[index].tab {
            self.leave_tab(index);
        }
        if self.panes[index].close_tab(tab) == Some(true) {
            self.load_active_tab(index);
        }
        self.usage_tab(index, "close");
        self.usage_arrived(index, from, NavVia::TabSwitch);
    }

    pub(super) fn apply_tab_action(&mut self, index: usize, action: TabAction) {
        self.active = index;
        self.rail_index = None;
        let (name, command) = match action {
            TabAction::Activate(_) => ("/switch-tab", Command::NextTab),
            TabAction::Close(_) => ("/close-tab", Command::CloseTab),
            TabAction::New => ("/new-tab", Command::NewTab),
        };
        self.usage_action(name, Via::TabStrip, shortcut_hint(command));
        match action {
            TabAction::Activate(tab) => self.activate_tab(index, tab),
            TabAction::Close(tab) => self.close_tab(index, tab),
            TabAction::New => self.new_tab(index),
        }
    }

    /// Current tab folders for preferences, unless this run neither restores nor saves tabs
    /// (see [`super::persist_tabs`]).
    pub(super) fn sync_tab_preferences(&mut self) {
        if !self.persist_tabs || !self.preferences_writable {
            return;
        }
        let tabs = [
            self.panes[0].tab_preferences(),
            self.panes[1].tab_preferences(),
        ];
        if self.preferences.tabs != tabs {
            self.preferences.tabs = tabs;
            self.save_pending = true;
        }
    }

    pub(super) fn parent(&mut self, index: usize, via: NavVia) {
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
            self.navigate(index, parent.to_owned(), via);
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
        self.navigate(index, path, NavVia::Return);
        let pane = &mut self.panes[index];
        pane.parent_returns = history;
        pane.focused = location.focused;
        pane.filter = location.filter;
        pane.scroll_offset = location.scroll_offset;
        pane.scroll_to_focus = false;
    }

    /// Enter or double-click (`via`) on the focused row: folders open in the pane, ZIPs in the
    /// archive view, other files in their Windows application.
    pub(super) fn open_focused(&mut self, via: NavVia) {
        let pane = &self.panes[self.active];
        let Some(path) = pane.focused.clone() else {
            self.parent(self.active, NavVia::Parent);
            return;
        };
        let Some(directory) = pane
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .map(|entry| entry.directory)
        else {
            return;
        };
        if directory {
            self.navigate(self.active, path, via);
            return;
        }
        let action = if via == NavVia::DoubleClick {
            Via::DoubleClick
        } else {
            Via::Shortcut
        };
        self.usage_filter_opened(self.active);
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
        {
            self.usage_action("/open-zip", action, "Enter");
            if let Err(error) = self.archive.open_archive(self.active, path) {
                self.fail(error);
            }
        } else {
            self.usage_action("/open", action, "Enter");
            if let Err(error) = self.work.try_send(Work::Open(path)) {
                self.fail(format!("Could not queue file opening: {error}"));
            }
        }
    }
}
