//! Reviewed Undo: an inverse plan built from a finished journal record.
//!
//! Nothing is undone automatically. An inverse plan is prepared from what the durable outcome
//! proves, validated against the current state, reviewed, explicitly approved and journaled with
//! `undo_of=<key>` like any other plan. Undo itself is never undone (no redo).

use crate::{
    changes,
    files::{self, FileResult, Outcome, Scope, display_path},
    journal::{self, Journal},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Recycle Bin deletion times may differ from the recorded window by clock granularity.
const CLOCK_TOLERANCE: Duration = Duration::from_secs(2);
/// Folder fingerprints stop at this many descendants; larger trees are not undoable.
const TREE_LIMIT: usize = 200_000;
/// Operations rows show this many recent records.
pub const HISTORY_LIMIT: usize = 20;
/// `history` stopped by the shared cancellation flag.
pub const HISTORY_CANCELLED: &str = "Reading the operation history was cancelled.";
/// Undo data format; 2 adds `undo_kept` for cross-volume originals left in place.
const FORMAT: u8 = 2;

#[cfg(test)]
thread_local! {
    /// Descendants `digest` visited on this thread.
    static WALKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What kind of operation a journal record holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Copy,
    Move,
    MoveAcross,
    MoveAcrossKept,
    Recycle,
    Rename,
    NewFolder,
    ZipCreate,
    ZipExtract,
    ZipExtractMove,
    ZipInsert,
    ZipInsertMove,
    Save,
    Undo,
}

const KINDS: [(Kind, &str, &str, &str); 14] = [
    (Kind::Copy, "copy", "Copy", "Undo Copy"),
    (Kind::Move, "move", "Move", "Undo Move"),
    (Kind::MoveAcross, "move-across", "Move", "Undo Move"),
    (
        Kind::MoveAcrossKept,
        "move-across-kept",
        "Move",
        "Undo Move",
    ),
    (Kind::Recycle, "delete", "Delete", "Undo Delete"),
    (Kind::Rename, "rename", "Rename", "Undo Rename"),
    (
        Kind::NewFolder,
        "new-folder",
        "New folder",
        "Undo New folder",
    ),
    (
        Kind::ZipCreate,
        "zip-create",
        "Create ZIP",
        "Undo Create ZIP",
    ),
    (
        Kind::ZipExtract,
        "zip-extract",
        "Extract ZIP",
        "Undo Extract ZIP",
    ),
    (
        Kind::ZipExtractMove,
        "zip-extract-move",
        "Move out of ZIP",
        "Undo",
    ),
    (Kind::ZipInsert, "zip-insert", "Copy into ZIP", "Undo"),
    (
        Kind::ZipInsertMove,
        "zip-insert-move",
        "Move into ZIP",
        "Undo",
    ),
    (Kind::Save, "save", "Save", "Undo"),
    (Kind::Undo, "undo", "Undo", "Undo"),
];

impl Kind {
    fn entry(self) -> (Self, &'static str, &'static str, &'static str) {
        *KINDS
            .iter()
            .find(|(kind, ..)| *kind == self)
            .expect("Every kind is listed")
    }

    pub fn key(self) -> &'static str {
        self.entry().1
    }

    pub fn parse(key: &str) -> Option<Self> {
        KINDS
            .iter()
            .find(|(_, name, ..)| *name == key)
            .map(|(kind, ..)| *kind)
    }

    pub fn label(self) -> &'static str {
        self.entry().2
    }

    pub fn undo_label(self) -> &'static str {
        self.entry().3
    }

    /// Why records of this kind are never undoable.
    pub fn refusal(self) -> Option<&'static str> {
        match self {
            Self::ZipInsert | Self::ZipInsertMove => Some("Not undoable: ZIP insert"),
            Self::ZipExtractMove => Some("Not undoable: move out of ZIP"),
            Self::Save => Some("Not undoable: source Save"),
            Self::Undo => Some("Undo cannot be undone (no redo)"),
            _ => None,
        }
    }

    /// The kind of a ZIP plan from its review label.
    pub(crate) fn archive(label: &str) -> Self {
        match label {
            "Create ZIP" => Self::ZipCreate,
            "Copy ZIP members to folder" => Self::ZipExtract,
            "Move ZIP members to folder" => Self::ZipExtractMove,
            "Move files into ZIP" => Self::ZipInsertMove,
            _ => Self::ZipInsert,
        }
    }
}

/// What an executed operation proves it did; recorded in the journal outcome for a later undo.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// `None` records nothing for undo.
    pub kind: Option<Kind>,
    /// Complete items the operation created; undo recycles each one if unchanged.
    pub created: Vec<PathBuf>,
    /// `(original, result)` pairs; undo moves each result back.
    pub moved: Vec<(PathBuf, PathBuf)>,
    /// Originals sent to the Recycle Bin; undo restores them first.
    pub recycled: Vec<PathBuf>,
    /// Destination files replaced into the Recycle Bin; undo restores them last.
    pub replaced: Vec<PathBuf>,
    /// Emptied source folders a merging Move removed; undo recreates them.
    pub removed: Vec<PathBuf>,
    /// Created but incomplete paths; reported, never undone.
    pub partial: Vec<PathBuf>,
    /// Cross-volume Move: `(original, copy)` pairs whose original was not recycled. Undo
    /// recycles the copy only while that original is still present and unchanged.
    pub kept: Vec<(PathBuf, PathBuf)>,
}

impl Effects {
    fn lines(&self, ended: i128) -> String {
        let Some(kind) = self.kind else {
            return String::new();
        };
        let encode = |path: &Path| journal::encode_path(path);
        let mut text = format!("undo_format={FORMAT}\nkind={}\nended={ended}\n", kind.key());
        for path in &self.created {
            let mark = Mark::read(path).map_or_else(|_| "-".into(), |mark| mark.encode());
            text.push_str(&format!("undo_created={} {mark}\n", encode(path)));
        }
        for (original, copy) in &self.kept {
            let mark = Mark::read(original).map_or_else(|_| "-".into(), |mark| mark.encode());
            text.push_str(&format!(
                "undo_kept={} {} {mark}\n",
                encode(original),
                encode(copy)
            ));
        }
        for (original, result) in &self.moved {
            let mark = Mark::identity(result).map_or_else(|_| "-".into(), |mark| mark.encode());
            text.push_str(&format!(
                "undo_moved={} {} {mark}\n",
                encode(original),
                encode(result)
            ));
        }
        for (name, paths) in [
            ("undo_recycled", &self.recycled),
            ("undo_replaced", &self.replaced),
            ("undo_removed", &self.removed),
            ("undo_partial", &self.partial),
        ] {
            for path in paths {
                text.push_str(&format!("{name}={}\n", encode(path)));
            }
        }
        text
    }

    /// Outcome lines for the journal; `ended` is the wall-clock completion time.
    pub(crate) fn journal_lines(&self, budget: u64) -> String {
        let text = self.lines(now());
        match self.kind {
            Some(kind) if text.len() as u64 > budget => format!(
                "undo_format={FORMAT}\nkind={}\nended={}\nundo_unavailable=record size\n",
                kind.key(),
                now()
            ),
            _ => text,
        }
    }
}

/// Paths in the shortest form without nested duplicates.
pub(crate) fn outermost(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort_by_key(|path| path.components().count());
    let mut kept: Vec<PathBuf> = Vec::new();
    for path in paths {
        if !kept.iter().any(|outer| path.starts_with(outer)) {
            kept.push(path);
        }
    }
    kept
}

fn nanos(time: SystemTime) -> i128 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_nanos() as i128,
        Err(before) => -(before.duration().as_nanos() as i128),
    }
}

fn time(nanos: i128) -> SystemTime {
    let duration = Duration::from_nanos(nanos.unsigned_abs().min(u64::MAX as u128) as u64);
    if nanos >= 0 {
        UNIX_EPOCH + duration
    } else {
        UNIX_EPOCH - duration
    }
}

/// Wall-clock now in nanoseconds since the Unix epoch, as recorded in the journal.
pub(crate) fn now() -> i128 {
    nanos(SystemTime::now())
}

/// Fingerprint of an operation result at completion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    directory: bool,
    id: (u32, u64),
    size: u64,
    modified: i128,
    tree: u64,
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

fn fnv(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
}

impl Mark {
    /// File identity, size and modification time, or a folder's identity and every descendant.
    pub fn read(path: &Path) -> FileResult<Self> {
        Self::inspect(path, true)
    }

    /// Identity only: moving an item back does not depend on its contents.
    pub fn identity(path: &Path) -> FileResult<Self> {
        Self::inspect(path, false)
    }

    fn inspect(path: &Path, tree: bool) -> FileResult<Self> {
        let fingerprint = files::fingerprint(path)?;
        #[cfg(windows)]
        let id = fingerprint.file_id;
        #[cfg(not(windows))]
        let id = (0, 0);
        let mut mark = Self {
            directory: fingerprint.directory,
            id,
            size: 0,
            modified: 0,
            tree: 0,
        };
        if !fingerprint.directory {
            mark.size = fingerprint.size;
            mark.modified = nanos(fingerprint.modified);
        } else if tree {
            (mark.size, mark.tree) = digest(path)?;
        }
        Ok(mark)
    }

    fn same_item(&self, other: &Self) -> bool {
        self.directory == other.directory && self.id == other.id
    }

    fn encode(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{:x}",
            if self.directory { "d" } else { "f" },
            self.id.0,
            self.id.1,
            self.size,
            self.modified,
            self.tree
        )
    }

    fn decode(value: &str) -> Option<Self> {
        let parts: Vec<&str> = value.split(':').collect();
        let [kind, volume, index, size, modified, tree] = parts.as_slice() else {
            return None;
        };
        Some(Self {
            directory: match *kind {
                "d" => true,
                "f" => false,
                _ => return None,
            },
            id: (volume.parse().ok()?, index.parse().ok()?),
            size: size.parse().ok()?,
            modified: modified.parse().ok()?,
            tree: u64::from_str_radix(tree, 16).ok()?,
        })
    }
}

/// Total file bytes and a stable hash of every descendant's name, type, identity, size and time.
fn digest(root: &Path) -> FileResult<(u64, u64)> {
    let mut hash = FNV_OFFSET;
    let mut total = 0u64;
    let mut count = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(folder) = stack.pop() {
        let mut children = fs::read_dir(&folder)
            .and_then(|entries| {
                entries
                    .map(|entry| entry.map(|entry| entry.path()))
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|e| format!("Read folder {}: {e}", display_path(&folder)))?;
        children.sort();
        for child in children {
            count += 1;
            #[cfg(test)]
            WALKED.with(|walked| walked.set(walked.get() + 1));
            if count > TREE_LIMIT {
                return Err(format!(
                    "Folder is too large to verify: {}",
                    display_path(root)
                ));
            }
            let relative = child.strip_prefix(root).unwrap_or(&child);
            fnv(&mut hash, journal::encode_path(relative).as_bytes());
            let metadata = fs::symlink_metadata(&child)
                .map_err(|e| format!("Inspect {}: {e}", display_path(&child)))?;
            if files::reparse(&metadata) {
                fnv(&mut hash, b"l");
                continue;
            }
            let mark = Mark::identity(&child)?;
            fnv(&mut hash, if mark.directory { b"d" } else { b"f" });
            fnv(&mut hash, &mark.id.0.to_le_bytes());
            fnv(&mut hash, &mark.id.1.to_le_bytes());
            if mark.directory {
                stack.push(child);
            } else {
                fnv(&mut hash, &mark.size.to_le_bytes());
                fnv(&mut hash, &mark.modified.to_le_bytes());
                total = total.saturating_add(mark.size);
            }
        }
    }
    Ok((total, hash))
}

/// One item in the Recycle Bin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinItem {
    /// Exact identity of the stored item (its `$R` path on Windows).
    pub id: String,
    pub original: PathBuf,
    pub deleted: SystemTime,
    pub directory: bool,
    pub size: Option<u64>,
}

/// Recycle Bin access for restoring items; tests inject a fixture bin.
pub trait RecycleBin {
    /// Items whose original path is one of `originals`, ignoring case.
    fn items(&self, originals: &[PathBuf], cancel: &AtomicBool) -> FileResult<Vec<BinItem>>;
    /// Moves exactly `item` out of the bin into the new, empty `folder` as `name`. Never
    /// overwrites and never shows Windows UI.
    fn restore(&self, item: &BinItem, folder: &Path, name: &OsStr) -> FileResult<()>;
    /// Whether items at `path` can be recycled; undo never falls back to permanent deletion.
    fn accepts(&self, path: &Path) -> bool {
        crate::platform::recycle_bin_available(path).unwrap_or(false)
    }
}

/// The Windows Recycle Bin through the Shell.
pub struct WindowsRecycleBin;

impl RecycleBin for WindowsRecycleBin {
    fn items(&self, originals: &[PathBuf], cancel: &AtomicBool) -> FileResult<Vec<BinItem>> {
        crate::platform::recycle_bin_items(originals, cancel)
    }

    fn restore(&self, item: &BinItem, folder: &Path, name: &OsStr) -> FileResult<()> {
        crate::platform::restore_recycled(item, folder, name)
    }
}

/// Windows path equality, ignoring case and extended-length prefixes.
pub fn same_path(a: &Path, b: &Path) -> bool {
    crate::platform::equal_ignore_case(&display_path(a), &display_path(b))
}

fn fold(path: &Path) -> String {
    display_path(path).to_lowercase()
}

#[derive(Default)]
struct Record {
    key: String,
    finished: bool,
    operation: String,
    started: Option<i128>,
    ended: Option<i128>,
    format: Option<u8>,
    unavailable: Option<String>,
    kind: Option<Kind>,
    error: Option<String>,
    cancelled: bool,
    changed: bool,
    damaged: bool,
    undo_of: Option<String>,
    undone: Option<Kind>,
    /// Shared key of the batch this record belongs to (`batch=` line).
    batch: Option<String>,
    destination: Option<PathBuf>,
    created: Vec<(PathBuf, Option<Mark>)>,
    moved: Vec<(PathBuf, PathBuf, Option<Mark>)>,
    recycled: Vec<PathBuf>,
    replaced: Vec<PathBuf>,
    removed: Vec<PathBuf>,
    partial: Vec<PathBuf>,
    /// `(original, copy, mark)` from `undo_kept`.
    kept: Vec<(PathBuf, PathBuf, Option<Mark>)>,
    /// The outcome lists `kept=` originals of a cross-volume Move.
    originals_kept: bool,
}

fn unescape(value: &str) -> String {
    let inner = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value);
    let mut text = String::new();
    let mut characters = inner.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            text.push(character);
            continue;
        }
        match characters.next() {
            Some('n') => text.push('\n'),
            Some('t') => text.push('\t'),
            Some('r') => text.push('\r'),
            Some(other) => text.push(other),
            None => (),
        }
    }
    text
}

impl Record {
    fn parse(key: String, text: &str) -> Self {
        let mut record = Self {
            key,
            finished: text.ends_with("finished\n"),
            ..Default::default()
        };
        let body = text.strip_prefix(journal::HEADER).unwrap_or(text);
        let (snapshot, outcome) = match body.find("\n\ncancelled=") {
            Some(index) => (&body[..index], &body[index + 2..]),
            None => (body, ""),
        };
        for line in snapshot.lines() {
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            match name {
                "operation" if record.operation.is_empty() => record.operation = value.to_owned(),
                "started" => record.started = value.parse().ok(),
                "undo_of" => record.undo_of = Some(value.to_owned()),
                "undone_kind" => record.undone = Kind::parse(value),
                "batch" => record.batch = Some(value.to_owned()),
                "destination" if record.destination.is_none() => {
                    record.destination = journal::decode_path(value).ok();
                }
                _ => (),
            }
        }
        for line in outcome.lines() {
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let parsed = (|| -> FileResult<()> {
                let pair = |value: &str| -> FileResult<(PathBuf, PathBuf, Option<Mark>)> {
                    let mut parts = value.splitn(3, ' ');
                    let (Some(from), Some(to), Some(mark)) =
                        (parts.next(), parts.next(), parts.next())
                    else {
                        return Err("Damaged undo line".into());
                    };
                    Ok((
                        journal::decode_path(from)?,
                        journal::decode_path(to)?,
                        Mark::decode(mark),
                    ))
                };
                match name {
                    "cancelled" => record.cancelled = value == "true",
                    "completed" | "created" => record.changed = true,
                    "kept" => record.originals_kept = true,
                    "error" => record.error = Some(unescape(value)),
                    "undo_format" => {
                        record.format = value
                            .parse()
                            .ok()
                            .filter(|format| (1..=FORMAT).contains(format));
                    }
                    "undo_unavailable" => record.unavailable = Some(value.to_owned()),
                    "kind" => record.kind = Kind::parse(value),
                    "ended" => record.ended = value.parse().ok(),
                    "undo_created" => {
                        let (path, mark) = value.split_once(' ').ok_or("Damaged undo line")?;
                        record
                            .created
                            .push((journal::decode_path(path)?, Mark::decode(mark)));
                    }
                    "undo_moved" => record.moved.push(pair(value)?),
                    "undo_kept" => record.kept.push(pair(value)?),
                    "undo_recycled" => record.recycled.push(journal::decode_path(value)?),
                    "undo_replaced" => record.replaced.push(journal::decode_path(value)?),
                    "undo_removed" => record.removed.push(journal::decode_path(value)?),
                    "undo_partial" => record.partial.push(journal::decode_path(value)?),
                    _ => (),
                }
                Ok(())
            })();
            record.damaged |= parsed.is_err();
        }
        record
    }

    /// Claim time when recorded, else the plan identity's timestamp.
    fn order(&self) -> (i128, &str) {
        (
            self.started.unwrap_or_else(|| {
                self.key
                    .split('-')
                    .next()
                    .and_then(|time| time.parse().ok())
                    .unwrap_or(0)
            }),
            &self.key,
        )
    }

    fn has_effects(&self) -> bool {
        !(self.created.is_empty()
            && self.moved.is_empty()
            && self.recycled.is_empty()
            && self.replaced.is_empty()
            && self.removed.is_empty())
    }

    fn partly(&self) -> bool {
        self.error.is_some() || self.cancelled || !self.partial.is_empty()
    }

    /// A cross-volume Move whose originals left in place were not journaled: its copies may be
    /// the only remaining data, so they are never recycled.
    fn unproven(&self) -> bool {
        matches!(self.kind, Some(Kind::MoveAcross | Kind::MoveAcrossKept))
            && !self.created.is_empty()
            && self.kept.is_empty()
            && (self.kind == Some(Kind::MoveAcrossKept)
                || self.originals_kept
                // Format 1 did not journal originals that a stopped Move left in place.
                || (self.format == Some(1) && self.partly()))
    }

    fn label(&self) -> String {
        let kind = match self.kind {
            Some(kind) => kind,
            None => {
                return match self.operation.as_str() {
                    "Recycle" => "Delete".into(),
                    "" => "Unknown operation".into(),
                    other => other.to_owned(),
                };
            }
        };
        let name = |path: &Path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| display_path(path))
        };
        let detail = match kind {
            Kind::Rename => self
                .moved
                .first()
                .map(|(from, to, _)| format!("{} to {}", name(from), name(to))),
            Kind::NewFolder | Kind::ZipCreate => self.created.first().map(|(path, _)| name(path)),
            Kind::Undo => {
                let label = self.undone.map_or("Undo", Kind::undo_label);
                return if self.partly() {
                    format!("{label} (stopped; see its outcome)")
                } else {
                    label.to_owned()
                };
            }
            _ => {
                let count = self.created.len()
                    + self.moved.len()
                    + if kind == Kind::Recycle {
                        self.recycled.len()
                    } else {
                        0
                    };
                (count > 0).then(|| {
                    let mut text = items(count);
                    if let Some(destination) = &self.destination {
                        text.push_str(&format!(" to {}", display_path(destination)));
                    }
                    text
                })
            }
        };
        let mut label = match detail {
            Some(detail) => format!("{}: {detail}", kind.label()),
            None => kind.label().to_owned(),
        };
        if self.partly() {
            label.push_str(" (partial)");
        }
        label
    }
}

fn items(count: usize) -> String {
    format!("{count} {}", if count == 1 { "item" } else { "items" })
}

fn load(journal: &Journal) -> FileResult<Vec<Record>> {
    let mut records: Vec<Record> = journal
        .records()?
        .into_iter()
        .filter_map(|(path, text)| {
            let key = path.file_stem()?.to_str()?.to_owned();
            Some(Record::parse(key, &text))
        })
        .collect();
    records.sort_by(|a, b| b.order().cmp(&a.order()));
    Ok(records)
}

/// Whether a record text proves an undo made changes (or left staging paths).
pub(crate) fn undo_changed(text: &str) -> bool {
    Record::parse(String::new(), text).changed
}

/// Undo state of a recorded operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Undoable,
    Undone,
    /// A short reason, e.g. "Not undoable: ZIP insert" or "Changed since: D:\x".
    Refused(String),
}

/// One recent operation for the Operations dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub label: String,
    pub age: Option<Duration>,
    pub status: Status,
    /// Key of the batch (for example AI-proposed changes) this operation was a step of.
    pub batch: Option<String>,
}

impl Entry {
    /// "Move: 3 items to D:\x / 5 min ago".
    pub fn title(&self) -> String {
        match self.age {
            Some(age) => format!("{} / {}", self.label, ago(age)),
            None => self.label.clone(),
        }
    }
}

fn ago(age: Duration) -> String {
    let seconds = age.as_secs();
    match seconds {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86_400),
    }
}

/// Undo records by the key of the operation they undo.
type Links<'a> = BTreeMap<&'a str, Vec<&'a Record>>;

fn links(records: &[Record]) -> Links<'_> {
    let mut links = Links::new();
    for record in records {
        if let Some(original) = &record.undo_of {
            links.entry(original.as_str()).or_default().push(record);
        }
    }
    links
}

fn status(record: &Record, links: &Links) -> Status {
    let refused = |reason: &str| Status::Refused(reason.to_owned());
    if !record.finished {
        return refused("Interrupted; reconcile first");
    }
    if record.damaged {
        return refused("Not undoable: damaged record");
    }
    if record.undo_of.is_some() || record.kind == Some(Kind::Undo) {
        return refused("Undo cannot be undone (no redo)");
    }
    let Some(kind) = record.kind.filter(|_| record.format.is_some()) else {
        return refused("Not undoable (recorded by an older version)");
    };
    if let Some(reason) = &record.unavailable {
        return Status::Refused(format!("Not undoable: {reason}"));
    }
    if let Some(reason) = kind.refusal() {
        return refused(reason);
    }
    let mut undone = None;
    for undo in links.get(record.key.as_str()).into_iter().flatten() {
        if !undo.finished {
            return refused("Undo interrupted; reconcile first");
        }
        if undo.changed {
            undone = Some(undone.unwrap_or(false) || undo.error.is_some() || undo.cancelled);
        }
    }
    match undone {
        Some(false) => return Status::Undone,
        Some(true) => return refused("Partly undone; inspect Operations"),
        None => (),
    }
    if !record.has_effects() {
        return refused("Nothing to undo");
    }
    if record.unproven() {
        return refused("Not undoable: kept originals not recorded");
    }
    Status::Undoable
}

/// Recent operations, newest first, with their current undo state. Rows use only cheap checks
/// (the record, its undo state, existence and top-level identity of results and kept
/// originals); folder contents and the Recycle Bin are verified by `prepare`.
pub fn history(
    scope: &Scope,
    journal: &Journal,
    limit: usize,
    cancel: &AtomicBool,
) -> FileResult<Vec<Entry>> {
    let records = load(journal)?;
    let links = links(&records);
    let now = SystemTime::now();
    let mut entries = Vec::new();
    for record in records.iter().take(limit) {
        let mut status = status(record, &links);
        if status == Status::Undoable
            && let Err(reason) = bind(scope, record, None, cancel)
        {
            status = Status::Refused(reason);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(HISTORY_CANCELLED.into());
        }
        entries.push(Entry {
            key: record.key.clone(),
            label: record.label(),
            age: record
                .started
                .and_then(|started| now.duration_since(time(started)).ok()),
            status,
            batch: record.batch.clone(),
        });
    }
    Ok(entries)
}

#[derive(Clone, Debug)]
enum Step {
    Recreate(PathBuf),
    MoveBack {
        current: PathBuf,
        original: PathBuf,
        mark: Mark,
    },
    Recycle {
        path: PathBuf,
        mark: Mark,
        /// Cross-volume Move: the kept original that must still be present and unchanged.
        original: Option<(PathBuf, Mark)>,
    },
    Restore {
        original: PathBuf,
        item: BinItem,
    },
}

impl Step {
    fn path(&self) -> &Path {
        match self {
            Self::Recreate(path) | Self::Recycle { path, .. } => path,
            Self::MoveBack { original, .. } | Self::Restore { original, .. } => original,
        }
    }

    fn row(&self) -> String {
        match self {
            Self::Recreate(path) => format!("Recreate emptied folder: {}", display_path(path)),
            Self::MoveBack {
                current, original, ..
            } => format!(
                "Move back: {} -> {}",
                display_path(current),
                display_path(original)
            ),
            Self::Recycle { path, original, .. } => format!(
                "Recycle (unchanged since the operation): {}{}",
                display_path(path),
                original
                    .as_ref()
                    .map(|(original, _)| format!("; original present: {}", display_path(original)))
                    .unwrap_or_default()
            ),
            Self::Restore { original, item } => format!(
                "Restore from Recycle Bin: {}{}",
                display_path(original),
                SystemTime::now()
                    .duration_since(item.deleted)
                    .map(|age| format!(" (deleted {})", ago(age)))
                    .unwrap_or_default()
            ),
        }
    }
}

enum Draft<'a> {
    Recreate(&'a Path),
    MoveBack(&'a Path, &'a Path, Option<&'a Mark>),
    /// A created item and, for a cross-volume Move, its kept original.
    Recycle(
        &'a Path,
        Option<&'a Mark>,
        Option<(&'a Path, Option<&'a Mark>)>,
    ),
    Restore(&'a Path),
}

fn exists(path: &Path) -> FileResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Inspect {}: {error}", display_path(path))),
    }
}

/// Whether `path` is still the recorded item. Without `full` (Operations rows) a folder is
/// compared by identity only; preparing and executing also compare every descendant.
fn unchanged(path: &Path, mark: &Mark, full: bool) -> bool {
    let Ok(actual) = Mark::identity(path) else {
        return false;
    };
    if !mark.directory {
        return actual == *mark;
    }
    actual.same_item(mark) && (!full || Mark::read(path).ok().as_ref() == Some(mark))
}

/// A copy is recycled only while its kept original is still present and unchanged.
fn present<'m>(original: &Path, mark: Option<&'m Mark>, full: bool) -> Result<&'m Mark, String> {
    let kept = |reason: &str| format!("{reason}: {}; the copy is kept.", display_path(original));
    if !exists(original)? {
        return Err(kept("Original no longer present"));
    }
    match mark {
        Some(mark) if unchanged(original, mark, full) => Ok(mark),
        Some(_) => Err(kept("Original changed since")),
        None => Err(kept("Original could not be verified")),
    }
}

fn changed(path: &Path) -> String {
    format!("Changed since: {}", display_path(path))
}

fn occupied(path: &Path) -> String {
    format!("Original path occupied: {}", display_path(path))
}

/// Builds the inverse steps and validates them against the current state. Without a bin
/// (Operations rows) only cheap checks run: existence and top-level identity; folder contents
/// and Recycle Bin items are verified when preparing.
fn bind(
    scope: &Scope,
    record: &Record,
    bin: Option<&dyn RecycleBin>,
    cancel: &AtomicBool,
) -> Result<(Vec<Step>, Vec<String>), String> {
    let full = bin.is_some();
    let mut removed: Vec<&Path> = record.removed.iter().map(PathBuf::as_path).collect();
    removed.sort_by_key(|path| path.components().count());
    let mut drafts: Vec<Draft> = removed.into_iter().map(Draft::Recreate).collect();
    drafts.extend(record.recycled.iter().map(|path| Draft::Restore(path)));
    drafts.extend(
        record
            .moved
            .iter()
            .map(|(original, current, mark)| Draft::MoveBack(current, original, mark.as_ref())),
    );
    let originals: BTreeMap<&Path, (&Path, Option<&Mark>)> = record
        .kept
        .iter()
        .map(|(original, copy, mark)| (copy.as_path(), (original.as_path(), mark.as_ref())))
        .collect();
    drafts.extend(record.created.iter().map(|(path, mark)| {
        Draft::Recycle(path, mark.as_ref(), originals.get(path.as_path()).copied())
    }));
    drafts.extend(record.replaced.iter().map(|path| Draft::Restore(path)));
    let restores: Vec<PathBuf> = drafts
        .iter()
        .filter_map(|draft| match draft {
            Draft::Restore(path) => Some(path.to_path_buf()),
            _ => None,
        })
        .collect();
    let bin_items = match bin {
        Some(bin) if !restores.is_empty() => Some(bin.items(&restores, cancel)?),
        _ => None,
    };
    let window = match (record.started, record.ended) {
        (Some(started), Some(ended)) => Some((
            time(started) - CLOCK_TOLERANCE,
            time(ended) + CLOCK_TOLERANCE,
        )),
        _ => None,
    };
    let mut freed = BTreeSet::new();
    let mut made = BTreeSet::new();
    let mut steps = Vec::new();
    let mut notes: Vec<String> = record
        .partial
        .iter()
        .map(|path| format!("Not undone (incomplete): {}", display_path(path)))
        .collect();
    let free = |path: &Path, freed: &BTreeSet<String>| -> Result<(), String> {
        if exists(path)? && !freed.contains(&fold(path)) {
            return Err(occupied(path));
        }
        Ok(())
    };
    let parent = |path: &Path, made: &BTreeSet<String>| -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("No parent folder: {}", display_path(path)))?;
        if made.contains(&fold(parent)) {
            return Ok(());
        }
        match scope.resolve(parent) {
            Ok(resolved) if resolved == parent && resolved.is_dir() => Ok(()),
            Ok(_) => Err(format!("Folder changed: {}", display_path(parent))),
            Err(_) if !exists(parent)? => Err(format!("Missing folder: {}", display_path(parent))),
            Err(error) => Err(error),
        }
    };
    for draft in drafts {
        if cancel.load(Ordering::Acquire) {
            return Err("Undo planning was cancelled.".into());
        }
        match draft {
            Draft::Recreate(path) => {
                parent(path, &made)?;
                match fs::symlink_metadata(path) {
                    Ok(metadata) if metadata.is_dir() && !files::reparse(&metadata) => {
                        notes.push(format!("Folder already exists: {}", display_path(path)));
                    }
                    Ok(_) => return Err(occupied(path)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        made.insert(fold(path));
                        steps.push(Step::Recreate(path.to_path_buf()));
                    }
                    Err(error) => return Err(format!("Inspect {}: {error}", display_path(path))),
                }
            }
            Draft::MoveBack(current, original, mark) => {
                let mark = mark.ok_or_else(|| changed(current))?;
                if scope.mutation_source(current).ok().as_deref() != Some(current)
                    || !Mark::identity(current).is_ok_and(|actual| actual.same_item(mark))
                {
                    return Err(changed(current));
                }
                free(original, &freed)?;
                parent(original, &made)?;
                freed.insert(fold(current));
                steps.push(Step::MoveBack {
                    current: current.to_path_buf(),
                    original: original.to_path_buf(),
                    mark: mark.clone(),
                });
            }
            Draft::Recycle(path, mark, original) => {
                let mark = mark.ok_or_else(|| changed(path))?;
                if scope.mutation_source(path).ok().as_deref() != Some(path)
                    || !unchanged(path, mark, full)
                {
                    return Err(changed(path));
                }
                let original = match original {
                    Some((original, kept)) => Some((
                        original.to_path_buf(),
                        present(original, kept, full)?.clone(),
                    )),
                    None => None,
                };
                if !bin.unwrap_or(&WindowsRecycleBin).accepts(path) {
                    return Err(format!(
                        "No Recycle Bin on this drive; undo never deletes permanently: {}",
                        display_path(path)
                    ));
                }
                freed.insert(fold(path));
                steps.push(Step::Recycle {
                    path: path.to_path_buf(),
                    mark: mark.clone(),
                    original,
                });
            }
            Draft::Restore(original) => {
                free(original, &freed)?;
                parent(original, &made)?;
                freed.remove(&fold(original));
                let Some(found) = &bin_items else {
                    if bin.is_none() {
                        continue;
                    }
                    return Err("Recycle Bin was not searched.".into());
                };
                let (from, to) =
                    window.ok_or("Not undoable: the operation times were not recorded.")?;
                let matches: Vec<&BinItem> = found
                    .iter()
                    .filter(|item| {
                        same_path(&item.original, original)
                            && item.deleted >= from
                            && item.deleted <= to
                    })
                    .collect();
                match matches.as_slice() {
                    [] => {
                        return Err(format!(
                            "Not in Recycle Bin (emptied or restored): {}",
                            display_path(original)
                        ));
                    }
                    [item] => steps.push(Step::Restore {
                        original: original.to_path_buf(),
                        item: (*item).clone(),
                    }),
                    many => {
                        return Err(format!(
                            "Ambiguous Recycle Bin match ({} items): {}",
                            many.len(),
                            display_path(original)
                        ));
                    }
                }
            }
        }
    }
    Ok((steps, notes))
}

/// An inverse plan awaiting explicit approval.
pub struct Plan {
    pub id: u64,
    key: String,
    original: String,
    kind: Kind,
    steps: Vec<Step>,
    notes: Vec<String>,
    partial: bool,
}

/// Prepares the inverse plan for `key`, or for the most recent undoable, not-yet-undone
/// operation. Nothing changes before approval.
pub fn prepare(
    scope: &Scope,
    journal: &Journal,
    id: u64,
    key: Option<&str>,
    cancel: &AtomicBool,
) -> FileResult<Plan> {
    prepare_with(scope, journal, id, key, cancel, &WindowsRecycleBin)
}

/// `prepare` with an injected Recycle Bin.
#[doc(hidden)]
pub fn prepare_with(
    scope: &Scope,
    journal: &Journal,
    id: u64,
    key: Option<&str>,
    cancel: &AtomicBool,
    bin: &dyn RecycleBin,
) -> FileResult<Plan> {
    let records = load(journal)?;
    let links = links(&records);
    let (record, skipped) = match key {
        Some(key) => {
            let record = records
                .iter()
                .find(|record| record.key == key)
                .ok_or("Operation record not found; it may have been archived.")?;
            match status(record, &links) {
                Status::Undoable => (record, 0),
                Status::Undone => return Err("Already undone; there is no second undo.".into()),
                Status::Refused(reason) => return Err(reason),
            }
        }
        None => {
            let latest = records
                .first()
                .ok_or("Nothing to undo: no recorded operations.")?;
            let position = records
                .iter()
                .position(|record| status(record, &links) == Status::Undoable)
                .ok_or_else(|| {
                    format!(
                        "Nothing to undo. Latest operation: {} / {}",
                        latest.label(),
                        match status(latest, &links) {
                            Status::Refused(reason) => reason,
                            Status::Undone => "Undone".into(),
                            Status::Undoable => "Undoable".into(),
                        }
                    )
                })?;
            // Newer operations that were never undone and cannot be.
            let skipped = records[..position]
                .iter()
                .filter(|record| {
                    record.undo_of.is_none() && matches!(status(record, &links), Status::Refused(_))
                })
                .count();
            (&records[position], skipped)
        }
    };
    let (steps, mut notes) = bind(scope, record, Some(bin), cancel)?;
    if steps.is_empty() {
        return Err("Nothing to undo: the recorded changes are already reversed.".into());
    }
    if skipped > 0 {
        notes.push(format!(
            "Skipped {skipped} newer operation{} that cannot be undone; see Operations.",
            if skipped == 1 { "" } else { "s" }
        ));
    }
    Ok(Plan {
        id,
        key: changes::identity()?,
        original: record.key.clone(),
        kind: record.kind.ok_or("Operation kind missing.")?,
        steps,
        notes,
        partial: record.partly(),
    })
}

/// An undo of every operation recorded with one `batch=` key, in reverse order. Rows are checked
/// cheaply here; each step's exact inverse is prepared and validated again just before it runs,
/// because earlier undo steps change the state later ones are checked against (for example a
/// created folder is only empty again after the items moved into it went back).
#[derive(Debug)]
pub struct BatchPlan {
    pub id: u64,
    batch: String,
    /// Operation keys and labels, newest first (the undo order).
    steps: Vec<(String, String)>,
    notes: Vec<String>,
}

/// Prepares (never executes) an undo of the batch `batch`. Refuses the whole batch when any
/// of its effective operations cannot be undone; each can still be undone on its own.
pub fn prepare_batch(
    scope: &Scope,
    journal: &Journal,
    id: u64,
    batch: &str,
    cancel: &AtomicBool,
) -> FileResult<BatchPlan> {
    let records = load(journal)?;
    let links = links(&records);
    let mut steps = Vec::new();
    let mut notes = Vec::new();
    let mut found = false;
    for record in records
        .iter()
        .filter(|record| record.batch.as_deref() == Some(batch))
    {
        found = true;
        if cancel.load(Ordering::Acquire) {
            return Err("Undo planning was cancelled.".into());
        }
        match status(record, &links) {
            Status::Undoable => {
                if let Err(reason) = bind(scope, record, None, cancel) {
                    return Err(format!(
                        "Cannot undo the batch: {}: {reason}. Undo its steps one by one in Operations.",
                        record.label()
                    ));
                }
                steps.push((record.key.clone(), record.label()));
            }
            Status::Undone => notes.push(format!("Already undone: {}", record.label())),
            Status::Refused(reason) if reason == "Nothing to undo" => {
                notes.push(format!("Nothing to undo: {}", record.label()));
            }
            Status::Refused(reason) => {
                return Err(format!(
                    "Cannot undo the batch: {}: {reason}. Undo its steps one by one in Operations.",
                    record.label()
                ));
            }
        }
    }
    if !found {
        return Err("Batch not found; its records may have been archived.".into());
    }
    if steps.is_empty() {
        return Err("Nothing to undo in this batch.".into());
    }
    Ok(BatchPlan {
        id,
        batch: batch.to_owned(),
        steps,
        notes,
    })
}

impl BatchPlan {
    pub fn label(&self) -> &'static str {
        "Undo batch"
    }

    /// "Undo batch: 5 operations in reverse order".
    pub fn heading(&self) -> String {
        format!(
            "Undo batch: {} in reverse order",
            match self.steps.len() {
                1 => "1 operation".to_owned(),
                count => format!("{count} operations"),
            }
        )
    }

    pub fn batch(&self) -> &str {
        &self.batch
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn effect_lines(&self) -> Vec<String> {
        vec![
            "Each operation is undone like a single Undo: items move back, unchanged results go to the Recycle Bin and recycled originals are restored.".into(),
            "Each step is prepared and checked again just before it runs; the batch stops at the first change.".into(),
            "Nothing is overwritten. This undo cannot be undone (no redo).".into(),
        ]
    }

    /// The operations in undo order, then notes.
    pub fn rows(&self) -> Vec<String> {
        self.steps
            .iter()
            .enumerate()
            .map(|(index, (_, label))| format!("{}. Undo {label}", index + 1))
            .chain(self.notes.iter().cloned())
            .collect()
    }

    pub fn execute(
        &self,
        scope: &Scope,
        journal: &Journal,
        approved: bool,
        cancel: &AtomicBool,
    ) -> Outcome {
        self.execute_with(scope, journal, approved, cancel, &Hooks::default())
    }

    /// Undoes each operation in order with a freshly prepared, journaled inverse plan; stops at
    /// the first refusal, failure or cancellation and reports what was not undone.
    #[doc(hidden)]
    pub fn execute_with(
        &self,
        scope: &Scope,
        journal: &Journal,
        approved: bool,
        cancel: &AtomicBool,
        hooks: &Hooks,
    ) -> Outcome {
        let mut total = Outcome {
            id: self.id,
            effects: Effects {
                kind: Some(Kind::Undo),
                ..Default::default()
            },
            ..Default::default()
        };
        if !approved {
            total.error = Some("Explicit undo approval is required; no changes made.".into());
            return total;
        }
        let mut stopped = None;
        for (index, (key, _)) in self.steps.iter().enumerate() {
            if cancel.load(Ordering::Acquire) {
                total.cancelled = true;
                stopped = Some((index, "Cancelled before this operation.".to_owned()));
                break;
            }
            let plan = match prepare_with(scope, journal, self.id, Some(key), cancel, hooks.bin) {
                Ok(plan) => plan,
                Err(error) => {
                    stopped = Some((index, error));
                    break;
                }
            };
            let outcome = plan.execute_with(scope, journal, true, cancel, hooks);
            total.completed.extend(outcome.completed);
            total.created.extend(outcome.created);
            total.incomplete.extend(outcome.incomplete);
            total.cancelled |= outcome.cancelled;
            if let Some(error) = outcome.error {
                stopped = Some((index, error));
                break;
            }
            if outcome.cancelled {
                stopped = Some((index, "Cancelled during this operation.".to_owned()));
                break;
            }
        }
        if let Some((index, error)) = stopped {
            let remaining: Vec<&str> = self.steps[index..]
                .iter()
                .map(|(_, label)| label.as_str())
                .collect();
            total.error = Some(format!(
                "Batch undo stopped after {index} of {} operations: {error}\nNot (fully) undone: {}",
                self.steps.len(),
                remaining.join("; ")
            ));
        }
        total
    }
}

/// Effect seams for tests; `Hooks::default()` uses the real Recycle Bin.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct Hooks<'a> {
    /// Sends items to the Recycle Bin; must never delete permanently.
    pub recycle: &'a dyn Fn(&[PathBuf]) -> FileResult<()>,
    pub bin: &'a dyn RecycleBin,
}

impl Default for Hooks<'static> {
    fn default() -> Self {
        Self {
            recycle: &crate::platform::recycle_many,
            bin: &WindowsRecycleBin,
        }
    }
}

fn place(paths: &[&Path]) -> String {
    let mut common: Option<&Path> = None;
    for path in paths {
        let Some(parent) = path.parent() else {
            return "several folders".into();
        };
        common = match common {
            None => Some(parent),
            Some(common) => match common
                .ancestors()
                .find(|ancestor| parent.starts_with(ancestor))
            {
                Some(ancestor) if !ancestor.as_os_str().is_empty() => Some(ancestor),
                _ => return "several folders".into(),
            },
        };
    }
    common.map_or_else(|| "several folders".into(), display_path)
}

impl Plan {
    /// "Undo Move".
    pub fn label(&self) -> &'static str {
        self.kind.undo_label()
    }

    /// The key of the operation this plan undoes.
    pub fn undoes(&self) -> &str {
        &self.original
    }

    /// Whether the undone operation only partly completed.
    pub fn partial(&self) -> bool {
        self.partial
    }

    /// "Undo Move: 3 items back to D:\x".
    pub fn heading(&self) -> String {
        let name = |path: &Path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| display_path(path))
        };
        if self.kind == Kind::Rename
            && let [
                Step::MoveBack {
                    current, original, ..
                },
            ] = self.steps.as_slice()
        {
            return format!(
                "{}: {} back to {}",
                self.label(),
                name(current),
                name(original)
            );
        }
        let mut groups: Vec<(u8, Vec<&Path>)> = Vec::new();
        for step in &self.steps {
            let group = match step {
                Step::Recreate(_) => continue,
                Step::MoveBack { .. } => 0,
                Step::Recycle { .. } => 1,
                Step::Restore { .. } => 2,
            };
            match groups.iter_mut().find(|(kind, _)| *kind == group) {
                Some((_, paths)) => paths.push(step.path()),
                None => groups.push((group, vec![step.path()])),
            }
        }
        let parts: Vec<String> = groups
            .iter()
            .map(|(group, paths)| match group {
                0 => format!("{} back to {}", items(paths.len()), place(paths)),
                1 => format!("recycle {} in {}", items(paths.len()), place(paths)),
                _ => format!(
                    "restore {} from the Recycle Bin to {}",
                    items(paths.len()),
                    place(paths)
                ),
            })
            .collect();
        if parts.is_empty() {
            return format!("{}: recreate emptied folders", self.label());
        }
        format!("{}: {}", self.label(), parts.join(", "))
    }

    /// Short effect lines under the heading.
    pub fn effect_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self
            .steps
            .iter()
            .any(|step| matches!(step, Step::Recycle { .. }))
        {
            lines.push("Unchanged results go to the Recycle Bin.".into());
        }
        if self.steps.iter().any(|step| {
            matches!(
                step,
                Step::Recycle {
                    original: Some(_),
                    ..
                }
            )
        }) {
            lines.push("A copy is recycled only while its kept original is unchanged.".into());
        }
        lines.push("Nothing is overwritten; a changed item stops the undo.".into());
        if self.partial {
            lines.push("Partial operation: only completed items are undone.".into());
        }
        lines.push("This undo cannot be undone (no redo).".into());
        lines
    }

    /// Every step and note, for Details.
    pub fn rows(&self) -> Vec<String> {
        self.steps
            .iter()
            .map(Step::row)
            .chain(self.notes.iter().cloned())
            .collect()
    }

    fn stage(&self, index: usize, original: &Path) -> Option<PathBuf> {
        Some(
            original
                .parent()?
                .join(format!(".tomas-commander-restore-{}-{index}", self.key)),
        )
    }

    fn snapshot(&self) -> (String, Vec<PathBuf>) {
        let encode = |path: &Path| journal::encode_path(path);
        let mut text = format!(
            "operation=Undo\nundo_of={}\nid={}\nundone_kind={}\n",
            self.original,
            self.id,
            self.kind.key()
        );
        let mut affected = Vec::new();
        for (index, step) in self.steps.iter().enumerate() {
            match step {
                Step::Recreate(path) => {
                    text.push_str(&format!("step={index} recreate\ntarget={}\n", encode(path)));
                    affected.push(path.clone());
                }
                Step::MoveBack {
                    current,
                    original,
                    mark,
                } => {
                    text.push_str(&format!(
                        "step={index} move-back\nsource={}\ntarget={}\nmark={}\n",
                        encode(current),
                        encode(original),
                        mark.encode()
                    ));
                    affected.extend([current.clone(), original.clone()]);
                }
                Step::Recycle {
                    path,
                    mark,
                    original,
                } => {
                    text.push_str(&format!(
                        "step={index} recycle\nsource={}\nmark={}\n",
                        encode(path),
                        mark.encode()
                    ));
                    affected.push(path.clone());
                    if let Some((original, mark)) = original {
                        text.push_str(&format!(
                            "original={}\noriginal_mark={}\n",
                            encode(original),
                            mark.encode()
                        ));
                        affected.push(original.clone());
                    }
                }
                Step::Restore { original, item } => {
                    text.push_str(&format!(
                        "step={index} restore\ntarget={}\nbin_item={}\n",
                        encode(original),
                        encode(Path::new(&item.id))
                    ));
                    affected.push(original.clone());
                    if let Some(stage) = self.stage(index, original) {
                        text.push_str(&format!("target={}\n", encode(&stage)));
                        affected.push(stage);
                    }
                }
            }
        }
        (text, affected)
    }

    pub fn execute(
        &self,
        scope: &Scope,
        journal: &Journal,
        approved: bool,
        cancel: &AtomicBool,
    ) -> Outcome {
        self.execute_with(scope, journal, approved, cancel, &Hooks::default())
    }

    /// `execute` with Recycle Bin seams.
    #[doc(hidden)]
    pub fn execute_with(
        &self,
        scope: &Scope,
        journal: &Journal,
        approved: bool,
        cancel: &AtomicBool,
        hooks: &Hooks,
    ) -> Outcome {
        let mut outcome = Outcome {
            id: self.id,
            incomplete: self.steps.iter().map(|step| step.path().into()).collect(),
            effects: Effects {
                kind: Some(Kind::Undo),
                ..Default::default()
            },
            ..Default::default()
        };
        if !approved {
            outcome.error = Some("Explicit undo approval is required; no changes made.".into());
            return outcome;
        }
        if cancel.load(Ordering::Acquire) {
            outcome.cancelled = true;
            outcome.error = Some("Undo cancelled before any change.".into());
            return outcome;
        }
        let (snapshot, affected) = self.snapshot();
        let record = match journal.claim_snapshot(&self.key, &snapshot, &affected) {
            Ok(record) => record,
            Err(error) => {
                outcome.error = Some(error);
                return outcome;
            }
        };
        let mut done = 0;
        let result = (|| -> FileResult<()> {
            for (index, step) in self.steps.iter().enumerate() {
                if cancel.load(Ordering::Acquire) {
                    outcome.cancelled = true;
                    return Ok(());
                }
                self.apply(scope, index, step, hooks, &mut outcome)?;
                outcome.completed.push(step.path().to_path_buf());
                done = index + 1;
            }
            Ok(())
        })();
        outcome.incomplete = self.steps[done..]
            .iter()
            .map(|step| step.path().to_path_buf())
            .collect();
        if let Err(error) = result {
            outcome.error = Some(error);
        }
        Journal::finish(record, outcome)
    }

    fn apply(
        &self,
        scope: &Scope,
        index: usize,
        step: &Step,
        hooks: &Hooks,
        outcome: &mut Outcome,
    ) -> FileResult<()> {
        let parent = |path: &Path| -> FileResult<()> {
            let parent = path.parent().ok_or("No parent folder.")?;
            if scope.resolve(parent)? != parent {
                return Err(format!("Folder changed: {}", display_path(parent)));
            }
            Ok(())
        };
        match step {
            Step::Recreate(path) => {
                parent(path)?;
                fs::create_dir(path)
                    .map_err(|e| format!("Recreate emptied folder {}: {e}", display_path(path)))?;
                outcome.created.push(path.clone());
            }
            Step::MoveBack {
                current,
                original,
                mark,
            } => {
                if scope.mutation_source(current).ok().as_ref() != Some(current)
                    || !Mark::identity(current).is_ok_and(|actual| actual.same_item(mark))
                {
                    return Err(changed(current));
                }
                parent(original)?;
                if exists(original)? {
                    return Err(format!("{}; nothing was moved.", occupied(original)));
                }
                crate::platform::move_no_replace(current, original)?;
                if exists(current)?
                    || !Mark::identity(original).is_ok_and(|actual| actual.same_item(mark))
                {
                    return Err(format!(
                        "Move-back read-back is uncertain; inspect {} and {}",
                        display_path(current),
                        display_path(original)
                    ));
                }
            }
            Step::Recycle {
                path,
                mark,
                original,
            } => {
                if scope.mutation_source(path).ok().as_ref() != Some(path)
                    || !unchanged(path, mark, true)
                {
                    return Err(changed(path));
                }
                if let Some((original, mark)) = original {
                    present(original, Some(mark), true)?;
                }
                (hooks.recycle)(std::slice::from_ref(path))?;
                if exists(path)? {
                    return Err(format!("Windows did not recycle: {}", display_path(path)));
                }
            }
            Step::Restore { original, item } => {
                self.restore(scope, index, original, item, hooks, outcome)?;
            }
        }
        Ok(())
    }

    /// Moves the bin item into a new staging folder beside the original, verifies it and
    /// publishes it without replacement. A failure keeps the item in the bin or in staging.
    fn restore(
        &self,
        scope: &Scope,
        index: usize,
        original: &Path,
        item: &BinItem,
        hooks: &Hooks,
        outcome: &mut Outcome,
    ) -> FileResult<()> {
        let parent = original.parent().ok_or("No parent folder.")?;
        let name = original.file_name().ok_or("No item name.")?;
        if scope.resolve(parent)? != parent {
            return Err(format!("Folder changed: {}", display_path(parent)));
        }
        if exists(original)? {
            return Err(format!(
                "{}; the item stays in the Recycle Bin.",
                occupied(original)
            ));
        }
        let staging = self
            .stage(index, original)
            .ok_or("No staging folder for the restore.")?;
        // A new folder cannot collide, so the Shell never offers to overwrite.
        fs::create_dir(&staging).map_err(|e| {
            format!(
                "Create restore staging folder {}: {e}; the item stays in the Recycle Bin.",
                display_path(&staging)
            )
        })?;
        if let Err(error) = hooks.bin.restore(item, &staging, name) {
            if fs::remove_dir(&staging).is_err() {
                outcome.created.push(staging.clone());
                return Err(format!(
                    "Restore from the Recycle Bin failed: {}: {error}\nStaging folder kept: {}",
                    display_path(original),
                    display_path(&staging)
                ));
            }
            return Err(format!(
                "Restore from the Recycle Bin failed: {}: {error}",
                display_path(original)
            ));
        }
        let staged = staging.join(name);
        let kept = |reason: String, outcome: &mut Outcome| {
            outcome.created.push(staging.clone());
            Err(format!(
                "{reason}\nRestored item kept in staging: {}",
                display_path(&staged)
            ))
        };
        let verified = fs::symlink_metadata(&staged).is_ok_and(|metadata| {
            !files::reparse(&metadata)
                && metadata.is_dir() == item.directory
                && (item.directory || item.size.is_none_or(|size| size == metadata.len()))
        });
        if !verified {
            return kept(
                format!(
                    "Restored item could not be verified: {}",
                    display_path(original)
                ),
                outcome,
            );
        }
        if exists(original)? {
            return kept(occupied(original), outcome);
        }
        if let Err(error) = crate::platform::move_no_replace(&staged, original) {
            return kept(
                format!("Publishing the restored item failed: {error}"),
                outcome,
            );
        }
        fs::remove_dir(&staging).map_err(|e| {
            outcome.created.push(staging.clone());
            format!(
                "Restored {}, but the empty staging folder was kept: {}: {e}",
                display_path(original),
                display_path(&staging)
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_and_name_what_is_never_undoable() {
        for (kind, key, ..) in KINDS {
            assert_eq!(Kind::parse(key), Some(kind));
            assert_eq!(kind.key(), key);
        }
        assert_eq!(Kind::parse("unknown"), None);
        for (label, kind, refusal) in [
            ("Create ZIP", Kind::ZipCreate, None),
            ("Copy ZIP members to folder", Kind::ZipExtract, None),
            (
                "Move ZIP members to folder",
                Kind::ZipExtractMove,
                Some("Not undoable: move out of ZIP"),
            ),
            (
                "Copy files into ZIP",
                Kind::ZipInsert,
                Some("Not undoable: ZIP insert"),
            ),
            (
                "Move files into ZIP",
                Kind::ZipInsertMove,
                Some("Not undoable: ZIP insert"),
            ),
        ] {
            assert_eq!(Kind::archive(label), kind);
            assert_eq!(kind.refusal(), refusal);
        }
        assert_eq!(Kind::Save.refusal(), Some("Not undoable: source Save"));
        assert!(Kind::Undo.refusal().unwrap().contains("no redo"));
    }

    #[test]
    fn folder_marks_follow_descendants_but_not_the_folder_time() {
        let root =
            std::env::temp_dir().join(format!("tc-undo-mark-{}", changes::identity().unwrap()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("a.txt"), b"a").unwrap();
        let mark = Mark::read(&root).unwrap();
        assert_eq!(Mark::decode(&mark.encode()), Some(mark.clone()));
        assert!(Mark::decode("x:1:2:3:4:5").is_none() && Mark::decode("-").is_none());
        fs::write(root.join("temporary.txt"), b"t").unwrap();
        assert_ne!(
            Mark::read(&root).unwrap(),
            mark,
            "A new descendant is a change."
        );
        fs::remove_file(root.join("temporary.txt")).unwrap();
        assert_eq!(
            Mark::read(&root).unwrap(),
            mark,
            "Only the folder time moved."
        );
        fs::write(root.join("a.txt"), b"edited").unwrap();
        let edited = Mark::read(&root).unwrap();
        assert_ne!(edited, mark, "An edited descendant is a change.");
        assert!(edited.same_item(&mark), "Still the same folder.");
        fs::remove_file(root.join("a.txt")).unwrap();
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn records_parse_outcomes_and_oversized_undo_data_is_dropped() {
        let path = journal::encode_path(Path::new(r"C:\owned\a.txt"));
        let legacy = format!(
            "{}operation=Copy\nid=1\nsource={path}\n\ncancelled=false\ncompleted={path}\nfinished\n",
            journal::HEADER
        );
        let record = Record::parse("5-1-0".into(), &legacy);
        assert!(record.finished && record.changed && record.format.is_none());
        assert_eq!(
            status(&record, &Links::new()),
            Status::Refused("Not undoable (recorded by an older version)".into())
        );
        let effects = Effects {
            kind: Some(Kind::Recycle),
            recycled: vec![PathBuf::from(r"C:\owned\a.txt")],
            ..Default::default()
        };
        let lines = effects.journal_lines(u64::MAX);
        assert!(lines.contains("undo_recycled=") && !lines.contains("undo_unavailable"));
        let current = format!(
            "{}operation=Recycle\nid=2\nstarted=10\n\ncancelled=false\ncompleted={path}\n{lines}error=\"Stopped \\\"here\\\"\\nnext\"\nfinished\n",
            journal::HEADER
        );
        let record = Record::parse("6-1-0".into(), &current);
        assert_eq!(record.kind, Some(Kind::Recycle));
        assert_eq!(record.recycled, vec![PathBuf::from(r"C:\owned\a.txt")]);
        assert_eq!(record.error.as_deref(), Some("Stopped \"here\"\nnext"));
        assert_eq!(record.label(), "Delete: 1 item (partial)");
        let dropped = effects.journal_lines(8);
        assert!(
            dropped.contains("undo_unavailable=record size"),
            "{dropped}"
        );
        let record = Record::parse(
            "7-1-0".into(),
            &format!(
                "{}operation=Recycle\n\ncancelled=false\n{dropped}finished\n",
                journal::HEADER
            ),
        );
        assert_eq!(
            status(&record, &Links::new()),
            Status::Refused("Not undoable: record size".into())
        );
        assert_eq!(
            outermost(vec![
                PathBuf::from(r"C:\a\b"),
                PathBuf::from(r"C:\a"),
                PathBuf::from(r"C:\c"),
            ]),
            vec![PathBuf::from(r"C:\a"), PathBuf::from(r"C:\c")]
        );
        assert_eq!(
            place(&[Path::new(r"C:\x\a.txt"), Path::new(r"C:\x\Sub\b.txt")]),
            r"C:\x"
        );
        assert_eq!(
            place(&[Path::new(r"C:\x\a.txt"), Path::new(r"D:\y\b.txt")]),
            "several folders"
        );
    }

    #[test]
    fn cross_volume_copies_are_undoable_only_with_journaled_kept_originals() {
        let (original, copy) = (PathBuf::from(r"E:\Photos"), PathBuf::from(r"D:\Photos"));
        let effects = Effects {
            kind: Some(Kind::MoveAcrossKept),
            created: vec![copy.clone()],
            kept: vec![(original.clone(), copy.clone())],
            ..Default::default()
        };
        let lines = effects.journal_lines(u64::MAX);
        assert!(lines.starts_with("undo_format=2\n"), "{lines}");
        let encoded = |path: &Path| journal::encode_path(path);
        let kept_line = format!("undo_kept={} {} -\n", encoded(&original), encoded(&copy));
        assert!(lines.contains(&kept_line), "{lines}");
        let parse = |format: u8, kind: &str, outcome: &str, kept: bool| {
            Record::parse(
                "9-1-0".into(),
                &format!(
                    "{}operation=Move\nid=1\nstarted=10\n\ncancelled=false\ncompleted={}\n{outcome}undo_format={format}\nkind={kind}\nended=20\nundo_created={} d:1:2:0:0:0\n{}finished\n",
                    journal::HEADER,
                    encoded(&original),
                    encoded(&copy),
                    if kept { kept_line.as_str() } else { "" }
                ),
            )
        };
        let kept_outcome = format!("kept={}\n", encoded(&original));
        let refused = Status::Refused("Not undoable: kept originals not recorded".into());
        for (format, kind, outcome, kept, expected) in [
            (
                2,
                "move-across-kept",
                kept_outcome.as_str(),
                false,
                &refused,
            ),
            (
                1,
                "move-across-kept",
                kept_outcome.as_str(),
                false,
                &refused,
            ),
            (2, "move-across", kept_outcome.as_str(), false, &refused),
            (1, "move-across", "error=\"Stopped\"\n", false, &refused),
            (
                2,
                "move-across",
                "error=\"Stopped\"\n",
                false,
                &Status::Undoable,
            ),
            (1, "move-across", "", false, &Status::Undoable),
            (
                2,
                "move-across-kept",
                kept_outcome.as_str(),
                true,
                &Status::Undoable,
            ),
        ] {
            let record = parse(format, kind, outcome, kept);
            assert!(!record.damaged);
            assert_eq!(
                &status(&record, &Links::new()),
                expected,
                "{format} {kind} {outcome} {kept}"
            );
        }
        let record = parse(2, "move-across-kept", &kept_outcome, true);
        assert_eq!(record.kept, vec![(original, copy, None)]);
    }

    /// An empty bin that accepts every path.
    struct EmptyBin;

    impl RecycleBin for EmptyBin {
        fn items(&self, _: &[PathBuf], _: &AtomicBool) -> FileResult<Vec<BinItem>> {
            Ok(Vec::new())
        }

        fn restore(&self, _: &BinItem, _: &Path, _: &OsStr) -> FileResult<()> {
            Err("Nothing to restore.".into())
        }

        fn accepts(&self, _: &Path) -> bool {
            true
        }
    }

    #[test]
    fn operations_rows_never_walk_folders_and_preparing_finds_deep_changes() {
        let root =
            std::env::temp_dir().join(format!("tc-undo-rows-{}", changes::identity().unwrap()));
        fs::create_dir(&root).unwrap();
        let scope = Scope::fixture(&root).unwrap();
        let root = scope.resolve(&root).unwrap();
        let copy = root.join("Copy");
        let nested = copy.join("Nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("deep.txt"), b"deep").unwrap();
        let journal_root = root.join("Journal");
        fs::create_dir(&journal_root).unwrap();
        let record = journal_root.join("100-1-0.operation");
        let effects = Effects {
            kind: Some(Kind::Copy),
            created: vec![copy.clone()],
            ..Default::default()
        };
        fs::write(
            &record,
            format!(
                "{}operation=Copy\nid=1\nstarted={}\n\ncancelled=false\ncreated={}\n{}finished\n",
                journal::HEADER,
                now(),
                journal::encode_path(&copy),
                effects.journal_lines(u64::MAX)
            ),
        )
        .unwrap();
        let journal = Journal::new(journal_root.clone());
        let cancel = AtomicBool::new(false);
        let walked = || WALKED.with(std::cell::Cell::get);
        let row = |expected: Status| {
            let before = walked();
            let rows = history(&scope, &journal, HISTORY_LIMIT, &cancel).unwrap();
            assert_eq!(walked(), before, "Operations rows never walk descendants.");
            assert_eq!(rows[0].status, expected);
            rows[0].key.clone()
        };
        row(Status::Undoable);
        fs::write(nested.join("deep.txt"), b"edited deep content").unwrap();
        let key = row(Status::Undoable);
        let changed = format!("Changed since: {}", display_path(&copy));
        let before = walked();
        assert_eq!(
            prepare_with(&scope, &journal, 2, Some(&key), &cancel, &EmptyBin)
                .err()
                .unwrap(),
            changed,
            "Preparing finds the deep change and names the result."
        );
        assert!(walked() > before, "Preparing verifies every descendant.");
        let away = root.join("Away");
        fs::rename(&copy, &away).unwrap();
        fs::create_dir(&copy).unwrap();
        row(Status::Refused(changed));
        assert_eq!(
            history(&scope, &journal, HISTORY_LIMIT, &AtomicBool::new(true))
                .err()
                .unwrap(),
            HISTORY_CANCELLED
        );
        fs::remove_file(away.join("Nested").join("deep.txt")).unwrap();
        fs::remove_dir(away.join("Nested")).unwrap();
        fs::remove_dir(&away).unwrap();
        fs::remove_dir(&copy).unwrap();
        fs::remove_file(&record).unwrap();
        fs::remove_dir(&journal_root).unwrap();
        fs::remove_dir(&root).unwrap();
    }
}
