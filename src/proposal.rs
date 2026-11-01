//! AI change proposals. The read-only file agent may end a run with one `propose_changes`
//! call. This module validates that call outside the model's control (real-file IDs, current
//! versions, approved roots, Windows names, collisions, ordering) and later runs only the steps
//! the user selected and approved, one at a time, through the ordinary planners and journal.
//! Model output never approves or executes anything.

use crate::{
    changes,
    file_tools::{self, Observed, Settings, Tools},
    files::{self, ConflictPolicy, FileResult, Operation, Outcome, Scope, display_path},
    journal::Journal,
    naming,
    undo::same_path,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// Operations one proposal may contain.
pub const MAX_OPERATIONS: usize = 500;
/// Bytes of `propose_changes` arguments accepted from the provider.
pub const MAX_ARGUMENTS: usize = 256 * 1024;
/// Problems reported back for one rejected proposal.
const MAX_PROBLEMS: usize = 20;
const MAX_SUMMARY: usize = 4096;
const MAX_REASON: usize = 1024;

/// The `propose_changes` function offered to the model. It is not strict so that one item
/// shape can carry every operation; the validator below is the actual contract.
pub fn tool_definition() -> Value {
    json!({"type":"function","name":"propose_changes",
        "description":"Final step for requests to organize, rename, move, copy or delete files: propose an ordered list of at most 500 changes. Nothing runs until the user reviews, selects and approves them. Sources are numbered real-file IDs from earlier tool results in this run, never paths. A parent/destination is an existing folder ID, an existing folder path (absolute inside an approved root, or relative to one) or the ref of an earlier new_folder in this proposal: the n-th new_folder is \"fn\" (f1, f2, ...). Existing targets are never overwritten. Use recycle (Recycle Bin) only when the user explicitly asked to delete.",
        "parameters":{"type":"object","properties":{
            "summary":{"type":"string","description":"Short summary of the proposal in the user's language"},
            "operations":{"type":"array","maxItems":MAX_OPERATIONS,"items":{"type":"object","properties":{
                "op":{"type":"string","enum":["new_folder","move","copy","rename","recycle"]},
                "ref":{"type":["string","null"],"description":"new_folder only: its ref, fn for the n-th new_folder"},
                "parent":{"type":["integer","string","null"],"description":"new_folder: existing folder ID, existing folder path or earlier new_folder ref"},
                "source":{"type":["integer","null"],"description":"move/copy/rename/recycle: real-file ID from tool results"},
                "destination":{"type":["integer","string","null"],"description":"move/copy: existing folder ID, existing folder path or earlier new_folder ref"},
                "name":{"type":["string","null"],"description":"new_folder/rename: one valid Windows file name, not a path"},
                "reason":{"type":"string","description":"Short evidence-based reason in the user's language"}},
                "required":["op","reason"],"additionalProperties":false}}},
            "required":["summary","operations"],"additionalProperties":false},
        "strict":false})
}

/// What one proposed step does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    NewFolder,
    Move,
    Copy,
    Rename,
    Recycle,
}

impl Kind {
    /// Review order of the grouped sections.
    pub const ALL: [Self; 5] = [
        Self::NewFolder,
        Self::Move,
        Self::Copy,
        Self::Rename,
        Self::Recycle,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::NewFolder => "new_folder",
            Self::Move => "move",
            Self::Copy => "copy",
            Self::Rename => "rename",
            Self::Recycle => "recycle",
        }
    }

    fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::NewFolder => "New folder",
            Self::Move => "Move",
            Self::Copy => "Copy",
            Self::Rename => "Rename",
            Self::Recycle => "Delete (Recycle Bin)",
        }
    }

    /// Heading of the review section.
    pub fn section(self) -> &'static str {
        match self {
            Self::NewFolder => "New folders",
            Self::Move => "Moves",
            Self::Copy => "Copies",
            Self::Rename => "Renames",
            Self::Recycle => "Deletes (Recycle Bin)",
        }
    }
}

/// The folder receiving a step's result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Place {
    /// An existing folder, bound to its observed identity.
    Existing(Observed),
    /// The folder created by the `new_folder` step with this index.
    New(usize),
}

/// One validated step. Paths are bound to observed identities; the step is planned again
/// against the current state just before it runs.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub kind: Kind,
    /// The item a move, copy, rename or delete changes.
    pub source: Option<Observed>,
    /// Where a new folder, move or copy puts its result.
    pub place: Option<Place>,
    /// New name of a new folder or rename.
    pub name: Option<String>,
    pub reason: String,
    /// Result path when proposed (the source itself for a delete).
    pub target: PathBuf,
}

impl Step {
    /// The new folder this step needs, by step index.
    pub fn dependency(&self) -> Option<usize> {
        match &self.place {
            Some(Place::New(index)) => Some(*index),
            _ => None,
        }
    }

    /// The changed item, if any.
    pub fn before(&self) -> Option<&Path> {
        self.source.as_ref().map(|source| source.path.as_path())
    }

    /// The new path, if the step creates or renames something.
    pub fn after(&self) -> Option<&Path> {
        (self.kind != Kind::Recycle).then_some(self.target.as_path())
    }

    /// "Move: D:\a.pdf → D:\PDF\a.pdf".
    pub fn label(&self) -> String {
        let source = self.before().map(display_path).unwrap_or_default();
        let target = display_path(&self.target);
        match self.kind {
            Kind::NewFolder => format!("New folder: {target}"),
            Kind::Recycle => format!("Delete (Recycle Bin): {source}"),
            kind => format!("{}: {source} → {target}", kind.label()),
        }
    }

    fn place_record(&self) -> FileResult<&Place> {
        self.place
            .as_ref()
            .ok_or_else(|| "Malformed step: missing folder.".into())
    }

    fn name_text(&self) -> FileResult<&str> {
        self.name
            .as_deref()
            .ok_or_else(|| "Malformed step: missing name.".into())
    }
}

/// A validated proposal awaiting review.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub summary: String,
    pub steps: Vec<Step>,
}

impl Proposal {
    pub fn count(&self, kind: Kind) -> usize {
        self.steps.iter().filter(|step| step.kind == kind).count()
    }

    /// Steps that need the new folder of step `index`, directly.
    pub fn dependents(&self, index: usize) -> Vec<usize> {
        self.steps
            .iter()
            .enumerate()
            .filter(|(_, step)| step.dependency() == Some(index))
            .map(|(dependent, _)| dependent)
            .collect()
    }
}

fn fold(path: &Path) -> String {
    display_path(path).to_lowercase()
}

/// Whether `child` is `parent` or lies inside it, ignoring case.
fn within(child: &Path, parent: &Path) -> bool {
    Path::new(&fold(child)).starts_with(Path::new(&fold(parent)))
}

/// Items a folder source may contain; larger trees are not proposed as one change.
const MAX_TREE: usize = 100_000;

/// Refuses a folder that holds credential/configuration paths the harness never reads, so a
/// change never carries them along. Links are not followed.
fn guard_tree(path: &Path) -> Result<(), String> {
    let mut pending = vec![path.to_path_buf()];
    let mut seen = 0usize;
    while let Some(folder) = pending.pop() {
        let entries =
            fs::read_dir(&folder).map_err(|error| format!("{}: {error}", display_path(&folder)))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("{}: {error}", display_path(&folder)))?;
            seen += 1;
            if seen > MAX_TREE {
                return Err(format!(
                    "{} holds more than {MAX_TREE} items; propose smaller changes",
                    display_path(path)
                ));
            }
            let child = entry.path();
            if file_tools::protected(&child) {
                return Err(format!(
                    "{} contains a protected credential/configuration path: {}",
                    display_path(path),
                    display_path(&child)
                ));
            }
            let metadata = fs::symlink_metadata(&child)
                .map_err(|error| format!("{}: {error}", display_path(&child)))?;
            if metadata.is_dir() && !files::reparse(&metadata) {
                pending.push(child);
            }
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    summary: String,
    operations: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    op: String,
    #[serde(default, rename = "ref")]
    reference: Option<String>,
    #[serde(default)]
    parent: Option<Value>,
    #[serde(default)]
    source: Option<Value>,
    #[serde(default)]
    destination: Option<Value>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

/// A local new-folder reference such as `f3`.
fn is_reference(text: &str) -> bool {
    text.strip_prefix('f').is_some_and(|number| {
        !number.is_empty() && !number.starts_with('0') && number.bytes().all(|b| b.is_ascii_digit())
    })
}

/// Validates `propose_changes` arguments against the real-file IDs and approved roots of this
/// run. Every problem is reported (up to a bound) so the model can fix them in one step.
pub fn validate(arguments: &str, tools: &mut Tools) -> Result<Proposal, Vec<String>> {
    if arguments.len() > MAX_ARGUMENTS {
        return Err(vec![format!(
            "The proposal exceeds {} KiB; propose fewer changes.",
            MAX_ARGUMENTS / 1024
        )]);
    }
    let arguments: Arguments = serde_json::from_str(arguments)
        .map_err(|error| vec![format!("Malformed propose_changes arguments: {error}")])?;
    let count = arguments.operations.len();
    if count == 0 {
        return Err(vec![
            "The proposal has no operations; answer with the final JSON instead.".into(),
        ]);
    }
    if count > MAX_OPERATIONS {
        return Err(vec![format!(
            "{count} operations exceed the limit of {MAX_OPERATIONS}; propose a smaller batch."
        )]);
    }
    let mut problems = Vec::new();
    let summary = arguments.summary.trim().to_owned();
    if summary.is_empty() || summary.chars().count() > MAX_SUMMARY {
        problems.push(format!("The summary must have 1-{MAX_SUMMARY} characters."));
    }
    // Refs are positional: the n-th new_folder is "fn".
    let mut references = BTreeMap::new();
    for (index, value) in arguments.operations.iter().enumerate() {
        if value.get("op").and_then(Value::as_str) == Some("new_folder") {
            references.insert(format!("f{}", references.len() + 1), index);
        }
    }
    let mut steps: Vec<Option<Step>> = Vec::with_capacity(count);
    for (index, value) in arguments.operations.into_iter().enumerate() {
        let result = serde_json::from_value::<Raw>(value)
            .map_err(|error| format!("malformed operation: {error}"))
            .and_then(|raw| operation(index, raw, &references, &steps, tools));
        match result {
            Ok(step) => steps.push(Some(step)),
            Err(error) => {
                problems.push(format!("Operation {}: {error}", index + 1));
                steps.push(None);
            }
        }
    }
    cross_check(&steps, &mut problems);
    if problems.is_empty() {
        return Ok(Proposal {
            summary,
            steps: steps.into_iter().flatten().collect(),
        });
    }
    if problems.len() > MAX_PROBLEMS {
        let more = problems.len() - MAX_PROBLEMS;
        problems.truncate(MAX_PROBLEMS);
        problems.push(format!("...and {more} more problems."));
    }
    Err(problems)
}

fn operation(
    index: usize,
    raw: Raw,
    references: &BTreeMap<String, usize>,
    steps: &[Option<Step>],
    tools: &mut Tools,
) -> Result<Step, String> {
    let kind = Kind::parse(&raw.op).ok_or_else(|| {
        format!(
            "unknown op \"{}\"; use new_folder, move, copy, rename or recycle",
            raw.op
        )
    })?;
    let reason = raw.reason.as_deref().map(str::trim).unwrap_or("");
    if reason.is_empty() || reason.chars().count() > MAX_REASON {
        return Err(format!("give a short reason (1-{MAX_REASON} characters)"));
    }
    let (required, allowed): (&[&str], &[&str]) = match kind {
        Kind::NewFolder => (&["parent", "name"], &["ref", "parent", "name"]),
        Kind::Move | Kind::Copy => (&["source", "destination"], &["source", "destination"]),
        Kind::Rename => (&["source", "name"], &["source", "name"]),
        Kind::Recycle => (&["source"], &["source"]),
    };
    for (field, present) in [
        ("ref", raw.reference.is_some()),
        ("parent", raw.parent.is_some()),
        ("source", raw.source.is_some()),
        ("destination", raw.destination.is_some()),
        ("name", raw.name.is_some()),
    ] {
        if present && !allowed.contains(&field) {
            return Err(format!("{} does not take \"{field}\"", kind.key()));
        }
        if !present && required.contains(&field) {
            return Err(format!("{} requires \"{field}\"", kind.key()));
        }
    }
    if let Some(given) = &raw.reference {
        let expected = references
            .iter()
            .find(|(_, position)| **position == index)
            .map(|(reference, _)| reference.as_str())
            .unwrap_or("");
        if given != expected {
            return Err(format!(
                "this new_folder's ref is \"{expected}\" (the n-th new_folder is fn), not \"{given}\""
            ));
        }
    }
    let source = match &raw.source {
        Some(value) => Some(source(value, tools)?),
        None => None,
    };
    let place = match kind {
        Kind::NewFolder => raw.parent.as_ref(),
        Kind::Move | Kind::Copy => raw.destination.as_ref(),
        _ => None,
    }
    .map(|value| place(index, value, references, steps, tools))
    .transpose()?;
    if let Some(name) = &raw.name {
        naming::validate_name(name).map_err(|error| format!("name \"{name}\": {error}"))?;
    }
    let folder = match &place {
        Some(Place::Existing(folder)) => Some(folder.path.clone()),
        Some(Place::New(position)) => Some(
            steps
                .get(*position)
                .and_then(Option::as_ref)
                .map(|step| step.target.clone())
                .ok_or_else(|| {
                    format!("its folder comes from invalid operation {}", position + 1)
                })?,
        ),
        None => None,
    };
    let target = match (kind, &source, &folder, &raw.name) {
        (Kind::NewFolder, _, Some(folder), Some(name)) => folder.join(name),
        (Kind::Move | Kind::Copy, Some(source), Some(folder), _) => {
            folder.join(source.path.file_name().ok_or("the source has no name")?)
        }
        (Kind::Rename, Some(source), _, Some(name)) => source
            .path
            .parent()
            .ok_or("the source has no parent folder")?
            .join(name),
        (Kind::Recycle, Some(source), _, _) => source.path.clone(),
        _ => return Err("incomplete operation".into()),
    };
    if let (Some(source), Some(folder)) = (&source, &folder)
        && within(folder, &source.path)
    {
        return Err("a folder cannot be moved or copied into itself or its subfolder".into());
    }
    if kind != Kind::Recycle {
        if file_tools::protected(&target) {
            return Err(format!(
                "{} is a protected credential/configuration path",
                display_path(&target)
            ));
        }
        if let Some(source) = &source
            && same_path(&source.path, &target)
        {
            return Err(match kind {
                Kind::Rename if source.path.file_name() == target.file_name() => {
                    "the name is unchanged".into()
                }
                Kind::Rename => "case-only renames are not supported".into(),
                _ => "the item is already in that folder".into(),
            });
        }
        if fs::symlink_metadata(&target).is_ok() {
            return Err(format!(
                "{} already exists; nothing is ever overwritten",
                display_path(&target)
            ));
        }
    }
    Ok(Step {
        kind,
        source,
        place,
        name: raw.name,
        reason: reason.to_owned(),
        target,
    })
}

/// The observed record of real-file ID `id`; `exact` also requires its observed version.
fn record(id: u64, tools: &mut Tools, exact: bool) -> Result<Observed, String> {
    let original = usize::try_from(id)
        .ok()
        .and_then(|id| id.checked_sub(1))
        .and_then(|index| tools.observed.get(index))
        .cloned()
        .ok_or_else(|| {
            format!("unknown file ID {id}; use only IDs returned by tools in this run")
        })?;
    let observed = tools
        .observe(&original.path, "proposal")
        .map_err(|error| format!("file ID {id}: {error}"))?;
    let now = observed["id"].as_u64().unwrap_or(0);
    let latest = usize::try_from(now)
        .ok()
        .and_then(|now| now.checked_sub(1))
        .and_then(|index| tools.observed.get(index))
        .cloned()
        .ok_or("observation failed")?;
    if exact && now != id {
        return Err(format!(
            "file ID {id} changed since it was observed (now ID {now}); inspect it again"
        ));
    }
    if latest.identity != original.identity
        || latest.directory != original.directory
        || latest.root != original.root
    {
        return Err(format!("ID {id} was replaced since it was observed"));
    }
    Ok(latest)
}

fn source(value: &Value, tools: &mut Tools) -> Result<Observed, String> {
    let id = match value {
        Value::String(_) => {
            return Err("sources must be real-file IDs from tool results, not paths".into());
        }
        value => value.as_u64().ok_or("source must be a real-file ID")?,
    };
    let source = record(id, tools, true)?;
    if same_path(&source.path, &source.root) {
        return Err("an approved root itself cannot be changed".into());
    }
    if source.directory {
        guard_tree(&source.path)?;
    }
    Ok(source)
}

fn place(
    index: usize,
    value: &Value,
    references: &BTreeMap<String, usize>,
    steps: &[Option<Step>],
    tools: &mut Tools,
) -> Result<Place, String> {
    match value {
        Value::String(text) if is_reference(text.trim()) => {
            let text = text.trim();
            match references.get(text) {
                Some(&position) if position < index => {
                    if steps.get(position).is_none_or(Option::is_none) {
                        return Err(format!(
                            "{text} comes from invalid operation {}",
                            position + 1
                        ));
                    }
                    Ok(Place::New(position))
                }
                Some(&position) => Err(format!(
                    "{text} is used before its new_folder (operation {})",
                    position + 1
                )),
                None => Err(format!(
                    "unknown ref {text}; refs name earlier new_folder operations (f1, f2, ...)"
                )),
            }
        }
        Value::String(text) => Ok(Place::Existing(folder_path(text, tools)?)),
        value => {
            let id = value
                .as_u64()
                .ok_or("a folder must be an ID, a folder path or a new_folder ref")?;
            let folder = record(id, tools, false)?;
            if !folder.directory {
                return Err(format!("ID {id} is a file, not a folder"));
            }
            Ok(Place::Existing(folder))
        }
    }
}

/// An existing folder given by an absolute path inside an approved root or a path relative to
/// exactly one approved root.
fn folder_path(text: &str, tools: &mut Tools) -> Result<Observed, String> {
    let text = text.trim().replace('/', "\\");
    let path = PathBuf::from(&text);
    let candidate = if path.is_absolute() {
        path
    } else {
        if text.is_empty()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(format!(
                "folder \"{text}\" must be absolute inside an approved root or relative to one, without .. or a drive"
            ));
        }
        let found: Vec<PathBuf> = tools
            .settings()
            .roots
            .iter()
            .map(|root| root.path.join(&path))
            .filter(|candidate| candidate.is_dir())
            .collect();
        match found.as_slice() {
            [] => {
                return Err(format!(
                    "folder \"{text}\" was not found in the approved roots"
                ));
            }
            [single] => single.clone(),
            _ => {
                return Err(format!(
                    "folder \"{text}\" exists in several approved roots; use its ID or absolute path"
                ));
            }
        }
    };
    let observed = tools
        .observe(&candidate, "proposal folder")
        .map_err(|error| format!("folder \"{text}\": {error}"))?;
    let folder = observed["id"]
        .as_u64()
        .and_then(|id| usize::try_from(id).ok())
        .and_then(|id| id.checked_sub(1))
        .and_then(|index| tools.observed.get(index))
        .cloned()
        .ok_or("observation failed")?;
    if !folder.directory {
        return Err(format!("\"{text}\" is a file, not a folder"));
    }
    Ok(folder)
}

/// Rules across operations: one change per item, no item inside another changed item, no
/// folder that another operation changes, and unique targets.
fn cross_check(steps: &[Option<Step>], problems: &mut Vec<String>) {
    let valid: Vec<(usize, &Step)> = steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| step.as_ref().map(|step| (index, step)))
        .collect();
    for &(index, step) in &valid {
        for &(other, earlier) in &valid {
            let Some(changed) = &earlier.source else {
                continue;
            };
            if other == index {
                continue;
            }
            if let Some(source) = &step.source {
                if same_path(&source.path, &changed.path) {
                    if other < index {
                        problems.push(format!(
                            "Operation {}: the same item is already changed by operation {}",
                            index + 1,
                            other + 1
                        ));
                    }
                } else if within(&source.path, &changed.path) {
                    problems.push(format!(
                        "Operation {}: changes an item inside the source of operation {}",
                        index + 1,
                        other + 1
                    ));
                }
            }
            if let Some(Place::Existing(folder)) = &step.place
                && within(&folder.path, &changed.path)
            {
                problems.push(format!(
                    "Operation {}: its folder is changed by operation {}",
                    index + 1,
                    other + 1
                ));
            }
        }
    }
    let mut targets = BTreeMap::new();
    for &(index, step) in &valid {
        if step.kind == Kind::Recycle {
            continue;
        }
        let key = fold(&step.target);
        if let Some(other) = targets.get(&key) {
            problems.push(format!(
                "Operation {}: {} is also the target of operation {}",
                index + 1,
                display_path(&step.target),
                other + 1
            ));
        } else {
            targets.insert(key, index);
        }
    }
}

/// The steps a user selected and approved, in proposal order. `Place::New` indexes refer to
/// positions in `steps`.
#[derive(Clone)]
pub struct Batch {
    /// Shared `batch=` key of every journal record the batch writes.
    pub key: String,
    /// Plan id of the first step; later steps count up from it.
    pub first_id: u64,
    pub steps: Vec<Step>,
    pub settings: Settings,
    /// Where `settings` are persisted; a change there stops the batch.
    pub settings_path: PathBuf,
}

impl std::fmt::Debug for Batch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Batch")
            .field("key", &self.key)
            .field("first_id", &self.first_id)
            .field("steps", &self.steps)
            .field("settings_path", &self.settings_path)
            .finish_non_exhaustive()
    }
}

impl Batch {
    /// Selected steps in proposal order. A step whose new folder is not selected is refused.
    pub fn new(
        proposal: &Proposal,
        selected: &[bool],
        first_id: u64,
        settings: Settings,
        settings_path: PathBuf,
    ) -> FileResult<Self> {
        if selected.len() != proposal.steps.len() {
            return Err("The selection does not match the proposal.".into());
        }
        let mut positions = vec![None; proposal.steps.len()];
        let mut steps = Vec::new();
        for (index, step) in proposal.steps.iter().enumerate() {
            if !selected[index] {
                continue;
            }
            let mut step = step.clone();
            if let Some(Place::New(folder)) = &mut step.place {
                *folder = positions[*folder].ok_or_else(|| {
                    format!(
                        "Change {} needs a new folder that is not selected.",
                        index + 1
                    )
                })?;
            }
            positions[index] = Some(steps.len());
            steps.push(step);
        }
        if steps.is_empty() {
            return Err("Select at least one change.".into());
        }
        Ok(Self {
            key: format!("ai-{}", changes::identity()?),
            first_id,
            steps,
            settings,
            settings_path,
        })
    }
}

fn settings_unchanged(batch: &Batch) -> FileResult<()> {
    if Settings::load(&batch.settings_path)? != batch.settings {
        return Err(
            "AI access settings changed since the proposal; ask again. Nothing further was changed."
                .into(),
        );
    }
    Ok(())
}

/// The current path of an observed item inside the approved roots: the same item, and with
/// `exact` also its observed size and modification time. Folders that receive results are
/// compared by identity only, because earlier steps legitimately change their contents.
fn current(settings: &Settings, record: &Observed, exact: bool) -> FileResult<PathBuf> {
    let mut tools = Tools::new(settings.clone())?;
    tools
        .observe(&record.path, "proposal step")
        .map_err(|error| format!("{}: {error}", display_path(&record.path)))?;
    let actual = tools.observed.first().ok_or("Item disappeared.")?;
    if actual.identity != record.identity
        || actual.directory != record.directory
        || actual.root != record.root
        || (exact && (actual.size != record.size || actual.modified != record.modified))
    {
        return Err(format!(
            "Changed since the proposal: {}",
            display_path(&record.path)
        ));
    }
    Ok(actual.path.clone())
}

/// The current path of a step's source at its proposed version; a folder is checked again for
/// protected contents.
fn current_source(settings: &Settings, record: &Observed) -> FileResult<PathBuf> {
    let path = current(settings, record, true)?;
    if record.directory {
        guard_tree(&path)?;
    }
    Ok(path)
}

fn folder_of(step: &Step, batch: &Batch, results: &[Option<PathBuf>]) -> FileResult<PathBuf> {
    match step.place_record()? {
        Place::Existing(folder) => current(&batch.settings, folder, false),
        Place::New(index) => results
            .get(*index)
            .cloned()
            .flatten()
            .ok_or_else(|| format!("Its new folder (step {}) is not available.", index + 1)),
    }
}

/// Plans one step without effects and returns its result path.
fn check(
    scope: &Scope,
    batch: &Batch,
    step: &Step,
    results: &[Option<PathBuf>],
    cancel: &AtomicBool,
) -> FileResult<PathBuf> {
    let source = match &step.source {
        Some(record) => Some(current_source(&batch.settings, record)?),
        None => None,
    };
    let source_tree = |source: &Path| {
        // Recycle planning walks and checks the source tree only.
        files::plan_with_policy(
            scope,
            0,
            Operation::Recycle,
            &[source.to_path_buf()],
            None,
            ConflictPolicy::Fail,
            cancel,
        )
        .map(|_| ())
    };
    match (step.kind, &source) {
        (Kind::NewFolder, _) => {
            let name = step.name_text()?;
            match step.place_record()? {
                Place::Existing(_) => {
                    let parent = folder_of(step, batch, results)?;
                    Ok(naming::Plan::prepare(scope, 0, &parent, None, name)?
                        .target()
                        .to_path_buf())
                }
                Place::New(_) => {
                    naming::validate_name(name)?;
                    Ok(folder_of(step, batch, results)?.join(name))
                }
            }
        }
        (Kind::Move | Kind::Copy, Some(source)) => {
            let name = source.file_name().ok_or("The source has no name.")?;
            let destination = folder_of(step, batch, results)?;
            if within(&destination, source) {
                return Err("A folder cannot be moved or copied into itself.".into());
            }
            match step.place_record()? {
                Place::Existing(_) => {
                    let operation = if step.kind == Kind::Move {
                        Operation::Move
                    } else {
                        Operation::Copy
                    };
                    let plan = files::plan_with_policy(
                        scope,
                        0,
                        operation,
                        std::slice::from_ref(source),
                        Some(&destination),
                        ConflictPolicy::Fail,
                        cancel,
                    )?;
                    Ok(plan.destination().unwrap_or(&destination).join(name))
                }
                Place::New(_) => {
                    source_tree(source)?;
                    Ok(destination.join(name))
                }
            }
        }
        (Kind::Rename, Some(source)) => {
            let parent = source.parent().ok_or("The source has no parent folder.")?;
            Ok(
                naming::Plan::prepare(scope, 0, parent, Some(source), step.name_text()?)?
                    .target()
                    .to_path_buf(),
            )
        }
        (Kind::Recycle, Some(source)) => {
            source_tree(source)?;
            Ok(source.clone())
        }
        _ => Err("Malformed step: missing source.".into()),
    }
}

/// Plans every step in order without effects, simulating the folders that earlier steps
/// create. Returns every problem found; nothing has changed either way.
pub fn dry_run(scope: &Scope, batch: &Batch, cancel: &AtomicBool) -> Result<(), Vec<String>> {
    settings_unchanged(batch).map_err(|error| vec![error])?;
    let mut problems = Vec::new();
    let mut results: Vec<Option<PathBuf>> = Vec::with_capacity(batch.steps.len());
    let mut targets: BTreeMap<String, usize> = BTreeMap::new();
    for (index, step) in batch.steps.iter().enumerate() {
        if cancel.load(Ordering::Acquire) {
            return Err(vec!["Check cancelled; nothing was changed.".into()]);
        }
        let result = check(scope, batch, step, &results, cancel).and_then(|target| {
            if step.kind != Kind::Recycle {
                let key = fold(&target);
                if let Some(other) = targets.get(&key) {
                    return Err(format!(
                        "{} is also the target of step {}.",
                        display_path(&target),
                        other + 1
                    ));
                }
                targets.insert(key, index);
            }
            Ok(target)
        });
        match result {
            Ok(target) => results.push(Some(target)),
            Err(error) => {
                problems.push(format!("Step {} ({}): {error}", index + 1, step.label()));
                results.push(None);
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

/// Result of one step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepState {
    Done,
    Failed(String),
    Cancelled,
    NotRun,
}

#[derive(Clone, Debug)]
pub struct StepReport {
    pub label: String,
    pub state: StepState,
    pub created: Vec<PathBuf>,
    pub incomplete: Vec<PathBuf>,
}

/// What a batch did, step by step.
#[derive(Clone, Debug)]
pub struct Report {
    pub key: String,
    pub steps: Vec<StepReport>,
    pub cancelled: bool,
    pub elapsed: Duration,
}

impl Report {
    pub fn done(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| step.state == StepState::Done)
            .count()
    }

    /// Steps that started.
    pub fn ran(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| step.state != StepState::NotRun)
            .count()
    }

    /// Index of the step that failed.
    pub fn failed_at(&self) -> Option<usize> {
        self.steps
            .iter()
            .position(|step| matches!(step.state, StepState::Failed(_)))
    }

    /// "AI changes: 3 of 5 steps done; stopped at step 4 (...): error; 1 not run".
    pub fn summary(&self) -> String {
        let mut text = format!(
            "AI changes: {} of {} steps done",
            self.done(),
            self.steps.len()
        );
        if let Some(index) = self.failed_at()
            && let StepState::Failed(error) = &self.steps[index].state
        {
            text.push_str(&format!(
                "; stopped at step {} ({}): {error}",
                index + 1,
                self.steps[index].label
            ));
        }
        if self.cancelled {
            text.push_str("; cancelled");
        }
        let not_run = self.steps.len() - self.ran();
        if not_run > 0 {
            text.push_str(&format!("; {not_run} not run"));
        }
        text
    }

    /// One line per step, then its error and created or unfinished paths.
    pub fn rows(&self) -> Vec<String> {
        let mut rows = Vec::new();
        for (index, step) in self.steps.iter().enumerate() {
            let state = match &step.state {
                StepState::Done => "DONE",
                StepState::Failed(_) => "FAILED",
                StepState::Cancelled => "CANCELLED",
                StepState::NotRun => "NOT RUN",
            };
            rows.push(format!("{}. {state}: {}", index + 1, step.label));
            if let StepState::Failed(error) = &step.state {
                rows.push(format!("   ERROR: {error}"));
            }
            if step.state != StepState::Done {
                for path in &step.created {
                    rows.push(format!("   CREATED: {}", display_path(path)));
                }
                for path in &step.incomplete {
                    rows.push(format!("   NOT COMPLETED: {}", display_path(path)));
                }
            }
        }
        rows
    }
}

enum Planned {
    Name(naming::Plan),
    Files(files::Plan),
}

#[allow(clippy::too_many_arguments)]
fn run_step(
    scope: &Scope,
    journal: &Journal,
    batch: &Batch,
    step: &Step,
    id: u64,
    results: &[Option<PathBuf>],
    cancel: &AtomicBool,
    hooks: &files::Hooks,
) -> Outcome {
    let planned = (|| -> FileResult<Planned> {
        settings_unchanged(batch)?;
        let source = match &step.source {
            Some(record) => Some(current_source(&batch.settings, record)?),
            None => None,
        };
        Ok(match (step.kind, source) {
            (Kind::NewFolder, _) => {
                let parent = folder_of(step, batch, results)?;
                Planned::Name(naming::Plan::prepare(
                    scope,
                    id,
                    &parent,
                    None,
                    step.name_text()?,
                )?)
            }
            (Kind::Move | Kind::Copy, Some(source)) => {
                let destination = folder_of(step, batch, results)?;
                Planned::Files(files::plan_with_policy(
                    scope,
                    id,
                    if step.kind == Kind::Move {
                        Operation::Move
                    } else {
                        Operation::Copy
                    },
                    &[source],
                    Some(&destination),
                    ConflictPolicy::Fail,
                    cancel,
                )?)
            }
            (Kind::Rename, Some(source)) => {
                let parent = source
                    .parent()
                    .ok_or("The source has no parent folder.")?
                    .to_path_buf();
                Planned::Name(naming::Plan::prepare(
                    scope,
                    id,
                    &parent,
                    Some(&source),
                    step.name_text()?,
                )?)
            }
            (Kind::Recycle, Some(source)) => Planned::Files(files::plan_with_policy(
                scope,
                id,
                Operation::Recycle,
                &[source],
                None,
                ConflictPolicy::Fail,
                cancel,
            )?),
            _ => return Err("Malformed step: missing source.".into()),
        })
    })();
    match planned {
        Ok(Planned::Name(plan)) => plan.execute(scope, journal, true, cancel),
        Ok(Planned::Files(plan)) => journal.execute_with(scope, &plan, true, cancel, hooks),
        Err(error) => Outcome {
            id,
            error: Some(format!(
                "Not started; nothing changed by this step: {error}"
            )),
            incomplete: vec![step.before().unwrap_or(&step.target).to_path_buf()],
            ..Default::default()
        },
    }
}

/// Runs the approved steps in order. Each is planned again just before it runs and journaled
/// like a manual operation with the shared batch key. Stops at the first failure; cancellation
/// is checked before every step. `on_step` receives each finished step's outcome.
pub fn execute(
    scope: &Scope,
    journal: &Journal,
    batch: &Batch,
    cancel: &AtomicBool,
    hooks: &files::Hooks,
    on_step: &mut dyn FnMut(usize, Outcome),
) -> Report {
    let started = Instant::now();
    let mut report = Report {
        key: batch.key.clone(),
        steps: batch
            .steps
            .iter()
            .map(|step| StepReport {
                label: step.label(),
                state: StepState::NotRun,
                created: Vec::new(),
                incomplete: Vec::new(),
            })
            .collect(),
        cancelled: false,
        elapsed: Duration::ZERO,
    };
    let journal = match journal.with_batch(&batch.key) {
        Ok(journal) => journal,
        Err(error) => {
            if let Some(first) = report.steps.first_mut() {
                first.state = StepState::Failed(error);
            }
            return report;
        }
    };
    let mut results: Vec<Option<PathBuf>> = vec![None; batch.steps.len()];
    for (index, step) in batch.steps.iter().enumerate() {
        if cancel.load(Ordering::Acquire) {
            report.cancelled = true;
            break;
        }
        let outcome = run_step(
            scope,
            &journal,
            batch,
            step,
            batch.first_id + index as u64,
            &results,
            cancel,
            hooks,
        );
        let entry = &mut report.steps[index];
        entry.created = outcome.created.clone();
        entry.incomplete = outcome.incomplete.clone();
        entry.state = match (&outcome.error, outcome.cancelled) {
            (Some(error), _) => StepState::Failed(error.clone()),
            (None, true) => StepState::Cancelled,
            (None, false) => StepState::Done,
        };
        if step.kind == Kind::NewFolder && entry.state == StepState::Done {
            results[index] = outcome.created.first().cloned();
        }
        let stop = entry.state != StepState::Done;
        report.cancelled |= outcome.cancelled;
        on_step(index, outcome);
        if stop {
            break;
        }
    }
    report.elapsed = started.elapsed();
    report
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::file_tools::tests::Fixture;

    fn id(tools: &mut Tools, path: &Path) -> u64 {
        tools.observe(path, "test").unwrap()["id"].as_u64().unwrap()
    }

    fn propose(tools: &mut Tools, operations: Value) -> Result<Proposal, Vec<String>> {
        validate(
            &json!({"summary":"Synthetic proposal","operations":operations}).to_string(),
            tools,
        )
    }

    fn rejected(tools: &mut Tools, operations: Value, expected: &str) {
        let problems = propose(tools, operations.clone()).unwrap_err();
        assert!(
            problems.iter().any(|problem| problem.contains(expected)),
            "{expected:?} not in {problems:?} for {operations}"
        );
    }

    struct Files {
        fixture: Fixture,
        downloads: PathBuf,
        docs: PathBuf,
        pdf: PathBuf,
        photo: PathBuf,
        setup: PathBuf,
    }

    fn files() -> Files {
        let fixture = Fixture::new();
        let downloads = fixture.root.join("Downloads");
        let docs = fixture.root.join("Docs");
        fs::create_dir(&downloads).unwrap();
        fs::create_dir(&docs).unwrap();
        let pdf = fixture.write("Downloads\\invoice.pdf", "invoice");
        let photo = fixture.write("Downloads\\photo.jpg", "photo");
        let setup = fixture.write("Downloads\\setup.exe", "setup");
        let canonical = |path: &Path| fs::canonicalize(path).unwrap();
        Files {
            downloads: canonical(&downloads),
            docs: canonical(&docs),
            pdf: canonical(&pdf),
            photo: canonical(&photo),
            setup: canonical(&setup),
            fixture,
        }
    }

    #[test]
    fn valid_proposal_binds_real_ids_refs_and_exact_targets_without_effects() {
        let files = files();
        let mut tools = files.fixture.tools();
        let downloads = id(&mut tools, &files.downloads);
        let docs = id(&mut tools, &files.docs);
        let pdf = id(&mut tools, &files.pdf);
        let photo = id(&mut tools, &files.photo);
        let setup = id(&mut tools, &files.setup);
        let proposal = propose(
            &mut tools,
            json!([
                {"op":"new_folder","ref":"f1","parent":"Downloads","name":"PDF","reason":"Group PDFs"},
                {"op":"new_folder","parent":"f1","name":"2026","reason":"Year"},
                {"op":"move","source":pdf,"destination":"f2","reason":"Invoice PDF"},
                {"op":"copy","source":photo,"destination":docs,"reason":"Keep a copy"},
                {"op":"rename","source":photo,"name":"2026-01-01_photo.jpg","reason":"Date"},
                {"op":"recycle","source":setup,"reason":"Old installer"},
                {"op":"new_folder","parent":files.downloads.display().to_string(),"name":"Images","reason":"Absolute parent"},
                {"op":"new_folder","parent":downloads,"name":"Archives","reason":"Parent by ID"}
            ]),
        );
        let problems = proposal.as_ref().err().cloned().unwrap_or_default();
        // The same photo is copied and renamed: one change per item.
        assert!(
            problems.iter().any(|problem| problem
                .contains("Operation 5: the same item is already changed by operation 4")),
            "{problems:?}"
        );
        let proposal = propose(
            &mut tools,
            json!([
                {"op":"new_folder","ref":"f1","parent":"Downloads","name":"PDF","reason":"Group PDFs"},
                {"op":"new_folder","parent":"f1","name":"2026","reason":"Year"},
                {"op":"move","source":pdf,"destination":"f2","reason":"Invoice PDF"},
                {"op":"rename","source":photo,"name":"2026-01-01_photo.jpg","reason":"Date"},
                {"op":"recycle","source":setup,"reason":"Old installer"},
                {"op":"new_folder","parent":files.downloads.display().to_string(),"name":"Images","reason":"Absolute parent"},
                {"op":"new_folder","parent":downloads,"name":"Archives","reason":"Parent by ID"},
                {"op":"new_folder","parent":docs,"name":"Faktury","reason":"Invoices"}
            ]),
        )
        .unwrap();
        assert_eq!(proposal.steps.len(), 8);
        assert_eq!(proposal.count(Kind::NewFolder), 5);
        assert_eq!(proposal.steps[1].dependency(), Some(0));
        assert_eq!(proposal.steps[2].dependency(), Some(1));
        assert_eq!(proposal.dependents(0), vec![1]);
        let same = |a: &Path, b: &Path| assert!(same_path(a, b), "{a:?} != {b:?}");
        same(&proposal.steps[0].target, &files.downloads.join("PDF"));
        same(
            &proposal.steps[2].target,
            &files.downloads.join("PDF\\2026\\invoice.pdf"),
        );
        same(
            &proposal.steps[3].target,
            &files.downloads.join("2026-01-01_photo.jpg"),
        );
        same(&proposal.steps[4].target, &files.setup);
        assert!(proposal.steps[4].after().is_none());
        same(&proposal.steps[5].target, &files.downloads.join("Images"));
        same(&proposal.steps[6].target, &files.downloads.join("Archives"));
        assert!(proposal.steps[2].label().starts_with("Move: "));
        assert!(proposal.steps[2].label().contains(" → "));
        // Validation never changes anything.
        assert!(files.pdf.exists() && files.photo.exists() && files.setup.exists());
        assert!(!files.downloads.join("PDF").exists());
        let round_trip: Proposal =
            serde_json::from_str(&serde_json::to_string(&proposal).unwrap()).unwrap();
        assert_eq!(round_trip.steps.len(), 8);
    }

    #[test]
    fn every_rejection_rule_is_explained() {
        let files = files();
        fs::create_dir(files.docs.join("Sub")).unwrap();
        fs::write(files.docs.join("invoice.pdf"), "existing").unwrap();
        fs::create_dir(files.fixture.root.join(".ssh")).unwrap();
        fs::create_dir(files.fixture.root.join("Configs")).unwrap();
        files.fixture.write("Configs\\.env", "SYNTHETIC=1");
        let mut tools = files.fixture.tools();
        let configs = id(&mut tools, &files.fixture.root.join("Configs"));
        let docs = id(&mut tools, &files.docs);
        let sub = id(&mut tools, &files.docs.join("Sub"));
        let pdf = id(&mut tools, &files.pdf);
        let photo = id(&mut tools, &files.photo);
        let setup = id(&mut tools, &files.setup);
        let downloads = id(&mut tools, &files.downloads);
        let root = id(&mut tools, &files.fixture.root);
        let reason = "Synthetic";
        for (operations, expected) in [
            (
                json!([{"op":"move","source":99,"destination":docs,"reason":reason}]),
                "unknown file ID 99",
            ),
            (
                json!([{"op":"move","source":files.pdf.display().to_string(),"destination":docs,"reason":reason}]),
                "not paths",
            ),
            (
                json!([{"op":"move","source":photo,"destination":std::env::temp_dir().display().to_string(),"reason":reason}]),
                "outside the owner-approved AI roots",
            ),
            (
                json!([{"op":"move","source":photo,"destination":"Missing\\Folder","reason":reason}]),
                "was not found in the approved roots",
            ),
            (
                json!([{"op":"move","source":photo,"destination":"..\\Docs","reason":reason}]),
                "without .. or a drive",
            ),
            (
                json!([{"op":"move","source":photo,"destination":pdf,"reason":reason}]),
                "is a file, not a folder",
            ),
            (
                json!([{"op":"rename","source":photo,"name":"a:b.jpg","reason":reason}]),
                "valid Windows filename",
            ),
            (
                json!([{"op":"rename","source":photo,"name":"CON","reason":reason}]),
                "valid Windows filename",
            ),
            (
                json!([{"op":"rename","source":photo,"name":"PHOTO.JPG","reason":reason}]),
                "case-only renames",
            ),
            (
                json!([{"op":"rename","source":photo,"name":"photo.jpg","reason":reason}]),
                "name is unchanged",
            ),
            (
                json!([{"op":"rename","source":photo,"name":"setup.exe","reason":reason}]),
                "already exists",
            ),
            (
                json!([{"op":"move","source":pdf,"destination":docs,"reason":reason}]),
                "already exists",
            ),
            (
                json!([{"op":"move","source":pdf,"destination":downloads,"reason":reason}]),
                "already in that folder",
            ),
            (
                json!([{"op":"move","source":docs,"destination":sub,"reason":reason}]),
                "into itself",
            ),
            (
                json!([{"op":"recycle","source":root,"reason":reason}]),
                "approved root itself",
            ),
            (
                json!([
                    {"op":"new_folder","parent":downloads,"name":"PDF","reason":reason},
                    {"op":"new_folder","parent":downloads,"name":"pdf","reason":reason}
                ]),
                "is also the target of operation 1",
            ),
            (
                json!([
                    {"op":"move","source":photo,"destination":docs,"reason":reason},
                    {"op":"recycle","source":photo,"reason":reason}
                ]),
                "already changed by operation 1",
            ),
            (
                json!([
                    {"op":"move","source":downloads,"destination":docs,"reason":reason},
                    {"op":"rename","source":photo,"name":"x.jpg","reason":reason}
                ]),
                "inside the source of operation 1",
            ),
            (
                json!([
                    {"op":"rename","source":docs,"name":"Documents","reason":reason},
                    {"op":"move","source":photo,"destination":sub,"reason":reason}
                ]),
                "its folder is changed by operation 1",
            ),
            (
                json!([
                    {"op":"move","source":photo,"destination":"f1","reason":reason},
                    {"op":"new_folder","parent":downloads,"name":"Images","reason":reason}
                ]),
                "used before its new_folder (operation 2)",
            ),
            (
                json!([{"op":"move","source":photo,"destination":"f9","reason":reason}]),
                "unknown ref f9",
            ),
            (
                json!([{"op":"new_folder","ref":"f2","parent":downloads,"name":"A","reason":reason}]),
                "ref is \"f1\"",
            ),
            (
                json!([
                    {"op":"new_folder","parent":downloads,"name":"bad:name","reason":reason},
                    {"op":"move","source":photo,"destination":"f1","reason":reason}
                ]),
                "comes from invalid operation 1",
            ),
            (
                json!([{"op":"new_folder","parent":downloads,"name":".ssh","reason":reason}]),
                "protected",
            ),
            (
                json!([{"op":"rename","source":photo,"name":".env","reason":reason}]),
                "protected",
            ),
            (
                json!([{"op":"move","source":photo,"destination":".ssh","reason":reason}]),
                "excluded",
            ),
            (
                json!([{"op":"recycle","source":configs,"reason":reason}]),
                "contains a protected credential/configuration path",
            ),
            (
                json!([{"op":"delete","source":photo,"reason":reason}]),
                "unknown op",
            ),
            (
                json!([{"op":"move","source":photo,"destination":docs,"name":"x","reason":reason}]),
                "does not take \"name\"",
            ),
            (
                json!([{"op":"move","source":photo,"reason":reason}]),
                "requires \"destination\"",
            ),
            (
                json!([{"op":"move","source":photo,"destination":docs}]),
                "reason",
            ),
            (
                json!([{"op":"move","source":photo,"destination":docs,"reason":reason,"overwrite":true}]),
                "malformed operation",
            ),
            (json!([]), "no operations"),
        ] {
            rejected(&mut tools, operations, expected);
        }
        let many: Vec<Value> = (0..=MAX_OPERATIONS)
            .map(|_| json!({"op":"recycle","source":setup,"reason":reason}))
            .collect();
        rejected(&mut tools, Value::Array(many), "exceed the limit of 500");
        assert!(
            validate("{\"summary\":\"x\"}", &mut tools).unwrap_err()[0]
                .contains("Malformed propose_changes arguments")
        );
        let problems = validate(
            &json!({"summary":"x","operations":(0..30).map(|_| json!({"op":"recycle","source":99,"reason":reason})).collect::<Vec<_>>()}).to_string(),
            &mut tools,
        )
        .unwrap_err();
        assert_eq!(problems.len(), MAX_PROBLEMS + 1);
        assert!(problems.last().unwrap().contains("10 more problems"));
        // A version changed after observation is stale; a fresh observation gets a new ID.
        fs::write(&files.photo, "changed photo with another size").unwrap();
        rejected(
            &mut tools,
            json!([{"op":"move","source":photo,"destination":docs,"reason":reason}]),
            "changed since it was observed",
        );
        assert_eq!(
            fs::read_to_string(&files.photo).unwrap(),
            "changed photo with another size"
        );
    }

    #[test]
    fn ids_from_another_run_are_unknown_and_folders_are_bound_by_identity() {
        let files = files();
        let mut first = files.fixture.tools();
        let pdf = id(&mut first, &files.pdf);
        let docs = id(&mut first, &files.docs);
        let operations = json!([{"op":"move","source":pdf,"destination":docs,"reason":"x"}]);
        assert!(propose(&mut first, operations.clone()).is_ok());
        let mut other = files.fixture.tools();
        rejected(&mut other, operations, "unknown file ID");
        // A destination folder whose contents changed keeps its identity and stays valid.
        fs::write(files.docs.join("new.txt"), "new").unwrap();
        let proposal = propose(
            &mut first,
            json!([{"op":"move","source":pdf,"destination":docs,"reason":"x"}]),
        )
        .unwrap();
        assert!(
            matches!(&proposal.steps[0].place, Some(Place::Existing(folder)) if folder.directory)
        );
    }

    #[test]
    fn batch_selection_renumbers_new_folder_dependencies_and_refuses_orphans() {
        let files = files();
        let mut tools = files.fixture.tools();
        let downloads = id(&mut tools, &files.downloads);
        let pdf = id(&mut tools, &files.pdf);
        let photo = id(&mut tools, &files.photo);
        let proposal = propose(
            &mut tools,
            json!([
                {"op":"rename","source":photo,"name":"renamed.jpg","reason":"x"},
                {"op":"new_folder","parent":downloads,"name":"PDF","reason":"x"},
                {"op":"move","source":pdf,"destination":"f1","reason":"x"}
            ]),
        )
        .unwrap();
        let settings = files.fixture.settings.clone();
        let path = files.fixture.root.join("access.json");
        let batch = Batch::new(
            &proposal,
            &[false, true, true],
            7,
            settings.clone(),
            path.clone(),
        )
        .unwrap();
        assert_eq!(batch.steps.len(), 2);
        assert_eq!(batch.steps[1].dependency(), Some(0));
        assert!(batch.key.starts_with("ai-"));
        assert!(
            Batch::new(
                &proposal,
                &[true, false, true],
                7,
                settings.clone(),
                path.clone()
            )
            .unwrap_err()
            .contains("needs a new folder")
        );
        assert!(
            Batch::new(&proposal, &[false, false, false], 7, settings, path)
                .unwrap_err()
                .contains("at least one")
        );
    }
}
