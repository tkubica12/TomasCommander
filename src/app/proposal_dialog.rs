//! Review of an AI change proposal: grouped rows with checkboxes, dependency handling, and a
//! guarded Run button. Approval starts a background batch that checks every selected step and
//! then runs each one as an ordinary journaled operation.

use super::*;
use tomas_commander::{
    file_tools::Settings,
    proposal::{self, Batch, Kind, Proposal},
};

pub(super) struct ProposalReview {
    pub(super) proposal: Proposal,
    pub(super) settings: Settings,
    pub(super) settings_path: PathBuf,
    pub(super) checked: Vec<bool>,
    /// Unchecked only because their new folder was unchecked; rechecking it restores them.
    cascaded: Vec<bool>,
    /// Common folder of every shown path; rows are shown relative to it when shorter.
    root: Option<PathBuf>,
    /// A dry run of the selection is in progress.
    pub(super) checking: bool,
    /// Problems from the latest dry run; nothing was started.
    pub(super) failures: Vec<String>,
    /// Dry runs of this review that found problems.
    pub(super) failed_checks: usize,
    pub(super) error: Option<String>,
    /// Requests initial keyboard focus on Run.
    pub(super) focus: bool,
    pub(super) opened: Instant,
    modal: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ProposalChoice {
    Run,
    Cancel,
}

/// Common ancestor folder of `paths`, by case-insensitive components.
fn common_root<'a>(paths: impl Iterator<Item = &'a std::path::Path>) -> Option<PathBuf> {
    let mut common: Option<PathBuf> = None;
    for path in paths {
        let path = PathBuf::from(files::display_path(path));
        let parent = path.parent()?.to_path_buf();
        common = Some(match common {
            None => parent,
            Some(common) => {
                let mut shared = PathBuf::new();
                for (a, b) in common.components().zip(parent.components()) {
                    if !a
                        .as_os_str()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
                    {
                        break;
                    }
                    shared.push(a);
                }
                shared
            }
        });
    }
    common.filter(|root| root.components().count() > 1)
}

impl ProposalReview {
    pub(super) fn new(proposal: Proposal, settings: Settings, settings_path: PathBuf) -> Self {
        let checked = proposal
            .steps
            .iter()
            .map(|step| step.kind != Kind::Recycle)
            .collect();
        let root = common_root(
            proposal
                .steps
                .iter()
                .flat_map(|step| step.before().into_iter().chain(step.after())),
        );
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self {
            cascaded: vec![false; proposal.steps.len()],
            proposal,
            settings,
            settings_path,
            checked,
            root,
            checking: false,
            failures: Vec::new(),
            failed_checks: 0,
            error: None,
            focus: true,
            opened: Instant::now(),
            modal: NEXT.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// The unchecked or unavailable new folder step `index` needs, if any.
    pub(super) fn blocked(&self, index: usize) -> Option<usize> {
        let folder = self.proposal.steps[index].dependency()?;
        if !self.checked[folder] || self.blocked(folder).is_some() {
            Some(folder)
        } else {
            None
        }
    }

    /// Checks or unchecks a row. Unchecking a new folder also unchecks the changes that need
    /// it; checking it again restores those.
    pub(super) fn set(&mut self, index: usize, value: bool) {
        if value && self.blocked(index).is_some() {
            return;
        }
        self.checked[index] = value;
        self.cascaded[index] = false;
        self.cascade(index, value);
    }

    fn cascade(&mut self, folder: usize, value: bool) {
        for dependent in self.proposal.dependents(folder) {
            if !value && self.checked[dependent] {
                self.checked[dependent] = false;
                self.cascaded[dependent] = true;
                self.cascade(dependent, false);
            } else if value && self.cascaded[dependent] {
                self.checked[dependent] = true;
                self.cascaded[dependent] = false;
                self.cascade(dependent, true);
            }
        }
    }

    /// Rows that will run: checked and not blocked.
    pub(super) fn selection(&self) -> Vec<bool> {
        (0..self.checked.len())
            .map(|index| self.checked[index] && self.blocked(index).is_none())
            .collect()
    }

    pub(super) fn selected(&self) -> usize {
        self.selection().into_iter().filter(|value| *value).count()
    }

    pub(super) fn unchecked_deletes(&self) -> usize {
        let selection = self.selection();
        self.proposal
            .steps
            .iter()
            .zip(selection)
            .filter(|(step, selected)| step.kind == Kind::Recycle && !selected)
            .count()
    }

    fn short(&self, path: &std::path::Path) -> String {
        let full = files::display_path(path);
        match &self.root {
            Some(root) => match PathBuf::from(&full).strip_prefix(root) {
                Ok(relative) if !relative.as_os_str().is_empty() => relative.display().to_string(),
                _ => full,
            },
            None => full,
        }
    }

    /// "a.pdf → PDF\a.pdf", "(new) → PDF", "setup.exe → Recycle Bin".
    pub(super) fn describe(&self, index: usize) -> String {
        let step = &self.proposal.steps[index];
        let before = step
            .before()
            .map_or_else(|| "(new folder)".to_owned(), |path| self.short(path));
        let after = step
            .after()
            .map_or_else(|| "Recycle Bin".to_owned(), |path| self.short(path));
        format!("{before} → {after}")
    }

    fn row(&mut self, ui: &mut egui::Ui, index: usize, space: bool) {
        let step = &self.proposal.steps[index];
        let blocked = self.blocked(index);
        let mut value = self.checked[index] && blocked.is_none();
        let mut text = self.describe(index);
        if let Some(folder) = blocked {
            text.push_str(&format!(
                "  (needs unchecked new folder {})",
                self.short(&self.proposal.steps[folder].target)
            ));
        }
        let text = if step.kind == Kind::Recycle {
            RichText::new(format!("DELETE  {text}")).color(ui.visuals().warn_fg_color)
        } else {
            RichText::new(text)
        };
        let dependents = self.proposal.dependents(index).len();
        let mut hover = step.reason.clone();
        if dependents > 0 {
            hover.push_str(&format!(
                "\n{dependents} later {} this folder.",
                if dependents == 1 {
                    "change needs"
                } else {
                    "changes need"
                }
            ));
        }
        let response = ui
            .push_id(("ai-proposal-row", index), |ui| {
                ui.add_enabled(
                    blocked.is_none() && !self.checking,
                    egui::Checkbox::new(&mut value, text),
                )
            })
            .inner
            .on_hover_text(&hover)
            .on_disabled_hover_text(match blocked {
                Some(folder) => format!(
                    "Needs the new folder {}, which is unchecked. Check it to enable this change.\n{hover}",
                    self.short(&self.proposal.steps[folder].target)
                ),
                None => hover.clone(),
            });
        if response.gained_focus() {
            response.scroll_to_me(None);
        }
        // Space is taken from the input before the rows are drawn, so it toggles only the
        // focused row and never a button.
        if space && response.has_focus() && blocked.is_none() && !self.checking {
            value = !self.checked[index];
        }
        if value != (self.checked[index] && blocked.is_none()) {
            self.set(index, value);
        }
    }

    /// Draws the review. Enter confirms only while Run has keyboard focus; a repeated (held)
    /// Enter never confirms; Escape cancels.
    pub(super) fn show(&mut self, ctx: &egui::Context) -> Option<ProposalChoice> {
        let mut choice = None;
        let modal = egui::Modal::new(Id::new(("ai-proposal", self.modal)))
            .show(ctx, |ui| choice = self.content(ui));
        if modal.should_close() && choice.is_none() {
            choice = Some(ProposalChoice::Cancel);
        }
        choice
    }

    /// The review's contents; keys are taken from the input before any widget sees them.
    pub(super) fn content(&mut self, ui: &mut egui::Ui) -> Option<ProposalChoice> {
        let mut choice = None;
        ui.set_width(760.0);
        let (mut enter, mut escape, mut space, mut key_event) = (false, false, false, false);
        ui.input_mut(|input| {
            input.events.retain(|event| {
                if let egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } = event
                    && matches!(key, Key::Enter | Key::Escape | Key::Space)
                {
                    if *pressed {
                        key_event |= *key != Key::Space;
                        if modifiers.is_none() && !repeat {
                            enter |= *key == Key::Enter;
                            space |= *key == Key::Space;
                        }
                        escape |= *key == Key::Escape && modifiers.is_none();
                    }
                    false
                } else {
                    true
                }
            });
        });
        let total = self.proposal.steps.len();
        ui.heading(format!(
            "Review AI proposal: {total} {}",
            if total == 1 { "change" } else { "changes" }
        ));
        ui.add(egui::Label::new(self.proposal.summary.as_str()).wrap());
        ui.label(
            RichText::new(
                "Nothing has changed yet. Checked changes run in order, each as a normal operation that is planned again first: no overwrite, deletes go to the Recycle Bin, Undo in Operations.",
            )
            .small(),
        );
        if let Some(root) = &self.root {
            ui.label(
                RichText::new(format!("Paths relative to {}", root.display()))
                    .small()
                    .weak(),
            );
        }
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("ai-proposal-rows")
            .max_height(340.0)
            .show(ui, |ui| {
                for kind in Kind::ALL {
                    let rows: Vec<usize> = (0..total)
                        .filter(|index| self.proposal.steps[*index].kind == kind)
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    let heading =
                        RichText::new(format!("{} ({})", kind.section(), rows.len())).strong();
                    ui.label(if kind == Kind::Recycle {
                        heading.color(ui.visuals().warn_fg_color)
                    } else {
                        heading
                    });
                    if kind == Kind::Recycle {
                        ui.label(
                            RichText::new(
                                "Unchecked by default; check only what you want deleted.",
                            )
                            .small()
                            .color(ui.visuals().warn_fg_color),
                        );
                    }
                    for index in rows {
                        self.row(ui, index, space);
                    }
                }
            });
        ui.collapsing("Details", |ui| {
            egui::ScrollArea::vertical()
                .id_salt("ai-proposal-details")
                .max_height(160.0)
                .show(ui, |ui| {
                    for (index, step) in self.proposal.steps.iter().enumerate() {
                        ui.add(
                            egui::Label::new(format!(
                                "{}. {}\n   {}",
                                index + 1,
                                step.label(),
                                step.reason
                            ))
                            .wrap(),
                        );
                    }
                });
        });
        if self.checking {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Checking the selected changes against the current files...");
            });
        }
        if !self.failures.is_empty() {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "Not started; nothing changed. Uncheck or fix these changes:",
            );
            egui::ScrollArea::vertical()
                .id_salt("ai-proposal-failures")
                .max_height(120.0)
                .show(ui, |ui| {
                    for failure in &self.failures {
                        ui.add(
                            egui::Label::new(
                                RichText::new(failure).color(ui.visuals().error_fg_color),
                            )
                            .wrap(),
                        );
                    }
                });
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.separator();
        let selected = self.selected();
        let enabled = selected > 0 && !self.checking;
        ui.horizontal(|ui| {
            let cancel = ui.button("Cancel").on_hover_text("Escape");
            let run = ui
                .add_enabled(
                    enabled,
                    egui::Button::new(format!("Run selected ({selected})")),
                )
                .on_hover_text("Enter while focused");
            // Focus is placed once, when the review opens: on Run when something is selected,
            // otherwise on Cancel. Checking a row later never moves focus to Run, so a
            // following Enter cannot start a delete-only proposal by surprise.
            if self.focus {
                if selected > 0 {
                    run.request_focus();
                } else {
                    cancel.request_focus();
                }
                self.focus = false;
            }
            let cancelled =
                (cancel.clicked() && !key_event) || escape || (enter && cancel.has_focus());
            if cancelled {
                choice = Some(ProposalChoice::Cancel);
            } else if enabled && ((run.clicked() && !key_event) || (enter && run.has_focus())) {
                choice = Some(ProposalChoice::Run);
            }
            let deletes = self.proposal.count(Kind::Recycle);
            if deletes > 0 {
                let unchecked = self.unchecked_deletes();
                ui.label(
                    RichText::new(format!(
                        "{} of {deletes} deletes selected",
                        deletes - unchecked
                    ))
                    .small()
                    .color(ui.visuals().warn_fg_color),
                );
            }
        });
        choice
    }
}

impl Ledger {
    pub(super) fn open_proposal(
        &mut self,
        proposal: Proposal,
        settings: Settings,
        settings_path: PathBuf,
    ) {
        self.proposal_review = Some(ProposalReview::new(proposal, settings, settings_path));
    }

    /// The review modal; returns whether it owns this frame.
    pub(super) fn proposal_dialog(&mut self, ctx: &egui::Context) -> bool {
        let Some(review) = &mut self.proposal_review else {
            return false;
        };
        match review.show(ctx) {
            Some(ProposalChoice::Run) => self.start_batch(),
            Some(ProposalChoice::Cancel) => {
                if review.checking {
                    self.cancellation.store(true, Ordering::Release);
                }
                self.close_proposal("cancelled");
                self.log("AI proposal closed; nothing changed. The bell reopens it.");
            }
            None => (),
        }
        true
    }

    /// Queues the selected steps: the worker checks them all first and starts nothing if any
    /// check fails.
    pub(super) fn start_batch(&mut self) {
        let busy = self.operation_busy
            || self.plan.is_some()
            || self.change_plan.is_some()
            || self.reader.dirty()
            || self.panes.iter().any(|pane| pane.busy);
        let Some(review) = &mut self.proposal_review else {
            return;
        };
        if busy {
            review.error = Some(
                "Finish pending work and save or discard edits before running changes.".into(),
            );
            return;
        }
        let batch = match Batch::new(
            &review.proposal,
            &review.selection(),
            self.next_id,
            review.settings.clone(),
            review.settings_path.clone(),
        ) {
            Ok(batch) => batch,
            Err(error) => {
                review.error = Some(error);
                return;
            }
        };
        let steps = batch.steps.len() as u64;
        review.failures.clear();
        review.error = None;
        self.cancellation.store(false, Ordering::Release);
        match self.work.try_send(Work::Batch(Box::new(batch))) {
            Ok(()) => {
                self.next_id += steps;
                if let Some(review) = &mut self.proposal_review {
                    review.checking = true;
                }
                self.operation_busy = true;
                self.log(format!(
                    "Checking {steps} selected AI changes; nothing runs unless every check passes."
                ));
            }
            Err(error) => {
                if let Some(review) = &mut self.proposal_review {
                    review.error = Some(format!("The changes were not queued: {error}"));
                }
            }
        }
    }

    /// Closes the review and records its `ai_proposal` usage event.
    pub(super) fn close_proposal(&mut self, outcome: &'static str) {
        let Some(review) = self.proposal_review.take() else {
            return;
        };
        self.usage.outcome(
            "ai_proposal",
            if outcome == "run" {
                "confirmed"
            } else {
                "cancelled"
            },
        );
        let ops = Kind::ALL
            .into_iter()
            .map(|kind| (kind.key().to_owned(), review.proposal.count(kind) as u64))
            .filter(|(_, count)| *count > 0)
            .collect();
        let checked = review.selected();
        let unchecked_deletes = review.unchecked_deletes();
        let failed_checks = review.failed_checks;
        let open_ms = review.opened.elapsed();
        self.usage.log.record(|| {
            Event::new("ai_proposal")
                .with("ops", tomas_commander::usage::Value::Counts(ops))
                .with("checked", checked)
                .with("unchecked_deletes", unchecked_deletes)
                .with("outcome", outcome)
                .with("failed_checks", failed_checks)
                .with("open_ms", open_ms)
        });
        self.file_focus_requested = true;
    }

    pub(super) fn batch_checked(&mut self, result: Result<(), Vec<String>>) {
        match result {
            Ok(()) => {
                if self.proposal_review.is_some() {
                    self.file_agent.proposal_started();
                    self.close_proposal("run");
                }
                self.log("Running the approved AI changes, one journaled operation per step...");
            }
            Err(failures) => {
                self.operation_busy = false;
                let count = failures.len();
                if let Some(review) = &mut self.proposal_review {
                    review.checking = false;
                    review.failures = failures;
                    review.failed_checks += 1;
                    review.focus = true;
                }
                self.log(format!(
                    "AI changes not started: the check found {count} {}; nothing changed.",
                    if count == 1 { "problem" } else { "problems" }
                ));
            }
        }
    }

    pub(super) fn batch_step(&mut self, index: usize, total: usize, outcome: Outcome) {
        if let Err(error) = self.recent.record_many(
            outcome.created.iter().chain(&outcome.completed).cloned(),
            RecentActivity::FileAction,
        ) {
            self.log(format!("ERROR / {error}"));
        }
        self.log(format!(
            "AI changes: step {} of {total} {}",
            index + 1,
            match (&outcome.error, outcome.cancelled) {
                (Some(error), _) => format!("failed: {error}"),
                (None, true) => "cancelled".to_owned(),
                (None, false) => "done".to_owned(),
            }
        ));
    }

    pub(super) fn finish_batch(&mut self, report: proposal::Report) {
        self.operation_busy = false;
        self.history = None;
        let summary = report.summary();
        let (steps, ran, done) = (report.steps.len(), report.ran(), report.done());
        let failed_at = report.failed_at().map(|index| index + 1);
        let cancelled = report.cancelled;
        let elapsed = report.elapsed;
        self.usage.log.record(|| {
            Event::new("ai_batch")
                .with("steps", steps)
                .with("run", ran)
                .with("done", done)
                .with("failed_at", failed_at)
                .with("cancelled", cancelled)
                .with("duration_ms", elapsed)
        });
        self.log(summary.clone());
        if failed_at.is_some() {
            self.fail(format!(
                "{summary}\nEarlier steps stay done and can be undone in Operations."
            ));
        }
        if let Err(error) = self.aggregate_cache.clear() {
            self.fail(error);
        }
        for index in 0..2 {
            self.navigate(index, self.panes[index].path.clone(), NavVia::Other);
        }
        self.last_batch = Some(report);
        if self.operations_open {
            self.request_history();
        }
    }

    /// Queues the reviewed undo of a whole batch.
    pub(super) fn prepare_undo_batch(&mut self, batch: String) {
        if self.operation_busy || self.plan.is_some() || self.change_plan.is_some() {
            self.fail("Finish or cancel the current operation first.");
            return;
        }
        self.cancellation.store(false, Ordering::Release);
        match self.work.try_send(Work::PrepareUndoBatch {
            id: self.next_id,
            batch,
        }) {
            Ok(()) => {
                self.next_id += 1;
                self.operation_busy = true;
                self.log("Preparing batch undo / no effects before approval.");
            }
            Err(error) => self.fail(format!("Batch undo was not queued: {error}")),
        }
    }
}
