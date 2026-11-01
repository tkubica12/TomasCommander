//! Aggregates the local usage log into a self-explanatory Markdown report for periodic analysis
//! by the user or an AI assistant. Malformed lines are counted and skipped; reading happens on
//! the caller's (background or CLI) thread.

use super::{DAY_MS, check_folder, date_text, local_day, local_offset_minutes, owned_files};
use crate::files::{self, FileResult};
use serde_json::Value as Json;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

pub const DEFAULT_DAYS: u32 = 30;
pub const MAX_DAYS: u32 = 365;
/// Rows per table.
const TOP: usize = 20;
/// Navigation steps closer than this belong to one chain.
const CHAIN_GAP_MS: u64 = 15_000;
/// A folder reached by at least this many steps ...
const CHAIN_STEPS: u64 = 3;
/// ... at least this often is a Places candidate.
const CANDIDATE_CHAINS: u64 = 2;

/// `--usage-report [--days N] [--settings <path>] [--out <file>]` (arguments after the flag).
#[derive(Debug, PartialEq, Eq)]
pub struct CliArgs {
    pub days: u32,
    pub settings: Option<PathBuf>,
    pub out: Option<PathBuf>,
}

impl CliArgs {
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut parsed = Self {
            days: DEFAULT_DAYS,
            settings: None,
            out: None,
        };
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--days") => {
                    let days = arguments
                        .next()
                        .and_then(|value| value.to_str().and_then(|value| value.parse().ok()))
                        .ok_or("--days requires a whole number of days")?;
                    if !(1..=MAX_DAYS).contains(&days) {
                        return Err(format!("--days must be between 1 and {MAX_DAYS}."));
                    }
                    parsed.days = days;
                }
                Some("--settings") => {
                    parsed.settings =
                        Some(arguments.next().ok_or("--settings requires a path")?.into())
                }
                Some("--out") => {
                    parsed.out = Some(arguments.next().ok_or("--out requires a path")?.into())
                }
                _ => {
                    return Err(format!(
                        "Unknown --usage-report argument: {}",
                        argument.to_string_lossy()
                    ));
                }
            }
        }
        Ok(parsed)
    }
}

/// How a `via` value was physically invoked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputClass {
    Keyboard,
    Mouse,
    Ask,
    Other,
}

pub fn input_class(via: &str) -> InputClass {
    match via {
        "shortcut" => InputClass::Keyboard,
        "button" | "context_menu" | "drag" | "double_click" | "click" | "rail" | "tab_strip"
        | "header" | "mouse_button" | "wheel" => InputClass::Mouse,
        "palette" | "slash" => InputClass::Ask,
        _ => InputClass::Other,
    }
}

#[derive(Clone, Debug, Default)]
pub struct Timing {
    pub count: u64,
    pub total_ms: u64,
    pub max_ms: u64,
}

impl Timing {
    fn add(&mut self, ms: u64) {
        self.count += 1;
        self.total_ms += ms;
        self.max_ms = self.max_ms.max(ms);
    }

    pub fn average_ms(&self) -> u64 {
        self.total_ms.checked_div(self.count).unwrap_or(0)
    }
}

#[derive(Debug, Default)]
pub struct CommandStats {
    pub total: u64,
    pub via: BTreeMap<String, u64>,
    pub shortcut: String,
    pub failed: u64,
}

impl CommandStats {
    pub fn count(&self, class: InputClass) -> u64 {
        self.via
            .iter()
            .filter(|(via, _)| input_class(via) == class)
            .map(|(_, count)| count)
            .sum()
    }
}

#[derive(Debug, Default)]
pub struct DialogStats {
    pub opened: u64,
    pub outcomes: BTreeMap<String, u64>,
    pub open: Timing,
    pub extras: BTreeMap<String, u64>,
}

#[derive(Debug, Default)]
pub struct FolderStats {
    pub visits: u64,
    pub via: BTreeMap<String, u64>,
}

/// A folder repeatedly reached through several navigation steps.
#[derive(Debug, PartialEq, Eq)]
pub struct Candidate {
    pub folder: String,
    pub chains: u64,
    pub steps: u64,
    /// Arrivals through saved places or Recent.
    pub direct: u64,
}

#[derive(Debug, Default)]
pub struct OperationStats {
    pub timing: Timing,
    pub items: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub policies: BTreeMap<String, u64>,
}

#[derive(Debug)]
struct Chain {
    steps: u64,
    last_t: u64,
    to: String,
    last_via: String,
}

/// Aggregated usage of a period; see [`Report::markdown`].
#[derive(Debug, Default)]
pub struct Report {
    pub days: u32,
    pub from_ms: u64,
    pub to_ms: u64,
    pub tz_minutes: i32,
    pub files: usize,
    pub lines: u64,
    pub malformed: u64,
    pub sessions: BTreeSet<String>,
    pub starts: u64,
    pub ends: u64,
    pub duration_ms: u64,
    pub active_ms: u64,
    pub idle_ms: u64,
    pub dropped: u64,
    pub versions: BTreeMap<String, u64>,
    pub kinds: BTreeMap<String, u64>,
    pub commands: BTreeMap<String, CommandStats>,
    pub unhandled: BTreeMap<(String, String), u64>,
    pub ask: BTreeMap<String, u64>,
    pub unmet_ask: BTreeMap<String, (u64, String)>,
    pub dialogs: BTreeMap<String, DialogStats>,
    pub errors: BTreeMap<String, u64>,
    pub slow_folders: BTreeMap<String, (Timing, u64)>,
    pub slow_actions: BTreeMap<String, Timing>,
    pub slow_frames: BTreeMap<String, Timing>,
    pub folders: BTreeMap<String, FolderStats>,
    pub nav_via: BTreeMap<String, u64>,
    pub candidates: Vec<Candidate>,
    pub tabs: BTreeMap<String, u64>,
    pub filters: BTreeMap<String, u64>,
    pub empty_filters: BTreeMap<String, u64>,
    pub searches: Timing,
    pub search_opened: u64,
    pub empty_searches: BTreeMap<String, u64>,
    pub ai_tasks: BTreeMap<String, Timing>,
    pub ai_prompts: Vec<(u64, String, String, u64)>,
    pub ai_opened: BTreeMap<u64, u64>,
    pub previews: BTreeMap<String, Timing>,
    pub preview_failures: u64,
    pub operations: BTreeMap<String, OperationStats>,
    pub settings: BTreeMap<String, u64>,
    pub counters: BTreeMap<String, u64>,
    pub other: BTreeMap<String, u64>,
    chains: BTreeMap<(String, i64), Chain>,
    chain_ends: BTreeMap<String, (u64, u64)>,
}

fn text<'a>(value: &'a Json, key: &str) -> &'a str {
    value.get(key).and_then(Json::as_str).unwrap_or("")
}

fn number(value: &Json, key: &str) -> u64 {
    value
        .get(key)
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_f64().map(|value| value.max(0.0) as u64))
        })
        .unwrap_or(0)
}

fn bump(map: &mut BTreeMap<String, u64>, key: &str) {
    add(map, key, 1);
}

fn add(map: &mut BTreeMap<String, u64>, key: &str, count: u64) {
    match map.get_mut(key) {
        Some(value) => *value += count,
        None => {
            map.insert(key.to_owned(), count);
        }
    }
}

impl Report {
    pub fn new(days: u32, now_ms: u64) -> Self {
        Self {
            days,
            from_ms: now_ms.saturating_sub(u64::from(days) * DAY_MS as u64),
            to_ms: now_ms,
            tz_minutes: local_offset_minutes(now_ms),
            ..Default::default()
        }
    }

    /// Reads the day files of the last `days` days from `dir`; a missing folder is empty.
    pub fn load(dir: &Path, days: u32, now_ms: u64) -> FileResult<Self> {
        let mut report = Self::new(days, now_ms);
        if !check_folder(dir)? {
            report.finish();
            return Ok(report);
        }
        let first = local_day(report.from_ms, report.tz_minutes) - 1;
        let last = local_day(now_ms, report.tz_minutes) + 1;
        let mut day_files: Vec<_> = owned_files(dir, |name| super::parse_day_file(name).is_some())?
            .into_iter()
            .filter_map(|(name, path, _)| {
                let day = super::parse_day_file(&name)?;
                (first..=last).contains(&day).then_some((day, path))
            })
            .collect();
        day_files.sort();
        for (_, path) in day_files {
            let bytes = fs::read(&path).map_err(|error| {
                format!("Read usage file {}: {error}", files::display_path(&path))
            })?;
            report.files += 1;
            for line in bytes.split(|byte| *byte == b'\n') {
                report.line(line);
            }
        }
        report.finish();
        Ok(report)
    }

    /// Aggregates already loaded lines.
    pub fn from_lines<'a>(
        lines: impl IntoIterator<Item = &'a str>,
        days: u32,
        now_ms: u64,
    ) -> Self {
        let mut report = Self::new(days, now_ms);
        for line in lines {
            report.line(line.as_bytes());
        }
        report.finish();
        report
    }

    fn line(&mut self, line: &[u8]) {
        let line = line.trim_ascii();
        if line.is_empty() {
            return;
        }
        match serde_json::from_slice::<Json>(line) {
            Ok(value) if value.get("k").and_then(Json::as_str).is_some() => self.ingest(&value),
            _ => self.malformed += 1,
        }
    }

    fn ingest(&mut self, value: &Json) {
        let t = number(value, "t");
        if t < self.from_ms || t > self.to_ms + DAY_MS as u64 {
            return;
        }
        let kind = text(value, "k");
        self.lines += 1;
        bump(&mut self.kinds, kind);
        let session = text(value, "s");
        if !session.is_empty() && !self.sessions.contains(session) {
            self.sessions.insert(session.to_owned());
        }
        match kind {
            "session_start" => {
                self.starts += 1;
                bump(&mut self.versions, text(value, "version"));
            }
            "session_end" => {
                self.ends += 1;
                self.duration_ms += number(value, "duration_ms");
                self.active_ms += number(value, "active_ms");
                self.idle_ms += number(value, "idle_ms");
                self.dropped += number(value, "dropped");
            }
            "command" => {
                let command = text(value, "cmd");
                if !self.commands.contains_key(command) {
                    self.commands
                        .insert(command.to_owned(), CommandStats::default());
                }
                let stats = self.commands.get_mut(command).expect("inserted above");
                stats.total += 1;
                bump(&mut stats.via, text(value, "via"));
                let shortcut = text(value, "shortcut");
                if !shortcut.is_empty() && stats.shortcut.is_empty() {
                    stats.shortcut = shortcut.to_owned();
                }
                if value.get("ok") == Some(&Json::Bool(false)) {
                    stats.failed += 1;
                }
            }
            "key_unhandled" => {
                *self
                    .unhandled
                    .entry((
                        text(value, "key").to_owned(),
                        text(value, "focus").to_owned(),
                    ))
                    .or_default() += 1;
            }
            "ask" => {
                let outcome = text(value, "outcome");
                let class = outcome.split(':').next().unwrap_or(outcome);
                bump(&mut self.ask, class);
                let prompt = text(value, "text").trim();
                if !prompt.is_empty() && matches!(outcome, "no_match" | "cancelled") {
                    let entry = self
                        .unmet_ask
                        .entry(prompt.to_owned())
                        .or_insert((0, String::new()));
                    entry.0 += 1;
                    entry.1 = outcome.to_owned();
                }
            }
            "dialog" => {
                let name = text(value, "name");
                if !self.dialogs.contains_key(name) {
                    self.dialogs.insert(name.to_owned(), DialogStats::default());
                }
                let stats = self.dialogs.get_mut(name).expect("inserted above");
                stats.opened += 1;
                bump(&mut stats.outcomes, text(value, "outcome"));
                stats.open.add(number(value, "open_ms"));
                let extra = text(value, "extra");
                if !extra.is_empty() {
                    bump(&mut stats.extras, extra);
                }
            }
            "error" => {
                let message: String = text(value, "message").chars().take(300).collect();
                bump(&mut self.errors, &message);
            }
            "slow_folder" => {
                let entry = self
                    .slow_folders
                    .entry(text(value, "path").to_owned())
                    .or_default();
                entry.0.add(number(value, "ms"));
                entry.1 = entry.1.max(number(value, "entries"));
            }
            "slow_action" => self
                .slow_actions
                .entry(text(value, "cmd").to_owned())
                .or_default()
                .add(number(value, "ms")),
            "slow_frame" => self
                .slow_frames
                .entry(text(value, "context").to_owned())
                .or_default()
                .add(number(value, "ms")),
            "navigate" => self.navigate(value, t, session),
            "tab" => bump(&mut self.tabs, text(value, "action")),
            "filter" => {
                bump(&mut self.filters, text(value, "outcome"));
                if number(value, "matches") == 0 {
                    bump(&mut self.empty_filters, text(value, "query"));
                }
            }
            "search" => {
                self.searches.add(number(value, "duration_ms"));
                if value.get("opened_rank").is_some_and(|rank| !rank.is_null()) {
                    self.search_opened += 1;
                }
                if number(value, "results") == 0 {
                    bump(&mut self.empty_searches, text(value, "query"));
                }
            }
            "ai_task" => {
                let outcome = text(value, "outcome");
                self.ai_tasks
                    .entry(outcome.to_owned())
                    .or_default()
                    .add(number(value, "duration_ms"));
                self.ai_prompts.push((
                    t,
                    text(value, "prompt").to_owned(),
                    outcome.to_owned(),
                    number(value, "results"),
                ));
            }
            "ai_result" => *self.ai_opened.entry(number(value, "rank")).or_default() += 1,
            "preview" => {
                if value.get("ok") == Some(&Json::Bool(false)) {
                    self.preview_failures += 1;
                } else if matches!(text(value, "action"), "page" | "rotate") {
                    bump(
                        &mut self.other,
                        &format!("preview: {}", text(value, "action")),
                    );
                } else {
                    let ext = text(value, "ext");
                    self.previews
                        .entry(if ext.is_empty() { "(none)" } else { ext }.to_owned())
                        .or_default()
                        .add(number(value, "open_ms"));
                }
            }
            "operation" => {
                let stats = self
                    .operations
                    .entry(text(value, "op").to_owned())
                    .or_default();
                stats.timing.add(number(value, "duration_ms"));
                stats.items += number(value, "items");
                if value.get("error").is_some_and(|error| !error.is_null()) {
                    stats.failed += 1;
                }
                if value.get("cancelled") == Some(&Json::Bool(true)) {
                    stats.cancelled += 1;
                }
                let policy = text(value, "policy");
                if !policy.is_empty() {
                    bump(&mut stats.policies, policy);
                }
            }
            "settings_change" => bump(&mut self.settings, text(value, "key")),
            "summary" => {
                if let Some(Json::Object(counts)) = value.get("counts") {
                    for (name, count) in counts {
                        add(&mut self.counters, name, count.as_u64().unwrap_or(0));
                    }
                }
                for key in ["frames", "slow_frames"] {
                    add(&mut self.counters, key, number(value, key));
                }
            }
            "open_with" => bump(
                &mut self.other,
                &format!("open_with: {}", text(value, "choice")),
            ),
            "recent_rail" | "zip" | "external_drag" | "editor_save" | "cap_reached" => {
                let detail = ["action", "result", "ext", "scope"]
                    .iter()
                    .map(|key| text(value, key))
                    .find(|detail| !detail.is_empty())
                    .unwrap_or("");
                bump(&mut self.other, &format!("{kind}: {detail}"));
            }
            _ => {}
        }
    }

    fn navigate(&mut self, value: &Json, t: u64, session: &str) {
        let to = text(value, "to");
        let via = text(value, "via");
        if !self.folders.contains_key(to) {
            self.folders.insert(to.to_owned(), FolderStats::default());
        }
        let folder = self.folders.get_mut(to).expect("inserted above");
        folder.visits += 1;
        bump(&mut folder.via, via);
        bump(&mut self.nav_via, via);
        if via == "tab_switch" {
            return;
        }
        let pane = value.get("pane").and_then(Json::as_i64).unwrap_or(0);
        let key = (session.to_owned(), pane);
        match self.chains.get_mut(&key) {
            Some(chain) if t.saturating_sub(chain.last_t) <= CHAIN_GAP_MS => {
                chain.steps += 1;
                chain.last_t = t;
                to.clone_into(&mut chain.to);
                via.clone_into(&mut chain.last_via);
            }
            _ => {
                let chain = Chain {
                    steps: 1,
                    last_t: t,
                    to: to.to_owned(),
                    last_via: via.to_owned(),
                };
                if let Some(previous) = self.chains.insert(key, chain) {
                    self.close_chain(previous);
                }
            }
        }
    }

    fn close_chain(&mut self, chain: Chain) {
        if chain.steps >= CHAIN_STEPS && !matches!(chain.last_via.as_str(), "place" | "recent") {
            let entry = self.chain_ends.entry(chain.to).or_default();
            entry.0 += 1;
            entry.1 += chain.steps;
        }
    }

    fn finish(&mut self) {
        for chain in std::mem::take(&mut self.chains).into_values() {
            self.close_chain(chain);
        }
        let mut candidates: Vec<_> = std::mem::take(&mut self.chain_ends)
            .into_iter()
            .filter(|(_, (chains, _))| *chains >= CANDIDATE_CHAINS)
            .map(|(folder, (chains, steps))| {
                let direct = self.folders.get(&folder).map_or(0, |stats| {
                    ["place", "recent"]
                        .iter()
                        .map(|via| stats.via.get(*via).copied().unwrap_or(0))
                        .sum()
                });
                Candidate {
                    folder,
                    chains,
                    steps,
                    direct,
                }
            })
            .collect();
        candidates.sort_by(|a, b| b.chains.cmp(&a.chains).then(a.folder.cmp(&b.folder)));
        self.candidates = candidates;
        self.ai_prompts
            .sort_by_key(|prompt| std::cmp::Reverse(prompt.0));
    }

    /// Commands used at least three times, at least half of them by mouse, although they have a
    /// keyboard shortcut.
    pub fn shortcut_opportunities(&self) -> Vec<(&str, &CommandStats)> {
        let mut found: Vec<_> = self
            .commands
            .iter()
            .filter(|(_, stats)| {
                !stats.shortcut.is_empty()
                    && stats.total >= 3
                    && stats.count(InputClass::Mouse) * 2 >= stats.total
            })
            .map(|(command, stats)| (command.as_str(), stats))
            .collect();
        found.sort_by(|a, b| {
            b.1.count(InputClass::Mouse)
                .cmp(&a.1.count(InputClass::Mouse))
        });
        found
    }

    pub fn markdown(&self, dir: &Path) -> String {
        let mut out = String::new();
        let today = local_day(self.to_ms, self.tz_minutes);
        let first = local_day(self.from_ms, self.tz_minutes);
        let _ = writeln!(out, "# Tomas Commander usage report\n");
        let _ = writeln!(
            out,
            "Period: last {} days, {} to {} (local dates). Generated from {} log files.\n",
            self.days,
            date_text(first),
            date_text(today),
            self.files
        );
        out.push_str(HOW_TO_READ);
        self.totals(&mut out);
        self.commands_section(&mut out);
        self.keys_section(&mut out);
        self.ask_section(&mut out);
        self.dialogs_section(&mut out);
        self.errors_section(&mut out);
        self.slow_section(&mut out);
        self.navigation_section(&mut out);
        self.search_section(&mut out);
        self.ai_section(&mut out);
        self.work_section(&mut out);
        self.counters_section(&mut out);
        section(&mut out, "Raw data");
        let _ = writeln!(
            out,
            "JSON Lines files `YYYY-MM-DD.jsonl` (one per local day) in {}. Each line is one \
             event: `t` (Unix ms, UTC), `tz` (local UTC offset in minutes), `s` (session id), \
             `k` (event kind) and kind-specific fields described in \
             `docs\\decisions\\usage-log.md`. Files are kept for 90 days.",
            code(&files::display_path(dir))
        );
        out
    }

    fn totals(&self, out: &mut String) {
        section(out, "Totals");
        let rows = [
            ("Sessions (distinct)", self.sessions.len().to_string()),
            (
                "Session starts / clean ends",
                format!("{} / {}", self.starts, self.ends),
            ),
            ("Session time (ended sessions)", duration(self.duration_ms)),
            ("Active time (input within 5 min)", duration(self.active_ms)),
            ("Idle time", duration(self.idle_ms)),
            ("Events", self.lines.to_string()),
            ("Malformed lines skipped", self.malformed.to_string()),
            ("Events dropped while recording", self.dropped.to_string()),
            (
                "Versions",
                self.versions
                    .iter()
                    .map(|(version, count)| format!("{version} ({count})"))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        ];
        table(
            out,
            &["Metric", "Value"],
            rows.iter()
                .map(|(metric, value)| vec![(*metric).to_owned(), value.clone()]),
        );
        if let Some(kinds) = inline_counts(&self.kinds) {
            let _ = writeln!(out, "Event kinds: {kinds}.\n");
        }
    }

    fn commands_section(&self, out: &mut String) {
        section(out, "Commands and how they were invoked");
        let mut commands: Vec<_> = self.commands.iter().collect();
        commands.sort_by(|a, b| b.1.total.cmp(&a.1.total).then(a.0.cmp(b.0)));
        table(
            out,
            &[
                "Command",
                "Uses",
                "Keyboard",
                "Mouse",
                "Ask",
                "Shortcut",
                "Refused/failed",
                "Routes",
            ],
            commands.iter().take(TOP * 2).map(|(command, stats)| {
                vec![
                    code(command),
                    stats.total.to_string(),
                    percent(stats.count(InputClass::Keyboard), stats.total),
                    percent(stats.count(InputClass::Mouse), stats.total),
                    percent(stats.count(InputClass::Ask), stats.total),
                    stats.shortcut.clone(),
                    stats.failed.to_string(),
                    inline_counts(&stats.via).unwrap_or_default(),
                ]
            }),
        );
        let _ = writeln!(out, "### Shortcut opportunities\n");
        let _ = writeln!(
            out,
            "Commands used at least 3 times, at least half of them with the mouse, although a \
             keyboard shortcut exists.\n"
        );
        table(
            out,
            &["Command", "Uses", "Mouse", "Shortcut"],
            self.shortcut_opportunities()
                .iter()
                .map(|(command, stats)| {
                    vec![
                        code(command),
                        stats.total.to_string(),
                        percent(stats.count(InputClass::Mouse), stats.total),
                        stats.shortcut.clone(),
                    ]
                }),
        );
    }

    fn keys_section(&self, out: &mut String) {
        section(out, "Unhandled keys");
        let _ = writeln!(
            out,
            "Keys with Ctrl/Alt, function keys and keys such as Escape or Delete that no handler \
             used. Repeated entries often reveal habits from other tools (for example F5 or \
             Delete from Total Commander or Explorer).\n"
        );
        let mut keys: Vec<_> = self.unhandled.iter().collect();
        keys.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        table(
            out,
            &["Key", "Focus", "Count"],
            keys.iter()
                .take(TOP)
                .map(|((key, focus), count)| vec![code(key), focus.clone(), count.to_string()]),
        );
    }

    fn ask_section(&self, out: &mut String) {
        section(out, "Ask (Ctrl+Space)");
        let _ = writeln!(
            out,
            "Outcomes: {}.\n",
            inline_counts(&self.ask).unwrap_or_else(|| "none".into())
        );
        let _ = writeln!(
            out,
            "### Unmet intent: texts that matched nothing or were cancelled\n"
        );
        let mut unmet: Vec<_> = self.unmet_ask.iter().collect();
        unmet.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
        table(
            out,
            &["Text", "Count", "Last outcome"],
            unmet.iter().take(TOP).map(|(prompt, (count, outcome))| {
                vec![code(prompt), count.to_string(), outcome.clone()]
            }),
        );
    }

    fn dialogs_section(&self, out: &mut String) {
        section(out, "Dialogs");
        let mut dialogs: Vec<_> = self.dialogs.iter().collect();
        let cancelled = |stats: &DialogStats| stats.outcomes.get("cancelled").copied().unwrap_or(0);
        dialogs.sort_by(|a, b| {
            cancelled(b.1)
                .cmp(&cancelled(a.1))
                .then(b.1.opened.cmp(&a.1.opened))
                .then(a.0.cmp(b.0))
        });
        table(
            out,
            &[
                "Dialog",
                "Opened",
                "Confirmed",
                "Cancelled",
                "Closed",
                "Average open",
                "Details",
            ],
            dialogs.iter().take(TOP).map(|(name, stats)| {
                let outcome = |key: &str| stats.outcomes.get(key).copied().unwrap_or(0).to_string();
                vec![
                    name.to_string(),
                    stats.opened.to_string(),
                    outcome("confirmed"),
                    outcome("cancelled"),
                    outcome("closed"),
                    duration(stats.open.average_ms()),
                    inline_counts(&stats.extras).unwrap_or_default(),
                ]
            }),
        );
    }

    fn errors_section(&self, out: &mut String) {
        section(out, "Errors shown to the user");
        table(
            out,
            &["Message", "Count"],
            sorted(&self.errors)
                .iter()
                .take(TOP)
                .map(|(message, count)| vec![code(message), count.to_string()]),
        );
    }

    fn slow_section(&self, out: &mut String) {
        section(out, "Slow work");
        let _ = writeln!(out, "### Slow folders (listing over 300 ms)\n");
        let mut folders: Vec<_> = self.slow_folders.iter().collect();
        folders.sort_by(|a, b| b.1.0.max_ms.cmp(&a.1.0.max_ms).then(a.0.cmp(b.0)));
        table(
            out,
            &["Folder", "Times", "Average", "Slowest", "Entries"],
            folders.iter().take(TOP).map(|(path, (timing, entries))| {
                vec![
                    code(path),
                    timing.count.to_string(),
                    duration(timing.average_ms()),
                    duration(timing.max_ms),
                    entries.to_string(),
                ]
            }),
        );
        timing_table(
            out,
            "Slow actions (preparation over 500 ms)",
            "Command",
            &self.slow_actions,
        );
        timing_table(
            out,
            "Slow frames (over 50 ms of UI work)",
            "Context",
            &self.slow_frames,
        );
    }

    fn navigation_section(&self, out: &mut String) {
        section(out, "Navigation");
        let _ = writeln!(
            out,
            "Routes: {}.\n",
            inline_counts(&self.nav_via).unwrap_or_else(|| "none".into())
        );
        let _ = writeln!(out, "### Most visited folders\n");
        let mut folders: Vec<_> = self.folders.iter().collect();
        folders.sort_by(|a, b| b.1.visits.cmp(&a.1.visits).then(a.0.cmp(b.0)));
        table(
            out,
            &["Folder", "Visits", "Routes"],
            folders.iter().take(TOP).map(|(folder, stats)| {
                vec![
                    code(folder),
                    stats.visits.to_string(),
                    inline_counts(&stats.via).unwrap_or_default(),
                ]
            }),
        );
        let _ = writeln!(out, "### Places candidates\n");
        let _ = writeln!(
            out,
            "Folders reached at least {CANDIDATE_CHAINS} times at the end of a chain of \
             {CHAIN_STEPS}+ navigation steps (steps less than 15 s apart in one pane). Saving \
             them as places (Alt+Up) or using Recent would shorten these routes.\n"
        );
        table(
            out,
            &[
                "Folder",
                "Chains",
                "Average steps",
                "Reached via place/Recent",
            ],
            self.candidates.iter().take(TOP).map(|candidate| {
                vec![
                    code(&candidate.folder),
                    candidate.chains.to_string(),
                    format!(
                        "{:.1}",
                        candidate.steps as f64 / candidate.chains.max(1) as f64
                    ),
                    candidate.direct.to_string(),
                ]
            }),
        );
        let _ = writeln!(
            out,
            "### Tabs and history\n\nTab actions: {}. Back: {}, Forward: {}.\n",
            inline_counts(&self.tabs).unwrap_or_else(|| "none".into()),
            self.nav_via.get("history_back").copied().unwrap_or(0),
            self.nav_via.get("history_forward").copied().unwrap_or(0)
        );
        let _ = writeln!(
            out,
            "### Filters\n\nFilter sessions by outcome: {}.\n",
            inline_counts(&self.filters).unwrap_or_else(|| "none".into())
        );
        table(
            out,
            &["Filter with no matches", "Count"],
            sorted(&self.empty_filters)
                .iter()
                .take(TOP)
                .map(|(query, count)| vec![code(query), count.to_string()]),
        );
    }

    fn search_section(&self, out: &mut String) {
        section(out, "Recursive filename search");
        let _ = writeln!(
            out,
            "{} searches, average {}, {} with an opened result.\n",
            self.searches.count,
            duration(self.searches.average_ms()),
            self.search_opened
        );
        table(
            out,
            &["Query without results", "Count"],
            sorted(&self.empty_searches)
                .iter()
                .take(TOP)
                .map(|(query, count)| vec![code(query), count.to_string()]),
        );
    }

    fn ai_section(&self, out: &mut String) {
        section(out, "AI tasks");
        table(
            out,
            &["Outcome", "Tasks", "Average duration"],
            self.ai_tasks.iter().map(|(outcome, timing)| {
                vec![
                    outcome.clone(),
                    timing.count.to_string(),
                    duration(timing.average_ms()),
                ]
            }),
        );
        let _ = writeln!(
            out,
            "Opened results by rank (0 = first): {}.\n",
            if self.ai_opened.is_empty() {
                "none".to_owned()
            } else {
                self.ai_opened
                    .iter()
                    .map(|(rank, count)| format!("#{rank}: {count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        );
        let _ = writeln!(out, "### Latest prompts\n");
        table(
            out,
            &["Prompt", "Outcome", "Results"],
            self.ai_prompts
                .iter()
                .take(TOP)
                .map(|(_, prompt, outcome, results)| {
                    vec![code(prompt), outcome.clone(), results.to_string()]
                }),
        );
    }

    fn work_section(&self, out: &mut String) {
        section(out, "Previews, operations and settings");
        let _ = writeln!(out, "Preview failures: {}.\n", self.preview_failures);
        timing_table(out, "Previews by extension", "Extension", &self.previews);
        let _ = writeln!(out, "### Operations\n");
        table(
            out,
            &[
                "Operation",
                "Count",
                "Items",
                "Average",
                "Slowest",
                "Failed",
                "Cancelled",
                "Conflict policies",
            ],
            self.operations.iter().map(|(op, stats)| {
                vec![
                    op.clone(),
                    stats.timing.count.to_string(),
                    stats.items.to_string(),
                    duration(stats.timing.average_ms()),
                    duration(stats.timing.max_ms),
                    stats.failed.to_string(),
                    stats.cancelled.to_string(),
                    inline_counts(&stats.policies).unwrap_or_default(),
                ]
            }),
        );
        let _ = writeln!(
            out,
            "Settings changes: {}.\n\nOther: {}.\n",
            inline_counts(&self.settings).unwrap_or_else(|| "none".into()),
            inline_counts(&self.other).unwrap_or_else(|| "none".into())
        );
    }

    fn counters_section(&self, out: &mut String) {
        section(out, "Input summary");
        let _ = writeln!(
            out,
            "Aggregated counts from periodic `summary` events: `key.*` keys pressed outside text \
             fields, `typed.*` typed characters by focus, `pointer.*` mouse buttons and wheel, \
             `frames` and `slow_frames` rendered frames.\n"
        );
        table(
            out,
            &["Counter", "Count"],
            sorted(&self.counters)
                .iter()
                .take(TOP * 3)
                .map(|(name, count)| vec![code(name), count.to_string()]),
        );
    }
}

const HOW_TO_READ: &str = "## How to read this

- This is a local usage log of one person's use of Tomas Commander, a keyboard-first two-pane \
file manager. Nothing in it was uploaded; it contains full folder/file paths and Ask texts.
- Counts are raw event counts in the period. Keyboard, Mouse and Ask percentages of a command \
add up to 100 % unless some uses came from other routes.
- `Keyboard` means a shortcut; `Mouse` means a button, context menu, click, double-click, drag, \
rail, tab strip, pane header control, mouse side button or Ctrl+wheel; `Ask` means the Ctrl+Space palette (picked from the \
list or typed as `/command`).
- `Refused/failed` counts commands that ended in an error message (for example nothing \
selected or work still running).
- Good questions: which frequent commands still go through the mouse, which keys are pressed \
in vain, what Ask texts found nothing, which dialogs are mostly cancelled, which folders are \
reached by long routes, and what is slow.

";

fn section(out: &mut String, title: &str) {
    let _ = writeln!(out, "## {title}\n");
}

fn table(out: &mut String, headers: &[&str], rows: impl IntoIterator<Item = Vec<String>>) {
    let mut rows = rows.into_iter().peekable();
    if rows.peek().is_none() {
        out.push_str("_No data._\n\n");
        return;
    }
    let _ = writeln!(out, "| {} |", headers.join(" | "));
    let _ = writeln!(out, "|{}", "---|".repeat(headers.len()));
    for row in rows {
        let cells: Vec<_> = row
            .iter()
            .map(|cell| cell.replace(['\r', '\n'], " "))
            .collect();
        let _ = writeln!(out, "| {} |", cells.join(" | "));
    }
    out.push('\n');
}

fn timing_table(out: &mut String, title: &str, label: &str, rows: &BTreeMap<String, Timing>) {
    let _ = writeln!(out, "### {title}\n");
    let mut rows: Vec<_> = rows.iter().collect();
    rows.sort_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(b.0)));
    table(
        out,
        &[label, "Times", "Average", "Slowest"],
        rows.iter().take(TOP).map(|(name, timing)| {
            vec![
                code(name),
                timing.count.to_string(),
                duration(timing.average_ms()),
                duration(timing.max_ms),
            ]
        }),
    );
}

/// Text as a Markdown code span, so paths keep their backslashes; pipes are escaped for tables.
fn code(text: &str) -> String {
    let text = text
        .chars()
        .take(200)
        .collect::<String>()
        .replace(['\r', '\n'], " ")
        .replace('|', "\\|");
    if text.trim().is_empty() {
        "(empty)".into()
    } else if text.contains('`') {
        format!("`` {text} ``")
    } else {
        format!("`{text}`")
    }
}

fn sorted(map: &BTreeMap<String, u64>) -> Vec<(&String, &u64)> {
    let mut rows: Vec<_> = map.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    rows
}

fn inline_counts(map: &BTreeMap<String, u64>) -> Option<String> {
    (!map.is_empty()).then(|| {
        sorted(map)
            .iter()
            .map(|(name, count)| {
                format!(
                    "{} {count}",
                    if name.is_empty() {
                        "(none)"
                    } else {
                        name.as_str()
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    })
}

fn percent(part: u64, total: u64) -> String {
    (part * 100 + total / 2)
        .checked_div(total)
        .map_or_else(|| "-".into(), |share| format!("{share} %"))
}

/// `850 ms`, `12 s`, `4 min 05 s`, `3 h 07 min`.
pub fn duration(ms: u64) -> String {
    match ms {
        0..1_000 => format!("{ms} ms"),
        1_000..60_000 => format!("{} s", ms / 1_000),
        60_000..3_600_000 => format!("{} min {:02} s", ms / 60_000, ms / 1_000 % 60),
        _ => format!("{} h {:02} min", ms / 3_600_000, ms / 60_000 % 60),
    }
}

/// `usage\report-YYYY-MM-DD.md` for the local date of `now_ms`.
pub fn export_path(dir: &Path, now_ms: u64) -> PathBuf {
    dir.join(format!(
        "report-{}.md",
        date_text(local_day(now_ms, local_offset_minutes(now_ms)))
    ))
}

/// Writes `markdown` to [`export_path`], creating the usage folder if needed.
pub fn export(dir: &Path, markdown: &str, now_ms: u64) -> FileResult<PathBuf> {
    if !check_folder(dir)? {
        fs::create_dir_all(dir).map_err(|error| {
            format!("Create usage folder {}: {error}", files::display_path(dir))
        })?;
        check_folder(dir)?;
    }
    let path = export_path(dir, now_ms);
    if let Ok(metadata) = fs::symlink_metadata(&path)
        && (files::reparse(&metadata) || !metadata.is_file())
    {
        return Err(format!(
            "{} is a link or not a file; the report was not written.",
            files::display_path(&path)
        ));
    }
    fs::write(&path, markdown)
        .map_err(|error| format!("Write usage report {}: {error}", files::display_path(&path)))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{Event, Value, day_file_name};

    const NOW: u64 = 1_793_448_000_000;

    fn line(offset_ms: u64, session: &str, event: Event) -> String {
        Event {
            at_ms: NOW - offset_ms,
            ..event
        }
        .to_line(session, 60)
    }

    fn command(cmd: &'static str, via: &'static str, shortcut: &'static str) -> Event {
        Event::new("command")
            .with("cmd", cmd)
            .with("via", via)
            .with("shortcut", shortcut)
            .with("ok", true)
    }

    fn navigate(pane: i64, to: &str, via: &'static str) -> Event {
        Event::new("navigate")
            .with("pane", pane)
            .with("from", r"C:\".to_owned())
            .with("to", to.to_owned())
            .with("via", via)
    }

    fn fixture() -> Vec<String> {
        let mut lines = vec![
            line(
                3_600_000,
                "a",
                Event::new("session_start").with("version", "0.6.0"),
            ),
            "not json at all".to_owned(),
            "{\"t\":1,\"no-kind\":true}".to_owned(),
            String::new(),
            line(
                40 * 86_400_000,
                "old",
                command("/copy", "shortcut", "Ctrl Shift C"),
            ),
        ];
        for index in 0..4 {
            lines.push(line(
                3_000_000 - index,
                "a",
                command("/copy", "button", "Ctrl Shift C"),
            ));
        }
        lines.push(line(
            2_900_000,
            "a",
            command("/copy", "shortcut", "Ctrl Shift C"),
        ));
        lines.push(line(
            2_800_000,
            "a",
            command("/sort", "palette", "Ctrl Shift S"),
        ));
        lines.push(line(
            2_700_000,
            "a",
            Event::new("command")
                .with("cmd", "/delete")
                .with("via", "context_menu")
                .with("shortcut", "Ctrl Shift D")
                .with("ok", false),
        ));
        for _ in 0..3 {
            lines.push(line(
                2_600_000,
                "a",
                Event::new("key_unhandled")
                    .with("key", "F5")
                    .with("focus", "pane"),
            ));
        }
        lines.push(line(
            2_500_000,
            "a",
            Event::new("ask")
                .with("text", "open settings for | pipes".to_owned())
                .with("outcome", "no_match")
                .with("candidates", 0),
        ));
        lines.push(line(
            2_400_000,
            "a",
            Event::new("ask")
                .with("text", "/copy".to_owned())
                .with("outcome", "command:/copy"),
        ));
        lines.push(line(
            2_300_000,
            "a",
            Event::new("dialog")
                .with("name", "review_copy")
                .with("outcome", "cancelled")
                .with("open_ms", 1500),
        ));
        lines.push(line(
            2_200_000,
            "a",
            Event::new("error").with("message", "Select or focus an item first.".to_owned()),
        ));
        lines.push(line(
            2_100_000,
            "a",
            Event::new("slow_folder")
                .with("path", r"\\server\share".to_owned())
                .with("entries", 5000)
                .with("ms", 2400),
        ));
        // Two chains of three steps ending in the same folder, and one ending through a place.
        let deep = r"D:\Projects\_deep\target";
        for (base, session) in [(2_000_000u64, "a"), (1_000_000, "b")] {
            lines.push(line(base, session, navigate(0, r"D:\Projects", "enter")));
            lines.push(line(
                base - 2_000,
                session,
                navigate(0, r"D:\Projects\_deep", "enter"),
            ));
            lines.push(line(base - 4_000, session, navigate(0, deep, "enter")));
        }
        lines.push(line(500_000, "a", navigate(1, r"E:\a", "breadcrumb")));
        lines.push(line(499_000, "a", navigate(1, r"E:\a\b", "enter")));
        lines.push(line(498_000, "a", navigate(1, r"E:\a\b\c", "place")));
        lines.push(line(400_000, "a", navigate(1, r"E:\x", "enter")));
        lines.push(line(
            300_000,
            "a",
            Event::new("summary").with(
                "counts",
                Value::Counts(vec![("key.Down".into(), 40), ("typed.filter".into(), 12)]),
            ),
        ));
        lines.push(line(
            200_000,
            "a",
            Event::new("operation")
                .with("op", "copy")
                .with("items", 3)
                .with("duration_ms", 900)
                .with("policy", "Skip")
                .with("error", None::<String>),
        ));
        lines.push(line(
            100_000,
            "a",
            Event::new("session_end")
                .with("duration_ms", 3_500_000u64)
                .with("active_ms", 3_000_000u64)
                .with("idle_ms", 500_000u64)
                .with("dropped", 0),
        ));
        lines
    }

    #[test]
    fn aggregation_tolerates_malformed_lines_and_counts_routes() {
        let lines = fixture();
        let report = Report::from_lines(lines.iter().map(String::as_str), 30, NOW);
        assert_eq!(report.malformed, 2);
        assert_eq!(
            report.sessions.len(),
            2,
            "the 40-day-old session is outside the period"
        );
        assert_eq!((report.starts, report.ends), (1, 1));
        assert_eq!(report.active_ms, 3_000_000);
        let copy = &report.commands["/copy"];
        assert_eq!(copy.total, 5);
        assert_eq!(copy.count(InputClass::Mouse), 4);
        assert_eq!(copy.count(InputClass::Keyboard), 1);
        assert_eq!(report.commands["/sort"].count(InputClass::Ask), 1);
        assert_eq!(report.commands["/delete"].failed, 1);
        let opportunities = report.shortcut_opportunities();
        assert_eq!(opportunities.len(), 1);
        assert_eq!(opportunities[0].0, "/copy");
        assert_eq!(report.unhandled[&("F5".into(), "pane".into())], 3);
        assert_eq!(report.unmet_ask.len(), 1);
        assert_eq!(report.ask["command"], 1);
        assert_eq!(report.ask["no_match"], 1);
        assert_eq!(report.dialogs["review_copy"].outcomes["cancelled"], 1);
        assert_eq!(report.counters["key.Down"], 40);
        assert_eq!(report.operations["copy"].items, 3);
        assert_eq!(
            report.candidates,
            vec![Candidate {
                folder: r"D:\Projects\_deep\target".into(),
                chains: 2,
                steps: 6,
                direct: 0,
            }],
            "a chain ending through a place is not a candidate"
        );
        assert_eq!(report.folders[r"E:\x"].visits, 1);
    }

    #[test]
    fn markdown_contains_every_section_and_escapes_cells() {
        let lines = fixture();
        let report = Report::from_lines(lines.iter().map(String::as_str), 30, NOW);
        let markdown =
            report.markdown(Path::new(r"C:\Users\me\AppData\Local\TomasCommander\usage"));
        for heading in [
            "# Tomas Commander usage report",
            "## How to read this",
            "## Totals",
            "## Commands and how they were invoked",
            "### Shortcut opportunities",
            "## Unhandled keys",
            "## Ask (Ctrl+Space)",
            "### Unmet intent",
            "## Dialogs",
            "## Errors shown to the user",
            "## Slow work",
            "### Slow folders",
            "## Navigation",
            "### Most visited folders",
            "### Places candidates",
            "### Tabs and history",
            "### Filters",
            "## Recursive filename search",
            "## AI tasks",
            "### Operations",
            "## Input summary",
            "## Raw data",
        ] {
            assert!(markdown.contains(heading), "{heading} missing:\n{markdown}");
        }
        assert!(
            markdown.contains("| `/copy` | 5 | 20 % | 80 % | 0 % | Ctrl Shift C |"),
            "{markdown}"
        );
        assert!(markdown.contains(r"`D:\Projects\_deep\target`"));
        assert!(markdown.contains(r"`open settings for \| pipes`"));
        assert!(markdown.contains(r"`\\server\share`"));
        assert!(markdown.contains(r"`C:\Users\me\AppData\Local\TomasCommander\usage`"));
        let empty = Report::from_lines([], 7, NOW).markdown(Path::new("usage"));
        assert!(empty.contains("_No data._"));
        assert!(empty.contains("last 7 days"));
    }

    #[test]
    fn loading_reads_only_day_files_in_the_period() {
        let dir = std::env::temp_dir().join(format!(
            "tc-usage-report-{}-{}",
            std::process::id(),
            crate::usage::now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(
            Report::load(&dir, 30, NOW).unwrap().lines,
            0,
            "a missing folder is empty"
        );
        fs::create_dir_all(&dir).unwrap();
        let today = local_day(NOW, local_offset_minutes(NOW));
        fs::write(dir.join(day_file_name(today)), fixture().join("\n")).unwrap();
        fs::write(
            dir.join(day_file_name(today - 60)),
            line(60 * 86_400_000, "x", command("/old", "button", "")),
        )
        .unwrap();
        fs::write(dir.join("notes.jsonl"), "{\"t\":1,\"k\":\"command\"}").unwrap();
        let report = Report::load(&dir, 30, NOW).unwrap();
        assert_eq!(report.files, 1);
        assert!(!report.commands.contains_key("/old"));
        assert_eq!(report.commands["/copy"].total, 5);
        let path = export(&dir, &report.markdown(&dir), NOW).unwrap();
        assert!(crate::usage::is_report_file(
            &path.file_name().unwrap().to_string_lossy()
        ));
        assert!(fs::read_to_string(&path).unwrap().contains("## Raw data"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cli_arguments_parse_strictly() {
        let parse = |args: &[&str]| CliArgs::parse(args.iter().map(OsString::from));
        assert_eq!(
            parse(&[]).unwrap(),
            CliArgs {
                days: 30,
                settings: None,
                out: None
            }
        );
        assert_eq!(
            parse(&["--days", "7", "--settings", r"C:\s\p.txt", "--out", "r.md"]).unwrap(),
            CliArgs {
                days: 7,
                settings: Some(PathBuf::from(r"C:\s\p.txt")),
                out: Some(PathBuf::from("r.md"))
            }
        );
        for bad in [
            &["--days"][..],
            &["--days", "0"],
            &["--days", "366"],
            &["--days", "x"],
            &["--out"],
            &["--settings"],
            &["--root", "C:\\"],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn durations_and_percentages_read_naturally() {
        assert_eq!(duration(850), "850 ms");
        assert_eq!(duration(12_400), "12 s");
        assert_eq!(duration(245_000), "4 min 05 s");
        assert_eq!(duration(11_220_000), "3 h 07 min");
        assert_eq!(percent(1, 3), "33 %");
        assert_eq!(percent(0, 0), "-");
    }
}
