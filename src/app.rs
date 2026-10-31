use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use eframe::egui::{self, Color32, Id, Key, Modifiers, RichText, Stroke, Vec2};
use tomas_commander::{
    commands,
    files::{self, Entry, FileResult, Listing, Operation, Outcome, Plan, Scope, Sort},
    platform,
    preferences::Preferences,
    recent::{self, Activity as RecentActivity},
    search,
};

mod command_dialog;
mod dialogs;
mod dispatch;
mod drag;
mod frame;
mod keyboard;
mod palette;
mod pane;
mod pane_ui;
mod recent_rail;
mod search_dialog;
mod setup;
#[cfg(test)]
mod tests;
mod theme;
mod work;

use self::{
    command_dialog::*, dialogs::*, drag::*, palette::*, pane::*, recent_rail::*, search_dialog::*,
    theme::*, work::*,
};
pub(crate) use self::{
    dialogs::confirmation_buttons,
    drag::{DragItems, DropLocation, register_drop_zone, remember_drag_reveal},
    frame::action_button,
    keyboard::{accessible_text_value, consume_prompt_key},
};
pub(super) use self::{
    keyboard::take_filter_text,
    pane_ui::{format_size, paint_file_row, reveal_row},
};

pub(super) const ROW_HEIGHT: f32 = 29.0;
pub(super) const FILE_NAME_FONT: egui::FontId = egui::FontId {
    size: 12.0,
    family: egui::FontFamily::Monospace,
};
const SCROLL_GUTTER: f32 = 12.0;
pub const MIN_WINDOW_SIZE: [f32; 2] = [880.0, 560.0];

pub struct Ledger {
    reader: crate::viewer_ui::ReaderPanel,
    archive: crate::viewer_ui::ReaderPanel,
    zip_dialog: Option<ZipDialog>,
    zip_drag: Option<ZipDrag>,
    change_plan: Option<Change>,
    ai: crate::ai_ui::AiPanel,
    file_agent: crate::file_agent_ui::FileAgentPanel,
    answers: crate::answers_ui::AnswerPanel,
    command_dialog: Option<CommandDialog>,
    panes: [Pane; 2],
    path_edits: [bool; 2],
    active: usize,
    scope: Scope,
    preferences: Preferences,
    name_dialog: Option<NameDialog>,
    open_with: Option<OpenWith>,
    settings_open: bool,
    settings_tab: SettingsTab,
    recent_roots_edit: String,
    recent_config_pending: bool,
    operations_open: bool,
    viewport_zoom: f32,
    preferences_writable: bool,
    save_pending: bool,
    saves_in_flight: usize,
    close_after_save: bool,
    scans: Vec<ScanWorker>,
    aggregate_cache: Arc<tomas_commander::directory_cache::DirectoryCache>,
    cache_changes: Receiver<FileResult<tomas_commander::directory_cache::Changes>>,
    scan_results: Receiver<ScanResult>,
    work: SyncSender<Work>,
    work_results: Receiver<WorkResult>,
    cancellation: Arc<AtomicBool>,
    operation_busy: bool,
    plan: Option<Plan>,
    next_id: u64,
    last_outcome: Option<Outcome>,
    rail: Rail,
    rail_index: Option<usize>,
    recent: recent::Monitor,
    recent_snapshot: recent::Snapshot,
    recent_pending: Option<recent::Snapshot>,
    activity: Vec<String>,
    status: String,
    error: Option<String>,
    palette: bool,
    palette_query: String,
    palette_focus: bool,
    palette_index: usize,
    palette_command_selection: bool,
    rail_open: bool,
    places_popup: Option<usize>,
    drives_popup: Option<DrivePicker>,
    place_edit: Option<(PathBuf, String)>,
    inline_pending: Option<(usize, PathBuf, f64)>,
    help_open: bool,
    approval_focus: bool,
    editor_focus: Option<Id>,
    file_focus_requested: bool,
    left_width: f32,
    workspace_width: f32,
    resize_requested: bool,
    balance_requested: bool,
    started: Instant,
    first_frame: bool,
    appearance_dirty: bool,
    search_dialog: Option<SearchDialog>,
    search_jobs: SyncSender<SearchRequest>,
    search_results: Receiver<(u64, FileResult<search::Results>)>,
    search_generation: Arc<AtomicU64>,
    copy_modifiers: Modifiers,
    #[cfg(feature = "native-desktop-tests")]
    desktop_metrics: Option<crate::desktop_metrics::Metrics>,
}

impl Ledger {
    #[cfg(feature = "native-desktop-tests")]
    pub fn enable_desktop_test(&mut self, metrics: crate::desktop_metrics::Metrics) {
        self.desktop_metrics = Some(metrics);
        self.ai.answer_test_mode = true;
        self.answers.enable_test();
    }
    #[cfg(feature = "native-answer-tests")]
    pub fn enable_answer_ui_test(&mut self) {
        self.ai.answer_test_mode = true;
        self.answers.enable_test();
        self.dispatch(Command::Answers);
    }

    #[cfg(feature = "native-search-tests")]
    pub fn enable_search_ui_test(&mut self) {
        self.ai.test_scenario = Some(crate::ai_ui::SearchTestScenario::Ambiguous);
        self.dispatch(Command::AiSearch);
        if let Some(dialog) = &mut self.search_dialog {
            dialog.query = "Synthetic failure-state test; no provider request.".into();
        }
    }

    fn log(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.status = message.clone();
        self.activity.insert(0, message);
        self.activity.truncate(30);
    }

    fn drag_allowed(&self) -> bool {
        self.file_actions_allowed() && !self.panes.iter().any(|pane| pane.busy)
    }

    fn file_actions_allowed(&self) -> bool {
        !self.operation_busy
            && self.plan.is_none()
            && self.change_plan.is_none()
            && self.zip_dialog.is_none()
            && self.name_dialog.is_none()
            && self.open_with.is_none()
            && !self.settings_open
            && !self.operations_open
            && self.error.is_none()
            && !self.palette
            && self.search_dialog.is_none()
            && self.command_dialog.is_none()
            && !self.ai.open
            && !self.file_agent.open
            && !self.answers.open
            && !self.help_open
            && self.places_popup.is_none()
            && self.drives_popup.is_none()
            && self.place_edit.is_none()
            && !self.reader.dirty()
    }

    fn fail(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.log(format!("ERROR / {message}"));
        self.error = Some(message);
    }
}
