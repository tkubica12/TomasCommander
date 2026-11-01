use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::SystemTime,
};

pub type FileResult<T> = Result<T, String>;

pub fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    #[cfg(windows)]
    {
        if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
            return format!("\\\\{unc}");
        }
        if let Some(local) = text.strip_prefix("\\\\?\\")
            && local.as_bytes().get(1) == Some(&b':')
            && local.as_bytes().get(2) == Some(&b'\\')
        {
            return local.to_owned();
        }
    }
    text
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
    pub link: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub folded: String,
    pub aggregate: Option<FileResult<DirectoryStats>>,
}

#[derive(Clone, Debug, Default)]
pub struct DirectoryStats {
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub examined: usize,
    pub skipped_links: usize,
    pub warnings: Vec<String>,
    pub truncated: bool,
}

impl DirectoryStats {
    pub fn partial(&self) -> bool {
        self.truncated || !self.warnings.is_empty()
    }
}

pub fn directory_stats(
    scope: &Scope,
    path: &Path,
    stale: impl Fn() -> bool,
) -> FileResult<DirectoryStats> {
    let root = scope.resolve(path)?;
    let identity = fingerprint(&root)?;
    let mut stats = DirectoryStats::default();
    let mut stack = vec![root.clone()];
    while let Some(folder) = stack.pop() {
        if stale() {
            return Err("Folder calculation was superseded.".into());
        }
        if stats.examined >= 100_000 || stats.warnings.len() >= 100 {
            stats.truncated = true;
            break;
        }
        let inspected =
            fs::symlink_metadata(&folder).map_err(|e| io_error("Inspect folder", &folder, e));
        match inspected {
            Ok(metadata) if reparse(&metadata) => {
                stats.skipped_links += 1;
                continue;
            }
            Err(error) => {
                stats.warnings.push(error);
                continue;
            }
            _ => (),
        }
        match scope.resolve(&folder) {
            Ok(resolved) if resolved == folder => (),
            Ok(_) => {
                stats
                    .warnings
                    .push(format!("Folder changed: {}", display_path(&folder)));
                continue;
            }
            Err(error) => {
                stats.warnings.push(error);
                continue;
            }
        }
        let children = match fs::read_dir(&folder) {
            Ok(children) => children,
            Err(error) => {
                stats.warnings.push(io_error("Read folder", &folder, error));
                continue;
            }
        };
        for child in children {
            if stale() {
                return Err("Folder calculation was superseded.".into());
            }
            if stats.examined >= 100_000 || stats.warnings.len() >= 100 {
                stats.truncated = true;
                break;
            }
            stats.examined += 1;
            let inspected = (|| -> FileResult<()> {
                let child = child.map_err(|e| io_error("Read item", &folder, e))?;
                let path = child.path();
                let metadata =
                    fs::symlink_metadata(&path).map_err(|e| io_error("Inspect item", &path, e))?;
                if reparse(&metadata) {
                    stats.skipped_links += 1;
                } else if metadata.is_dir() {
                    stack.push(path);
                } else if metadata.is_file() {
                    stats.size = stats
                        .size
                        .checked_add(metadata.len())
                        .ok_or("Folder size exceeds the supported range.")?;
                    let modified = metadata
                        .modified()
                        .map_err(|e| io_error("Read item time", &path, e))?;
                    stats.modified = Some(
                        stats
                            .modified
                            .map_or(modified, |latest| latest.max(modified)),
                    );
                }
                Ok(())
            })();
            if let Err(error) = inspected {
                stats.warnings.push(error);
            }
        }
        if stats.truncated {
            break;
        }
    }
    if fingerprint(&root)? != identity {
        return Err("Folder changed during calculation; refresh to calculate it again.".into());
    }
    Ok(stats)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Name,
    Size,
    Modified,
}

#[derive(Clone, Debug)]
pub struct Scope {
    root: Option<PathBuf>,
}

fn io_error(action: &str, path: &Path, error: impl std::fmt::Display) -> String {
    format!("{action} {}: {error}", display_path(path))
}

pub fn reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

impl Scope {
    pub fn unrestricted() -> Self {
        Self { root: None }
    }

    pub fn fixture(root: &Path) -> FileResult<Self> {
        let root = fs::canonicalize(root).map_err(|e| io_error("Resolve fixture root", root, e))?;
        Ok(Self { root: Some(root) })
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn resolve(&self, path: &Path) -> FileResult<PathBuf> {
        self.resolve_folder(path).map_err(|failure| failure.message)
    }

    fn resolve_folder(&self, path: &Path) -> Result<PathBuf, ListFailure> {
        let resolved = fs::canonicalize(path).map_err(|e| ListFailure::io("Resolve", path, e))?;
        if let Some(root) = &self.root
            && !resolved.starts_with(root)
        {
            return Err(ListFailure {
                message: format!("Outside the fixture boundary: {}", display_path(path)),
                unavailable: true,
            });
        }
        Ok(resolved)
    }

    pub fn mutation_source(&self, path: &Path) -> FileResult<PathBuf> {
        let metadata = fs::symlink_metadata(path).map_err(|e| io_error("Inspect", path, e))?;
        if reparse(&metadata) {
            return Err(format!(
                "Links/reparse points cannot be modified: {}",
                display_path(path)
            ));
        }
        let resolved = self.resolve(path)?;
        if self.root.as_ref() == Some(&resolved) {
            return Err("The fixture root itself cannot be modified.".into());
        }
        Ok(resolved)
    }
}

#[derive(Debug)]
pub struct Listing {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    pub warnings: Vec<String>,
}

pub fn list_directory(scope: &Scope, path: &Path) -> FileResult<Listing> {
    list_directory_cancellable(scope, path, || false)
}

pub fn list_directory_cancellable(
    scope: &Scope,
    path: &Path,
    stale: impl Fn() -> bool,
) -> FileResult<Listing> {
    read_listing(scope, path, &stale).map_err(|failure| failure.message)
}

/// [`list_directory_cancellable`] for reopening a remembered folder (an activated tab, a
/// Back/Forward step or a restored tab). When `path` no longer exists or cannot be reached
/// (deleted, unplugged drive, offline share), lists its nearest listable ancestor inside the
/// scope instead and also returns `path`; ancestors of a fixture root are never tried. Denied
/// access keeps its error. Each attempt may wait for a network timeout, so call it on a worker.
pub fn list_directory_or_ancestor(
    scope: &Scope,
    path: &Path,
    stale: impl Fn() -> bool,
) -> FileResult<(Listing, Option<PathBuf>)> {
    let failure = match read_listing(scope, path, &stale) {
        Ok(listing) => return Ok((listing, None)),
        Err(failure) => failure,
    };
    if failure.unavailable {
        for ancestor in path.ancestors().skip(1) {
            if stale()
                || scope
                    .root()
                    .is_some_and(|root| root != ancestor && root.starts_with(ancestor))
            {
                break;
            }
            if let Ok(listing) = read_listing(scope, ancestor, &stale) {
                return Ok((listing, Some(path.to_owned())));
            }
        }
    }
    Err(failure.message)
}

/// A failed folder listing. `unavailable` when the folder is missing or unreachable rather than
/// access-denied or superseded.
struct ListFailure {
    message: String,
    unavailable: bool,
}

impl ListFailure {
    fn io(action: &str, path: &Path, error: std::io::Error) -> Self {
        Self {
            unavailable: error.kind() != std::io::ErrorKind::PermissionDenied,
            message: io_error(action, path, error),
        }
    }
}

fn read_listing(
    scope: &Scope,
    path: &Path,
    stale: &impl Fn() -> bool,
) -> Result<Listing, ListFailure> {
    let path = scope.resolve_folder(path)?;
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    for result in fs::read_dir(&path).map_err(|e| ListFailure::io("Read folder", &path, e))? {
        if stale() {
            return Err(ListFailure {
                message: "Directory scan was superseded.".into(),
                unavailable: false,
            });
        }
        match result {
            Ok(item) => match fs::symlink_metadata(item.path()) {
                Ok(metadata) => {
                    let name = item.file_name().to_string_lossy().into_owned();
                    entries.push(Entry {
                        path: item.path(),
                        folded: name.to_lowercase(),
                        name,
                        directory: metadata.is_dir(),
                        link: reparse(&metadata),
                        size: metadata.len(),
                        aggregate: if reparse(&metadata) && metadata.is_dir() {
                            Some(Err("Link folders are not traversed.".into()))
                        } else {
                            None
                        },
                        modified: match metadata.modified() {
                            Ok(time) => Some(time),
                            Err(error) => {
                                warnings.push(io_error(
                                    "Read modification time",
                                    &item.path(),
                                    error,
                                ));
                                None
                            }
                        },
                    });
                }
                Err(error) => warnings.push(io_error("Inspect item", &item.path(), error)),
            },
            Err(error) => warnings.push(io_error("Read item in", &path, error)),
        }
    }
    sort_entries(&mut entries, Sort::Name);
    Ok(Listing {
        path,
        entries,
        warnings,
    })
}

pub fn sort_entries(entries: &mut [Entry], sort: Sort) {
    entries.sort_by(|a, b| {
        b.directory.cmp(&a.directory).then_with(|| match sort {
            Sort::Name => a.folded.cmp(&b.folded),
            Sort::Size => b.size.cmp(&a.size).then(a.folded.cmp(&b.folded)),
            Sort::Modified => b.modified.cmp(&a.modified).then(a.folded.cmp(&b.folded)),
        })
    });
}

pub fn sort_entries_ordered(entries: &mut [Entry], sort: Sort, descending: bool) {
    let value = |entry: &Entry| {
        if entry.directory {
            entry
                .aggregate
                .as_ref()
                .and_then(|result| result.as_ref().ok())
                .map(|stats| (stats.size, stats.modified))
        } else {
            Some((entry.size, entry.modified))
        }
    };
    entries.sort_by(|a, b| {
        let order = match sort {
            Sort::Name => {
                let names = a.folded.cmp(&b.folded).then(a.path.cmp(&b.path));
                return b.directory.cmp(&a.directory).then(if descending {
                    names.reverse()
                } else {
                    names
                });
            }
            Sort::Size | Sort::Modified => match (value(a), value(b)) {
                (None, Some(_)) => return std::cmp::Ordering::Greater,
                (Some(_), None) => return std::cmp::Ordering::Less,
                (Some(a), Some(b)) => {
                    if sort == Sort::Size {
                        a.0.cmp(&b.0)
                    } else {
                        a.1.cmp(&b.1)
                    }
                }
                (None, None) => std::cmp::Ordering::Equal,
            },
        };
        (if descending { order.reverse() } else { order })
            .then(a.folded.cmp(&b.folded))
            .then(a.path.cmp(&b.path))
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Copy,
    Move,
    Recycle,
}

impl Operation {
    pub fn name(self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::Move => "Move",
            Self::Recycle => "Delete",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    pub(crate) directory: bool,
    pub(crate) size: u64,
    pub(crate) modified: SystemTime,
    pub(crate) created: SystemTime,
    #[cfg(windows)]
    pub(crate) file_id: (u32, u64),
}

pub fn fingerprint(path: &Path) -> FileResult<Fingerprint> {
    let meta = fs::symlink_metadata(path).map_err(|e| io_error("Inspect", path, e))?;
    if reparse(&meta) || !(meta.is_file() || meta.is_dir()) {
        return Err(format!(
            "Unsupported link or special item: {}",
            display_path(path)
        ));
    }
    Ok(Fingerprint {
        directory: meta.is_dir(),
        size: meta.len(),
        modified: meta
            .modified()
            .map_err(|e| io_error("Read modification time", path, e))?,
        created: meta
            .created()
            .map_err(|e| io_error("Read creation time", path, e))?,
        #[cfg(windows)]
        file_id: crate::platform::path_identity(path)?,
    })
}

/// How Copy/Move resolve items that already exist in the destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// Legacy behavior for non-interactive callers: any existing target fails planning.
    Fail,
    Skip,
    ReplaceOlder,
    ReplaceAll,
    KeepBoth,
}

impl ConflictPolicy {
    pub const CHOICES: [Self; 4] = [
        Self::Skip,
        Self::ReplaceOlder,
        Self::ReplaceAll,
        Self::KeepBoth,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fail => "Stop on conflict",
            Self::Skip => "Skip",
            Self::ReplaceOlder => "Replace older",
            Self::ReplaceAll => "Replace all",
            Self::KeepBoth => "Keep both",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Fail => "fail",
            Self::Skip => "skip",
            Self::ReplaceOlder => "replace-older",
            Self::ReplaceAll => "replace-all",
            Self::KeepBoth => "keep-both",
        }
    }
}

/// How an approved Move reaches its destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveStrategy {
    /// Identity-verified handle rename on one volume.
    SameVolume,
    /// Copy and CRC-32-verify on the destination volume, then send the originals to the
    /// Recycle Bin, or keep them when the source drive has no Recycle Bin.
    CrossVolume { recycle_originals: bool },
}

impl MoveStrategy {
    pub fn cross_volume(self) -> bool {
        matches!(self, Self::CrossVolume { .. })
    }

    fn key(self) -> &'static str {
        match self {
            Self::SameVolume => "same-volume-rename",
            Self::CrossVolume {
                recycle_originals: true,
            } => "cross-volume-copy-verify-recycle",
            Self::CrossVolume {
                recycle_originals: false,
            } => "cross-volume-copy-verify-keep",
        }
    }

    /// One technical line for review details.
    pub fn detail(self) -> &'static str {
        match self {
            Self::SameVolume => "Strategy: same-drive rename; nothing is copied.",
            Self::CrossVolume {
                recycle_originals: true,
            } => {
                "Strategy: across drives / copy each file, verify its CRC-32 by re-reading it, recheck the sources, then send the originals to the Recycle Bin."
            }
            Self::CrossVolume {
                recycle_originals: false,
            } => {
                "Strategy: across drives / copy each file and verify its CRC-32 by re-reading it. The source drive has no Recycle Bin, so originals are kept."
            }
        }
    }
}

/// Planning switches beyond the conflict policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanOptions {
    /// Refuse a Move between volumes instead of planning copy, verify and recycle.
    pub same_volume_only: bool,
    /// Test seam: plan a same-volume Move with the cross-volume strategy.
    #[doc(hidden)]
    pub force_cross_volume: bool,
    /// Test seam: replaces the source-drive Recycle Bin probe.
    #[doc(hidden)]
    pub recycle_bin: Option<bool>,
}

/// Effect seams for tests; `Hooks::default()` uses the real Recycle Bin.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct Hooks<'a> {
    /// Sends the items to the Recycle Bin as one batch; must never delete permanently.
    pub recycle: &'a dyn Fn(&[PathBuf]) -> FileResult<()>,
    /// Runs after a verified-copy target is flushed and closed, before it is re-read.
    pub copied: &'a dyn Fn(&Path, &Path),
}

fn no_copy_hook(_source: &Path, _target: &Path) {}

impl Default for Hooks<'static> {
    fn default() -> Self {
        Self {
            recycle: &crate::platform::recycle_many,
            copied: &no_copy_hook,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resolution {
    Create,
    Merge,
    Replace,
    Skip,
    Rename,
}

impl Resolution {
    fn key(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Merge => "merge",
            Self::Replace => "replace",
            Self::Skip => "skip",
            Self::Rename => "rename",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConflictKind {
    Folder,
    Newer,
    OlderOrSame,
    Mismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Existing {
    path: PathBuf,
    identity: Fingerprint,
    kind: ConflictKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PlannedNode {
    source: PathBuf,
    target: Option<PathBuf>,
    identity: Fingerprint,
    resolution: Resolution,
    existing: Option<Existing>,
}

/// Counts shown when a plan meets existing destination items.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConflictSummary {
    pub conflicts: usize,
    pub new_items: usize,
    pub merged: usize,
    pub newer: usize,
    pub older_or_same: usize,
    pub mismatched: usize,
    pub replace: usize,
    pub skip: usize,
    pub rename: usize,
    /// Destination-relative names of conflicts other than merged folders, in plan order.
    pub examples: Vec<PathBuf>,
}

impl ConflictSummary {
    pub fn any(&self) -> bool {
        self.conflicts > 0
    }
}

#[derive(Clone, Debug)]
pub struct Plan {
    id: u64,
    operation: Operation,
    policy: ConflictPolicy,
    sources: Vec<PathBuf>,
    destination: Option<PathBuf>,
    target_name: Option<String>,
    nodes: Vec<PlannedNode>,
    destination_identity: Option<Fingerprint>,
    strategy: Option<MoveStrategy>,
    options: PlanOptions,
    execution_key: String,
}

impl Plan {
    #[cfg(feature = "native-desktop-tests")]
    pub(crate) fn desktop_test_stale_snapshot(&self, destination: bool) -> FileResult<Self> {
        let mut plan = self.clone();
        let identity = if destination {
            plan.destination_identity.as_mut()
        } else {
            plan.nodes.first_mut().map(|node| &mut node.identity)
        }
        .ok_or("Desktop rejection probe requires a source and destination snapshot.")?;
        identity.size = identity.size.wrapping_add(1);
        Ok(plan)
    }

    #[cfg(feature = "native-desktop-tests")]
    pub(crate) fn desktop_test_existing_target(&self) -> FileResult<Self> {
        let mut plan = self.clone();
        let node = plan
            .nodes
            .first_mut()
            .ok_or("Desktop target rejection probe requires a planned node.")?;
        node.target = Some(node.source.clone());
        Ok(plan)
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn operation(&self) -> Operation {
        self.operation
    }

    pub fn policy(&self) -> ConflictPolicy {
        self.policy
    }

    pub fn sources(&self) -> &[PathBuf] {
        &self.sources
    }

    pub fn destination(&self) -> Option<&Path> {
        self.destination.as_deref()
    }

    /// The Move strategy recorded at planning; `None` for Copy and Recycle.
    pub fn move_strategy(&self) -> Option<MoveStrategy> {
        self.strategy
    }

    /// Copy and cross-volume Move stream file contents into the destination.
    fn streams(&self) -> bool {
        self.operation == Operation::Copy || self.strategy.is_some_and(MoveStrategy::cross_volume)
    }

    pub(crate) fn execution_key(&self) -> &str {
        &self.execution_key
    }

    fn relative(&self, path: &Path) -> PathBuf {
        self.destination
            .as_deref()
            .and_then(|destination| path.strip_prefix(destination).ok())
            .unwrap_or(path)
            .to_path_buf()
    }

    pub fn conflicts(&self) -> ConflictSummary {
        let mut summary = ConflictSummary::default();
        for node in &self.nodes {
            match node.resolution {
                Resolution::Create | Resolution::Rename if node.target.is_some() => {
                    summary.new_items += 1
                }
                _ => (),
            }
            match node.resolution {
                Resolution::Replace => summary.replace += 1,
                Resolution::Skip => summary.skip += 1,
                Resolution::Rename => summary.rename += 1,
                _ => (),
            }
            let Some(existing) = &node.existing else {
                continue;
            };
            summary.conflicts += 1;
            match existing.kind {
                ConflictKind::Newer => summary.newer += 1,
                ConflictKind::OlderOrSame => summary.older_or_same += 1,
                ConflictKind::Mismatch => summary.mismatched += 1,
                ConflictKind::Folder => (),
            }
            if node.resolution == Resolution::Merge {
                summary.merged += 1;
            } else {
                summary.examples.push(self.relative(&existing.path));
            }
        }
        summary
    }

    /// One line per existing destination item and its planned resolution.
    pub fn conflict_rows(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter_map(|node| {
                let existing = node.existing.as_ref()?;
                let name = display_path(&self.relative(&existing.path));
                let detail = match existing.kind {
                    ConflictKind::Folder => "",
                    ConflictKind::Newer => " (source newer)",
                    ConflictKind::OlderOrSame => " (source older or same)",
                    ConflictKind::Mismatch => " (file/folder type differs)",
                };
                Some(match node.resolution {
                    Resolution::Merge => format!("Merge folder: {name}"),
                    Resolution::Replace => format!("Replace: {name}{detail}"),
                    Resolution::Skip => format!("Skip: {name}{detail}"),
                    Resolution::Rename => format!(
                        "Keep both: {name}{detail} -> {}",
                        node.target
                            .as_deref()
                            .and_then(Path::file_name)
                            .map(|name| name.to_string_lossy())
                            .unwrap_or_default()
                    ),
                    Resolution::Create => format!("Create: {name}"),
                })
            })
            .collect()
    }

    fn stage_path(&self, index: usize) -> Option<PathBuf> {
        let node = self.nodes.get(index)?;
        if !self.streams() || node.resolution != Resolution::Replace {
            return None;
        }
        Some(node.target.as_ref()?.parent()?.join(format!(
            ".tomas-commander-{}-{index}.stage",
            self.execution_key
        )))
    }

    #[cfg(windows)]
    fn parent_identity(&self, target: &Path) -> FileResult<(u32, u64)> {
        let parent = target.parent().ok_or("Target has no parent folder.")?;
        if Some(parent) == self.destination.as_deref() {
            return Ok(self
                .destination_identity
                .as_ref()
                .ok_or("Approved move destination identity missing.")?
                .file_id);
        }
        self.nodes
            .iter()
            .find(|node| {
                node.resolution == Resolution::Merge && node.target.as_deref() == Some(parent)
            })
            .and_then(|node| node.existing.as_ref())
            .map(|existing| existing.identity.file_id)
            .ok_or_else(|| "Approved merged destination identity missing.".into())
    }

    pub(crate) fn journal_snapshot(&self) -> String {
        let mut snapshot = format!(
            "operation={:?}\nid={}\npolicy={}\n",
            self.operation,
            self.id,
            self.policy.key()
        );
        if let Some(strategy) = self.strategy {
            snapshot.push_str(&format!("strategy={}\n", strategy.key()));
        }
        if let Some(destination) = &self.destination {
            snapshot.push_str(&format!(
                "destination={}\ndestination_identity={:?}\n",
                crate::journal::encode_path(destination),
                self.destination_identity
            ));
        }
        for (index, node) in self.nodes.iter().enumerate() {
            snapshot.push_str(&format!(
                "source={}\nidentity={:?}\n",
                crate::journal::encode_path(&node.source),
                node.identity
            ));
            if let Some(target) = &node.target {
                snapshot.push_str(&format!(
                    "target={}\nresolution={}\n",
                    crate::journal::encode_path(target),
                    node.resolution.key()
                ));
            }
            if let Some(existing) = &node.existing {
                snapshot.push_str(&format!(
                    "conflict={}\nconflict_identity={:?}\n",
                    crate::journal::encode_path(&existing.path),
                    existing.identity
                ));
            }
            if let Some(stage) = self.stage_path(index) {
                snapshot.push_str(&format!("stage={}\n", crate::journal::encode_path(&stage)));
            }
        }
        snapshot
    }

    pub(crate) fn affected_paths(&self) -> Vec<PathBuf> {
        self.sources
            .iter()
            .cloned()
            .chain(self.nodes.iter().filter_map(|node| node.target.clone()))
            .collect()
    }
}

pub fn plan(
    scope: &Scope,
    id: u64,
    operation: Operation,
    sources: &[PathBuf],
    destination: Option<&Path>,
) -> FileResult<Plan> {
    plan_cancellable(
        scope,
        id,
        operation,
        sources,
        destination,
        &AtomicBool::new(false),
    )
}

pub fn plan_cancellable(
    scope: &Scope,
    id: u64,
    operation: Operation,
    sources: &[PathBuf],
    destination: Option<&Path>,
    cancelled: &AtomicBool,
) -> FileResult<Plan> {
    plan_with_policy(
        scope,
        id,
        operation,
        sources,
        destination,
        ConflictPolicy::Fail,
        cancelled,
    )
}

fn folded(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn resolve_target(
    scope: &Scope,
    source: &Path,
    identity: &Fingerprint,
    target: PathBuf,
    policy: ConflictPolicy,
    reserved: &mut BTreeSet<String>,
) -> FileResult<(Resolution, Option<PathBuf>, Option<Existing>)> {
    let metadata = match fs::symlink_metadata(&target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Resolution::Create, Some(target), None));
        }
        Err(error) => return Err(io_error("Inspect destination", &target, error)),
    };
    if policy == ConflictPolicy::Fail {
        return Err(format!(
            "Conflict; no overwrite allowed: {}",
            display_path(&target)
        ));
    }
    if reparse(&metadata) || !(metadata.is_file() || metadata.is_dir()) {
        return Err(format!(
            "Existing destination is a link or special item; resolve it manually: {}",
            display_path(&target)
        ));
    }
    // The existing item's actual name keeps merged child paths canonical.
    let actual = scope.resolve(&target)?;
    if actual.parent() != target.parent() {
        return Err(format!(
            "Destination changed while planning: {}",
            display_path(&target)
        ));
    }
    let existing = fingerprint(&actual)?;
    #[cfg(windows)]
    let same = actual == source || existing.file_id == identity.file_id;
    #[cfg(not(windows))]
    let same = actual == source;
    let kind = match (identity.directory, existing.directory) {
        (true, true) => ConflictKind::Folder,
        (false, false) if identity.modified > existing.modified => ConflictKind::Newer,
        (false, false) => ConflictKind::OlderOrSame,
        _ => ConflictKind::Mismatch,
    };
    let refused = |reason: &str| {
        Err(format!(
            "{reason}: {}. Choose Skip or Keep both.",
            display_path(&actual)
        ))
    };
    let resolution = match (policy, kind) {
        (ConflictPolicy::Fail, _) => {
            return Err(format!(
                "Conflict; no overwrite allowed: {}",
                display_path(&actual)
            ));
        }
        (ConflictPolicy::Skip, ConflictKind::Folder) if same => Resolution::Skip,
        (ConflictPolicy::KeepBoth, ConflictKind::Folder) if same => Resolution::Rename,
        (_, ConflictKind::Folder) if same => {
            return refused("Source and destination are the same folder");
        }
        (_, ConflictKind::Folder) => Resolution::Merge,
        (ConflictPolicy::Skip, _) => Resolution::Skip,
        (ConflictPolicy::KeepBoth, _) => Resolution::Rename,
        (_, ConflictKind::Mismatch) if identity.directory => {
            return refused("A file cannot be replaced with a folder");
        }
        (_, ConflictKind::Mismatch) => return refused("A folder cannot be replaced with a file"),
        (_, _) if same => return refused("Source and destination are the same file"),
        (ConflictPolicy::ReplaceOlder, ConflictKind::Newer) | (ConflictPolicy::ReplaceAll, _) => {
            Resolution::Replace
        }
        (ConflictPolicy::ReplaceOlder, _) => Resolution::Skip,
    };
    let target = if resolution == Resolution::Rename {
        unique_target(&target, identity.directory, reserved)?
    } else {
        actual.clone()
    };
    Ok((
        resolution,
        Some(target),
        Some(Existing {
            path: actual,
            identity: existing,
            kind,
        }),
    ))
}

/// First free `name (n).ext`, unique on disk and against other planned targets.
fn unique_target(
    target: &Path,
    directory: bool,
    reserved: &mut BTreeSet<String>,
) -> FileResult<PathBuf> {
    let parent = target.parent().ok_or("Target has no parent folder.")?;
    let name = target.file_name().ok_or("Target has no name.")?;
    let (stem, extension) = match (directory, Path::new(name).file_stem()) {
        (false, Some(stem)) => (stem, Path::new(name).extension()),
        _ => (name, None),
    };
    for number in 2..10_000 {
        let mut candidate = std::ffi::OsString::from(stem);
        candidate.push(format!(" ({number})"));
        if let Some(extension) = extension {
            candidate.push(".");
            candidate.push(extension);
        }
        let candidate = parent.join(candidate);
        let key = folded(&candidate);
        if reserved.contains(&key) {
            continue;
        }
        match fs::symlink_metadata(&candidate) {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                reserved.insert(key);
                return Ok(candidate);
            }
            Err(error) => return Err(io_error("Inspect destination", &candidate, error)),
        }
    }
    Err(format!(
        "No free Keep both name for {}",
        display_path(target)
    ))
}

pub fn plan_with_policy(
    scope: &Scope,
    id: u64,
    operation: Operation,
    sources: &[PathBuf],
    destination: Option<&Path>,
    policy: ConflictPolicy,
    cancelled: &AtomicBool,
) -> FileResult<Plan> {
    plan_with_options(
        scope,
        id,
        operation,
        sources,
        destination,
        policy,
        PlanOptions::default(),
        cancelled,
    )
}

/// Chooses rename or copy-verify-recycle for a Move from the real volume identities.
fn move_strategy(
    sources: &[PathBuf],
    destination: &Path,
    options: PlanOptions,
) -> FileResult<MoveStrategy> {
    #[cfg(windows)]
    let across = options.force_cross_volume || {
        // Serial plus volume root: shares or cloned volumes may repeat a serial.
        let volume = |path: &Path| -> FileResult<(u32, Option<String>)> {
            Ok((
                crate::platform::path_identity(path)?.0,
                crate::platform::volume_root(path).ok(),
            ))
        };
        let target = volume(destination)?;
        let mut across = false;
        for source in sources {
            let source = volume(source)?;
            across |= source.0 != target.0
                || matches!((&source.1, &target.1), (Some(a), Some(b)) if a != b);
        }
        across
    };
    #[cfg(not(windows))]
    let across = {
        let _ = (sources, destination);
        options.force_cross_volume
    };
    if !across {
        return Ok(MoveStrategy::SameVolume);
    }
    if options.same_volume_only {
        return Err(
            "AI command moves stay on one drive; use Move in the panes across drives. No plan."
                .into(),
        );
    }
    let recycle_originals = match options.recycle_bin {
        Some(available) => available,
        // An unanswered probe keeps the originals; it never risks permanent deletion.
        None => sources
            .iter()
            .all(|source| crate::platform::recycle_bin_available(source).unwrap_or(false)),
    };
    Ok(MoveStrategy::CrossVolume { recycle_originals })
}

#[allow(clippy::too_many_arguments)]
pub fn plan_with_options(
    scope: &Scope,
    id: u64,
    operation: Operation,
    sources: &[PathBuf],
    destination: Option<&Path>,
    policy: ConflictPolicy,
    options: PlanOptions,
    cancelled: &AtomicBool,
) -> FileResult<Plan> {
    plan_with_target_name(
        scope,
        id,
        operation,
        sources,
        destination,
        policy,
        options,
        cancelled,
        None,
    )
}

/// Copies one regular file to an exact new name in its current folder.
pub fn plan_clone(
    scope: &Scope,
    id: u64,
    source: &Path,
    name: &str,
    cancelled: &AtomicBool,
) -> FileResult<Plan> {
    crate::naming::validate_name(name)?;
    let source = scope.mutation_source(source)?;
    if fingerprint(&source)?.directory {
        return Err("Clone requires one regular file, not a folder.".into());
    }
    let folder = source.parent().ok_or("Clone source has no folder.")?;
    plan_with_target_name(
        scope,
        id,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(folder),
        ConflictPolicy::Fail,
        PlanOptions::default(),
        cancelled,
        Some(name),
    )
}

#[allow(clippy::too_many_arguments)]
fn plan_with_target_name(
    scope: &Scope,
    id: u64,
    operation: Operation,
    sources: &[PathBuf],
    destination: Option<&Path>,
    policy: ConflictPolicy,
    options: PlanOptions,
    cancelled: &AtomicBool,
    target_name: Option<&str>,
) -> FileResult<Plan> {
    if sources.is_empty() {
        return Err("Select or focus an item first.".into());
    }
    let policy = if operation == Operation::Recycle {
        ConflictPolicy::Fail
    } else {
        policy
    };
    let destination = if operation == Operation::Recycle {
        None
    } else {
        let destination = destination.ok_or("Choose a destination folder.")?;
        let resolved = scope.resolve(destination)?;
        if !resolved.is_dir() {
            return Err("Destination is not a folder.".into());
        }
        Some(resolved)
    };
    let sources: Vec<PathBuf> = sources
        .iter()
        .map(|path| scope.mutation_source(path))
        .collect::<FileResult<_>>()?;
    if sources.iter().any(|source| source.file_name().is_none()) {
        return Err("Filesystem roots cannot be copied, moved, or recycled.".into());
    }
    for (index, source) in sources.iter().enumerate() {
        if sources
            .iter()
            .enumerate()
            .any(|(other, path)| other != index && source.starts_with(path))
        {
            return Err("Do not select both a folder and its descendant.".into());
        }
    }
    let mut reserved = BTreeSet::new();
    if let Some(destination) = &destination {
        for source in &sources {
            if destination.starts_with(source) {
                return Err("A folder cannot be copied or moved into itself.".into());
            }
            let target =
                destination.join(target_name.map(std::ffi::OsStr::new).unwrap_or_else(|| {
                    source
                        .file_name()
                        .expect("Root sources were rejected above")
                }));
            if !reserved.insert(folded(&target)) {
                return Err("Multiple selected items have the same destination name.".into());
            }
        }
    }
    let mut nodes = Vec::new();
    for source in &sources {
        let target = destination.as_ref().map(|destination| {
            destination.join(target_name.map(std::ffi::OsStr::new).unwrap_or_else(|| {
                source
                    .file_name()
                    .expect("Root sources were rejected above")
            }))
        });
        // Children of a newly created or renamed folder cannot conflict.
        let mut stack = vec![(source.clone(), target, false)];
        while let Some((source, target, fresh)) = stack.pop() {
            if cancelled.load(Ordering::Relaxed) {
                return Err("Planning was cancelled.".into());
            }
            scope.mutation_source(&source)?;
            let identity = fingerprint(&source)?;
            let (resolution, target, existing) = match target {
                Some(target) if !fresh => {
                    resolve_target(scope, &source, &identity, target, policy, &mut reserved)?
                }
                target => (Resolution::Create, target, None),
            };
            if identity.directory && resolution != Resolution::Skip {
                let fresh = resolution != Resolution::Merge;
                for child in
                    fs::read_dir(&source).map_err(|e| io_error("Read folder", &source, e))?
                {
                    let child = child.map_err(|e| io_error("Read child", &source, e))?;
                    let child_target = target.as_ref().map(|path| path.join(child.file_name()));
                    if !fresh && let Some(path) = &child_target {
                        reserved.insert(folded(path));
                    }
                    stack.push((child.path(), child_target, fresh));
                }
            }
            nodes.push(PlannedNode {
                source,
                target,
                identity,
                resolution,
                existing,
            });
        }
    }
    nodes.sort_by(|a, b| {
        a.source
            .components()
            .count()
            .cmp(&b.source.components().count())
            .then(a.source.cmp(&b.source))
    });
    let destination_identity = destination.as_deref().map(fingerprint).transpose()?;
    let strategy = match (operation, &destination) {
        (Operation::Move, Some(destination)) => {
            Some(move_strategy(&sources, destination, options)?)
        }
        _ => None,
    };
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|e| format!("Create operation identity: {e}"))?
        .as_nanos();
    Ok(Plan {
        id,
        operation,
        policy,
        sources,
        destination,
        target_name: target_name.map(str::to_owned),
        nodes,
        destination_identity,
        strategy,
        options,
        execution_key: format!(
            "{timestamp}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ),
    })
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub id: u64,
    pub completed: Vec<PathBuf>,
    pub created: Vec<PathBuf>,
    pub error: Option<String>,
    pub cancelled: bool,
    pub incomplete: Vec<PathBuf>,
    /// Destination files whose previous versions were sent to the Recycle Bin.
    pub replaced: Vec<PathBuf>,
    /// Existing destination folders that received merged contents.
    pub merged: Vec<PathBuf>,
    /// Sources excluded by the approved conflict policy.
    pub skipped: Vec<PathBuf>,
    /// Cross-volume Move: verified originals sent to the Recycle Bin.
    pub recycled: Vec<PathBuf>,
    /// Cross-volume Move: originals kept although their verified copies exist.
    pub kept: Vec<PathBuf>,
    pub operation: Option<Operation>,
    pub strategy: Option<MoveStrategy>,
    /// What the operation proves it did, journaled for a later reviewed undo.
    pub effects: crate::undo::Effects,
}

impl Outcome {
    /// Counts such as "12 copied, 3 replaced (originals in Recycle Bin), 2 skipped".
    pub fn summary(&self) -> Option<String> {
        let across = self.strategy.is_some_and(MoveStrategy::cross_volume);
        let verb = match self.operation? {
            Operation::Copy => "copied",
            Operation::Move if across && self.recycled.is_empty() => {
                "copied across drives (verified)"
            }
            Operation::Move if across => "moved across drives (verified)",
            Operation::Move => "moved",
            Operation::Recycle => return None,
        };
        if !across && self.replaced.is_empty() && self.merged.is_empty() && self.skipped.is_empty()
        {
            return None;
        }
        let mut parts = vec![format!(
            "{} {verb}",
            self.completed
                .len()
                .saturating_sub(self.replaced.len() + self.merged.len())
        )];
        if !self.replaced.is_empty() {
            parts.push(format!(
                "{} replaced (originals in Recycle Bin)",
                self.replaced.len()
            ));
        }
        if !self.merged.is_empty() {
            parts.push(format!(
                "{} {} merged",
                self.merged.len(),
                if self.merged.len() == 1 {
                    "folder"
                } else {
                    "folders"
                }
            ));
        }
        if !self.skipped.is_empty() {
            parts.push(format!("{} skipped", self.skipped.len()));
        }
        if !self.recycled.is_empty() {
            parts.push("originals in Recycle Bin".into());
        }
        match self.kept.len() {
            0 => (),
            _ if self.recycled.is_empty() => parts.push("originals kept".into()),
            1 => parts.push("1 original kept".into()),
            kept => parts.push(format!("{kept} originals kept")),
        }
        Some(parts.join(", "))
    }
}

fn validate_plan(scope: &Scope, plan: &Plan, cancelled: &AtomicBool) -> FileResult<()> {
    if let Some(destination) = &plan.destination {
        let resolved = scope.resolve(destination)?;
        if resolved != *destination || Some(fingerprint(destination)?) != plan.destination_identity
        {
            return Err("Destination folder changed; create a new plan.".into());
        }
    }
    for node in &plan.nodes {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Validation cancelled; no further changes made.".into());
        }
        if scope.mutation_source(&node.source)? != node.source
            || fingerprint(&node.source)? != node.identity
        {
            return Err(format!(
                "Source changed; create a new plan: {}",
                display_path(&node.source)
            ));
        }
        if let Some(target) = &node.target
            && matches!(node.resolution, Resolution::Create | Resolution::Rename)
            && target
                .try_exists()
                .map_err(|e| io_error("Inspect destination", target, e))?
        {
            return Err(format!(
                "Destination changed; no overwrite allowed: {}",
                display_path(target)
            ));
        }
        if let Some(existing) = &node.existing
            && fingerprint(&existing.path).ok().as_ref() != Some(&existing.identity)
        {
            return Err(format!(
                "Destination changed; create a new plan: {}",
                display_path(&existing.path)
            ));
        }
    }
    // Re-enumeration detects added/removed children and new or resolved conflicts.
    let current = plan_with_target_name(
        scope,
        plan.id,
        plan.operation,
        &plan.sources,
        plan.destination.as_deref(),
        plan.policy,
        plan.options,
        cancelled,
        plan.target_name.as_deref(),
    )?;
    if current.nodes.len() != plan.nodes.len()
        || current
            .nodes
            .iter()
            .zip(&plan.nodes)
            .any(|(a, b)| a.source != b.source || a.identity != b.identity)
    {
        return Err("Source tree changed; create a new plan.".into());
    }
    if current.nodes != plan.nodes {
        return Err("Destination changed; create a new plan.".into());
    }
    if current.strategy != plan.strategy {
        return Err("Move strategy changed (drive or Recycle Bin); create a new plan.".into());
    }
    Ok(())
}

fn stream_copy(
    node: &PlannedNode,
    target: &Path,
    verify: bool,
    hooks: &Hooks,
    outcome: &mut Outcome,
    cancelled: &AtomicBool,
) -> FileResult<bool> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0020_0000);
    }
    let mut source = options
        .open(&node.source)
        .map_err(|e| io_error("Open source", &node.source, e))?;
    let meta = source
        .metadata()
        .map_err(|e| io_error("Inspect open source", &node.source, e))?;
    #[cfg(windows)]
    if crate::platform::file_identity(&source)? != node.identity.file_id {
        return Err("Source identity changed while opening it; copy stopped.".into());
    }
    if reparse(&meta)
        || meta.len() != node.identity.size
        || meta
            .modified()
            .map_err(|e| io_error("Read source time", &node.source, e))?
            != node.identity.modified
        || meta
            .created()
            .map_err(|e| io_error("Read source creation time", &node.source, e))?
            != node.identity.created
    {
        return Err("Source changed while opening it; copy stopped.".into());
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|e| io_error("Create destination without overwrite", target, e))?;
    outcome.created.push(target.to_owned());
    let mut checksum = crc32fast::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut finished = true;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            outcome.cancelled = true;
            finished = false;
            break;
        }
        let count = source
            .read(&mut buffer)
            .map_err(|e| io_error("Read source", &node.source, e))?;
        if count == 0 {
            break;
        }
        if verify {
            checksum.update(&buffer[..count]);
        }
        output
            .write_all(&buffer[..count])
            .map_err(|e| io_error("Write destination", target, e))?;
    }
    if finished {
        output
            .set_modified(node.identity.modified)
            .map_err(|e| io_error("Preserve modification time", target, e))?;
    }
    output
        .sync_all()
        .map_err(|e| io_error("Flush destination", target, e))?;
    if !finished {
        return Ok(false);
    }
    if output
        .metadata()
        .map_err(|e| io_error("Inspect copied file", target, e))?
        .len()
        != node.identity.size
    {
        return Err("Copied file length does not match the approved source.".into());
    }
    if !verify {
        return Ok(true);
    }
    drop(output);
    drop(source);
    (hooks.copied)(&node.source, target);
    let Some((length, actual)) = file_checksum(target, cancelled)? else {
        outcome.cancelled = true;
        return Ok(false);
    };
    if length != node.identity.size || actual != checksum.finalize() {
        return Err(format!(
            "Copy verification failed: the destination bytes differ from the source (CRC-32). The source was kept; inspect {}",
            display_path(target)
        ));
    }
    Ok(true)
}

/// Re-reads a written file; `None` when cancelled between chunks.
fn file_checksum(path: &Path, cancelled: &AtomicBool) -> FileResult<Option<(u64, u32)>> {
    let mut file = File::open(path).map_err(|e| io_error("Open copy for verification", path, e))?;
    let mut checksum = crc32fast::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut length = 0u64;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let count = file
            .read(&mut buffer)
            .map_err(|e| io_error("Verify copy", path, e))?;
        if count == 0 {
            return Ok(Some((length, checksum.finalize())));
        }
        checksum.update(&buffer[..count]);
        length += count as u64;
    }
}

fn recycle_replaced(target: &Path, hooks: &Hooks) -> FileResult<()> {
    (hooks.recycle)(&[target.to_owned()])?;
    if target
        .try_exists()
        .map_err(|e| io_error("Verify recycling", target, e))?
    {
        return Err(format!(
            "Windows did not recycle the replaced destination: {}",
            display_path(target)
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn lock_merged_folder(target: &Path, existing: &Fingerprint) -> FileResult<File> {
    use std::os::windows::fs::OpenOptionsExt;
    let handle = OpenOptions::new()
        .access_mode(0)
        .share_mode(3)
        .custom_flags(0x0220_0000)
        .open(target)
        .map_err(|e| io_error("Lock merged folder", target, e))?;
    let metadata = handle
        .metadata()
        .map_err(|e| io_error("Inspect merged folder", target, e))?;
    if reparse(&metadata)
        || !metadata.is_dir()
        || crate::platform::file_identity(&handle)? != existing.file_id
    {
        return Err(format!(
            "Merged destination folder changed; copy stopped: {}",
            display_path(target)
        ));
    }
    Ok(handle)
}

fn move_node(
    plan: &Plan,
    node: &PlannedNode,
    hooks: &Hooks,
    outcome: &mut Outcome,
) -> FileResult<()> {
    let target = node.target.as_ref().ok_or("Move destination is missing.")?;
    #[cfg(windows)]
    let parent = plan.parent_identity(target)?;
    #[cfg(not(windows))]
    let _ = plan;
    let replaced = node.resolution == Resolution::Replace;
    if replaced {
        let existing = node
            .existing
            .as_ref()
            .ok_or("Replace plan has no destination snapshot.")?;
        #[cfg(windows)]
        if existing.identity.file_id.0 != node.identity.file_id.0 {
            return Err(format!(
                "Source and destination are on different drives; create a new plan. Nothing was replaced: {}",
                display_path(target)
            ));
        }
        if fingerprint(target).ok().as_ref() != Some(&existing.identity) {
            return Err(format!(
                "Destination changed; create a new plan: {}",
                display_path(target)
            ));
        }
        recycle_replaced(target, hooks)?;
        outcome.replaced.push(target.clone());
    }
    #[cfg(windows)]
    let moved = crate::platform::move_verified(&node.source, target, &node.identity, parent);
    #[cfg(not(windows))]
    let moved = crate::platform::move_no_replace(&node.source, target);
    moved.map_err(|error| {
        if replaced {
            format!("{error}\nThe replaced destination is in the Recycle Bin.")
        } else {
            error
        }
    })?;
    outcome.created.push(target.clone());
    outcome.completed.push(node.source.clone());
    Ok(())
}

/// Handles that pin paths during a streamed copy, keyed by the pinned path.
pub(crate) type Guards = Vec<(PathBuf, File)>;

/// Copies plan nodes in order by exclusive creation or staged replacement; cross-volume Move
/// also verifies every file. Returns `false` when cancelled; completed copies are reported.
fn copy_nodes(
    scope: &Scope,
    plan: &Plan,
    indices: &[usize],
    hooks: &Hooks,
    #[cfg_attr(not(windows), allow(unused_variables))] guards: &mut Guards,
    outcome: &mut Outcome,
    cancelled: &AtomicBool,
) -> FileResult<bool> {
    let verify = plan.strategy.is_some_and(MoveStrategy::cross_volume);
    for &index in indices {
        let node = &plan.nodes[index];
        if cancelled.load(Ordering::Relaxed) {
            outcome.cancelled = true;
            return Ok(false);
        }
        if node.resolution == Resolution::Skip {
            outcome.skipped.push(node.source.clone());
            continue;
        }
        scope.mutation_source(&node.source)?;
        #[cfg(windows)]
        if let Some(destination) = &plan.destination
            && crate::platform::path_identity(destination)?
                != plan
                    .destination_identity
                    .as_ref()
                    .ok_or("Destination identity missing.")?
                    .file_id
        {
            return Err("Destination identity changed; copy stopped.".into());
        }
        if fingerprint(&node.source)? != node.identity {
            return Err(format!(
                "Source changed; copy stopped: {}",
                display_path(&node.source)
            ));
        }
        let target = node
            .target
            .as_ref()
            .ok_or("Copy plan has no destination.")?;
        let parent = target.parent().ok_or("Target has no parent folder.")?;
        if scope.resolve(parent)? != parent {
            return Err("Destination parent changed; copy stopped.".into());
        }
        match node.resolution {
            Resolution::Merge => {
                let existing = node
                    .existing
                    .as_ref()
                    .ok_or("Merge plan has no destination snapshot.")?;
                #[cfg(windows)]
                guards.push((
                    target.clone(),
                    lock_merged_folder(target, &existing.identity)?,
                ));
                #[cfg(not(windows))]
                if !fingerprint(target)?.directory || !existing.identity.directory {
                    return Err("Merged destination folder changed; copy stopped.".into());
                }
                outcome.merged.push(target.clone());
            }
            _ if node.identity.directory => {
                fs::create_dir(target).map_err(|e| io_error("Create folder", target, e))?;
                outcome.created.push(target.clone());
                #[cfg(windows)]
                {
                    use std::os::windows::fs::OpenOptionsExt;
                    let handle = OpenOptions::new()
                        .access_mode(0)
                        .share_mode(3)
                        .custom_flags(0x0220_0000)
                        .open(target)
                        .map_err(|e| io_error("Lock created folder", target, e))?;
                    if reparse(
                        &handle
                            .metadata()
                            .map_err(|e| io_error("Inspect created folder", target, e))?,
                    ) {
                        return Err("Created copy folder changed to a link; copy stopped.".into());
                    }
                    guards.push((target.clone(), handle));
                }
            }
            Resolution::Replace => {
                let stage = plan
                    .stage_path(index)
                    .ok_or("Replacement staging path missing.")?;
                if !stream_copy(node, &stage, verify, hooks, outcome, cancelled)? {
                    return Ok(false);
                }
                let existing = node
                    .existing
                    .as_ref()
                    .ok_or("Replace plan has no destination snapshot.")?;
                let retained = format!("staged copy retained: {}", display_path(&stage));
                if fingerprint(target).ok().as_ref() != Some(&existing.identity) {
                    return Err(format!(
                        "Destination changed before replacement; {retained}"
                    ));
                }
                recycle_replaced(target, hooks).map_err(|e| format!("{e}; {retained}"))?;
                outcome.replaced.push(target.clone());
                crate::platform::move_no_replace(&stage, target).map_err(|e| {
                    format!(
                        "Publishing the copy failed; the replaced original is in the Recycle Bin and the {retained}: {e}"
                    )
                })?;
                outcome.created.retain(|path| path != &stage);
                outcome.created.push(target.clone());
            }
            _ => {
                if !stream_copy(node, target, verify, hooks, outcome, cancelled)? {
                    return Ok(false);
                }
            }
        }
        outcome.completed.push(node.source.clone());
    }
    Ok(true)
}

/// Rechecks a copied top-level source against the plan, including folder membership.
fn sources_unchanged(scope: &Scope, plan: &Plan, members: &[usize]) -> FileResult<()> {
    let mut children = std::collections::BTreeMap::<&Path, usize>::new();
    for &index in members {
        if let Some(parent) = plan.nodes[index].source.parent() {
            *children.entry(parent).or_default() += 1;
        }
    }
    for &index in members {
        let node = &plan.nodes[index];
        if scope.mutation_source(&node.source)? != node.source
            || fingerprint(&node.source)? != node.identity
        {
            return Err(format!(
                "Source changed during the copy: {}",
                display_path(&node.source)
            ));
        }
        if node.identity.directory && node.resolution != Resolution::Skip {
            let count = fs::read_dir(&node.source)
                .map_err(|e| io_error("Recheck folder", &node.source, e))?
                .count();
            if children.get(node.source.as_path()).copied().unwrap_or(0) != count {
                return Err(format!(
                    "Source folder contents changed during the copy: {}",
                    display_path(&node.source)
                ));
            }
        }
    }
    Ok(())
}

/// Confirms every copy of a top-level source is still in place just before recycling it.
fn copies_present(plan: &Plan, members: &[usize]) -> FileResult<()> {
    for &index in members {
        let node = &plan.nodes[index];
        let Some(target) = node
            .target
            .as_ref()
            .filter(|_| node.resolution != Resolution::Skip)
        else {
            continue;
        };
        let present = fs::symlink_metadata(target).is_ok_and(|metadata| {
            !reparse(&metadata)
                && if node.identity.directory {
                    metadata.is_dir()
                } else {
                    metadata.is_file() && metadata.len() == node.identity.size
                }
        });
        if !present {
            return Err(format!(
                "Verified copy changed before recycling: {}",
                display_path(target)
            ));
        }
    }
    Ok(())
}

/// Cross-volume Move: copy and verify each top-level source, recheck it, then recycle it.
/// Originals are never deleted permanently; any doubt keeps them beside their verified copies.
fn move_across(
    scope: &Scope,
    plan: &Plan,
    recycle_originals: bool,
    hooks: &Hooks,
    guards: &mut Guards,
    outcome: &mut Outcome,
    cancelled: &AtomicBool,
) -> FileResult<()> {
    let resolutions: std::collections::BTreeMap<&Path, Resolution> = plan
        .nodes
        .iter()
        .map(|node| (node.source.as_path(), node.resolution))
        .collect();
    for source in &plan.sources {
        if cancelled.load(Ordering::Relaxed) {
            outcome.cancelled = true;
            return Ok(());
        }
        let members: Vec<usize> = (0..plan.nodes.len())
            .filter(|&index| plan.nodes[index].source.starts_with(source))
            .collect();
        let top = members
            .iter()
            .map(|&index| &plan.nodes[index])
            .find(|node| node.source == *source)
            .ok_or("Approved move source identity missing.")?;
        if top.resolution == Resolution::Skip {
            outcome.skipped.push(source.clone());
            continue;
        }
        let target = top.target.as_ref().ok_or("Move destination is missing.")?;
        if !copy_nodes(scope, plan, &members, hooks, guards, outcome, cancelled)? {
            return Ok(());
        }
        if !recycle_originals {
            outcome.kept.push(source.clone());
            continue;
        }
        // Windows must be able to remove the source tree; destination pins stay.
        guards.retain(|(path, _)| !path.starts_with(source));
        if cancelled.load(Ordering::Relaxed) {
            outcome.cancelled = true;
            outcome.kept.push(source.clone());
            return Ok(());
        }
        if let Err(error) =
            sources_unchanged(scope, plan, &members).and_then(|()| copies_present(plan, &members))
        {
            outcome.kept.push(source.clone());
            return Err(format!(
                "{error}\nMove stopped before recycling. The original was kept: {}\nVerified copy: {}",
                display_path(source),
                display_path(target)
            ));
        }
        let skipped_inside = members
            .iter()
            .any(|&index| plan.nodes[index].resolution == Resolution::Skip);
        // Without skipped items the whole source is one Recycle Bin entry.
        let units: Vec<PathBuf> = if skipped_inside {
            members
                .iter()
                .map(|&index| &plan.nodes[index])
                .filter(|node| {
                    !matches!(node.resolution, Resolution::Merge | Resolution::Skip)
                        && node
                            .source
                            .parent()
                            .and_then(|parent| resolutions.get(parent))
                            == Some(&Resolution::Merge)
                })
                .map(|node| node.source.clone())
                .collect()
        } else {
            vec![source.clone()]
        };
        let result = if units.is_empty() {
            Ok(())
        } else {
            (hooks.recycle)(&units)
        };
        let mut remaining = Vec::new();
        for unit in units {
            match fs::symlink_metadata(&unit) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    outcome.recycled.push(unit)
                }
                _ => remaining.push(unit),
            }
        }
        if result.is_err() || !remaining.is_empty() {
            let reason = result
                .err()
                .unwrap_or_else(|| "Windows did not remove every original.".into());
            let count = remaining.len();
            outcome.kept.extend(remaining);
            return Err(format!(
                "Recycling originals failed; {count} {} kept and every verified copy remains in {}: {reason}",
                if count == 1 {
                    "original was"
                } else {
                    "originals were"
                },
                display_path(plan.destination.as_deref().unwrap_or(target))
            ));
        }
        if !skipped_inside {
            continue;
        }
        let mut merged: Vec<&PlannedNode> = members
            .iter()
            .map(|&index| &plan.nodes[index])
            .filter(|node| node.resolution == Resolution::Merge)
            .collect();
        merged.sort_by_key(|node| std::cmp::Reverse(node.source.components().count()));
        for node in merged {
            let holds_skipped = members.iter().any(|&index| {
                plan.nodes[index].resolution == Resolution::Skip
                    && plan.nodes[index].source.starts_with(&node.source)
            });
            if holds_skipped {
                continue;
            }
            #[cfg(windows)]
            if crate::platform::path_identity(&node.source)? != node.identity.file_id {
                return Err(format!(
                    "Merged source folder changed; it was kept: {}",
                    display_path(&node.source)
                ));
            }
            // remove_dir succeeds only for an empty folder.
            fs::remove_dir(&node.source).map_err(|e| {
                format!(
                    "Merged source folder was kept; it could not be removed as empty: {}: {e}",
                    display_path(&node.source)
                )
            })?;
            outcome.effects.removed.push(node.source.clone());
        }
    }
    Ok(())
}

pub fn execute(scope: &Scope, plan: &Plan, approved: bool, cancelled: &AtomicBool) -> Outcome {
    execute_with(scope, plan, approved, cancelled, &Hooks::default())
}

/// `execute` with test seams for recycling and copy verification.
#[doc(hidden)]
pub fn execute_with(
    scope: &Scope,
    plan: &Plan,
    approved: bool,
    cancelled: &AtomicBool,
    hooks: &Hooks,
) -> Outcome {
    let skipped = |path: &PathBuf| {
        plan.nodes
            .iter()
            .any(|node| node.source == *path && node.resolution == Resolution::Skip)
    };
    let streams = plan.streams();
    let mut outcome = Outcome {
        id: plan.id,
        incomplete: if streams {
            plan.nodes
                .iter()
                .filter(|node| node.resolution != Resolution::Skip)
                .map(|node| node.source.clone())
                .collect()
        } else {
            plan.sources
                .iter()
                .filter(|source| !skipped(source))
                .cloned()
                .collect()
        },
        operation: Some(plan.operation),
        strategy: plan.strategy,
        ..Default::default()
    };
    if !approved {
        outcome.error = Some("Explicit plan approval is required; no changes made.".into());
        return outcome;
    }
    let result = (|| -> FileResult<()> {
        if cancelled.load(Ordering::Relaxed) {
            outcome.cancelled = true;
            return Ok(());
        }
        #[cfg(windows)]
        let mut guards = if streams {
            lock_copy_paths(plan)?
        } else {
            Guards::new()
        };
        #[cfg(not(windows))]
        let mut guards = Guards::new();
        validate_plan(scope, plan, cancelled)?;
        match (plan.operation, plan.strategy) {
            (Operation::Copy, _) => {
                let all: Vec<usize> = (0..plan.nodes.len()).collect();
                copy_nodes(
                    scope,
                    plan,
                    &all,
                    hooks,
                    &mut guards,
                    &mut outcome,
                    cancelled,
                )?;
            }
            (Operation::Move, Some(MoveStrategy::CrossVolume { recycle_originals })) => {
                move_across(
                    scope,
                    plan,
                    recycle_originals,
                    hooks,
                    &mut guards,
                    &mut outcome,
                    cancelled,
                )?;
            }
            (Operation::Move, _) => {
                let resolutions: std::collections::BTreeMap<&Path, Resolution> = plan
                    .nodes
                    .iter()
                    .map(|node| (node.source.as_path(), node.resolution))
                    .collect();
                'sources: for source in &plan.sources {
                    if cancelled.load(Ordering::Relaxed) {
                        outcome.cancelled = true;
                        break;
                    }
                    let top = plan
                        .nodes
                        .iter()
                        .find(|node| node.source == *source)
                        .ok_or("Approved move source identity missing.")?;
                    if top.resolution == Resolution::Skip {
                        outcome.skipped.push(source.clone());
                        continue;
                    }
                    scope.mutation_source(source)?;
                    let members: Vec<&PlannedNode> = plan
                        .nodes
                        .iter()
                        .filter(|node| node.source.starts_with(source))
                        .collect();
                    for node in &members {
                        if cancelled.load(Ordering::Relaxed) {
                            return Err(
                                "Move validation cancelled; no further changes made.".into()
                            );
                        }
                        if fingerprint(&node.source)? != node.identity {
                            return Err(format!(
                                "Source changed; move stopped: {}",
                                display_path(&node.source)
                            ));
                        }
                    }
                    #[cfg(windows)]
                    if let Some(destination) = &plan.destination
                        && crate::platform::path_identity(destination)?
                            != plan
                                .destination_identity
                                .as_ref()
                                .ok_or("Destination identity missing.")?
                                .file_id
                    {
                        return Err("Destination identity changed; move stopped.".into());
                    }
                    if cancelled.load(Ordering::Relaxed) {
                        outcome.cancelled = true;
                        break;
                    }
                    if top.resolution != Resolution::Merge {
                        move_node(plan, top, hooks, &mut outcome)?;
                        continue;
                    }
                    // A merged folder cannot be renamed as a whole; move its direct units.
                    for node in &members {
                        let unit = node
                            .source
                            .parent()
                            .and_then(|parent| resolutions.get(parent))
                            == Some(&Resolution::Merge);
                        if !unit || node.resolution == Resolution::Merge {
                            continue;
                        }
                        if cancelled.load(Ordering::Relaxed) {
                            outcome.cancelled = true;
                            break 'sources;
                        }
                        if node.resolution == Resolution::Skip {
                            outcome.skipped.push(node.source.clone());
                        } else {
                            move_node(plan, node, hooks, &mut outcome)?;
                        }
                    }
                    let mut merged: Vec<&PlannedNode> = members
                        .iter()
                        .copied()
                        .filter(|node| node.resolution == Resolution::Merge)
                        .collect();
                    merged.sort_by_key(|node| std::cmp::Reverse(node.source.components().count()));
                    for node in merged {
                        let target = node.target.as_ref().ok_or("Move destination is missing.")?;
                        let kept = members.iter().any(|other| {
                            other.resolution == Resolution::Skip
                                && other.source.starts_with(&node.source)
                        });
                        if !kept {
                            #[cfg(windows)]
                            if crate::platform::path_identity(&node.source)?
                                != node.identity.file_id
                            {
                                return Err(format!(
                                    "Merged source folder changed; it was kept: {}",
                                    display_path(&node.source)
                                ));
                            }
                            // remove_dir succeeds only for an empty folder.
                            fs::remove_dir(&node.source).map_err(|e| {
                                format!(
                                    "Merged source folder was kept; it could not be removed as empty: {}: {e}",
                                    display_path(&node.source)
                                )
                            })?;
                            outcome.effects.removed.push(node.source.clone());
                        }
                        outcome.merged.push(target.clone());
                        outcome.completed.push(node.source.clone());
                    }
                }
            }
            (Operation::Recycle, _) => {
                for source in &plan.sources {
                    if cancelled.load(Ordering::Relaxed) {
                        outcome.cancelled = true;
                        break;
                    }
                    scope.mutation_source(source)?;
                    for node in plan
                        .nodes
                        .iter()
                        .filter(|node| node.source.starts_with(source))
                    {
                        if cancelled.load(Ordering::Relaxed) {
                            return Err(
                                "Recycle validation cancelled; no further changes made.".into()
                            );
                        }
                        if fingerprint(&node.source)? != node.identity {
                            return Err(format!(
                                "Source changed; recycling stopped: {}",
                                display_path(&node.source)
                            ));
                        }
                    }
                    if cancelled.load(Ordering::Relaxed) {
                        outcome.cancelled = true;
                        break;
                    }
                    (hooks.recycle)(std::slice::from_ref(source))?;
                    if source
                        .try_exists()
                        .map_err(|e| io_error("Verify recycling", source, e))?
                    {
                        return Err(format!(
                            "Windows did not remove the source: {}",
                            display_path(source)
                        ));
                    }
                    outcome.completed.push(source.clone());
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        outcome.error = Some(error);
        outcome.cancelled = cancelled.load(Ordering::Relaxed);
    }
    let completed: BTreeSet<_> = outcome.completed.iter().collect();
    outcome
        .incomplete
        .retain(|source| !completed.contains(source));
    let removed = std::mem::take(&mut outcome.effects.removed);
    outcome.effects = effects(plan, &outcome);
    outcome.effects.removed = removed;
    outcome
}

/// What a finished plan proves it did: complete created units, moved units, replaced and
/// recycled originals, and cross-volume originals left in place. Partial units are reported but
/// never undone.
fn effects(plan: &Plan, outcome: &Outcome) -> crate::undo::Effects {
    use crate::undo::{Effects, Kind};
    let kind = match (plan.operation, plan.strategy) {
        (Operation::Copy, _) => Kind::Copy,
        (Operation::Recycle, _) => Kind::Recycle,
        (
            Operation::Move,
            Some(MoveStrategy::CrossVolume {
                recycle_originals: true,
            }),
        ) => Kind::MoveAcross,
        (Operation::Move, Some(MoveStrategy::CrossVolume { .. })) => Kind::MoveAcrossKept,
        (Operation::Move, _) => Kind::Move,
    };
    let mut effects = Effects {
        kind: Some(kind),
        replaced: outcome.replaced.clone(),
        ..Default::default()
    };
    if kind == Kind::Recycle {
        effects.recycled = outcome.completed.clone();
        return effects;
    }
    let completed: BTreeSet<&Path> = outcome.completed.iter().map(PathBuf::as_path).collect();
    let resolutions: std::collections::BTreeMap<&Path, Resolution> = plan
        .nodes
        .iter()
        .map(|node| (node.source.as_path(), node.resolution))
        .collect();
    // A unit is a top-level item or a direct child of a merged folder that the plan writes.
    let units = plan.nodes.iter().filter(|node| {
        matches!(
            node.resolution,
            Resolution::Create | Resolution::Rename | Resolution::Replace
        ) && (plan.sources.contains(&node.source)
            || node
                .source
                .parent()
                .and_then(|parent| resolutions.get(parent))
                == Some(&Resolution::Merge))
    });
    if kind == Kind::Move {
        for unit in units {
            if let Some(target) = &unit.target
                && completed.contains(unit.source.as_path())
            {
                effects.moved.push((unit.source.clone(), target.clone()));
            }
        }
        return effects;
    }
    let mut covered = Vec::new();
    let recycled: BTreeSet<&Path> = outcome.recycled.iter().map(PathBuf::as_path).collect();
    for unit in units {
        let Some(target) = &unit.target else {
            continue;
        };
        let done = plan
            .nodes
            .iter()
            .filter(|node| node.source.starts_with(&unit.source))
            .all(|node| completed.contains(node.source.as_path()));
        if done && outcome.created.contains(target) {
            effects.created.push(target.clone());
            covered.push(target.as_path());
            // Any original not in the Recycle Bin is still in place, kept or not.
            if kind != Kind::Copy
                && !unit
                    .source
                    .ancestors()
                    .any(|original| recycled.contains(original))
            {
                effects.kept.push((unit.source.clone(), target.clone()));
            }
        }
    }
    effects.partial = crate::undo::outermost(
        outcome
            .created
            .iter()
            .filter(|path| !covered.iter().any(|unit| path.starts_with(unit)))
            .cloned()
            .collect(),
    );
    effects.recycled = outcome.recycled.clone();
    effects
}

#[cfg(windows)]
pub(crate) fn lock_mutation_parents(parent: &Path) -> FileResult<Guards> {
    lock_paths(parent.ancestors().map(Path::to_owned).collect())
}

#[cfg(windows)]
fn lock_copy_paths(plan: &Plan) -> FileResult<Guards> {
    let mut paths = BTreeSet::new();
    for node in &plan.nodes {
        paths.extend(node.source.ancestors().map(Path::to_owned));
    }
    if let Some(destination) = &plan.destination {
        paths.extend(destination.ancestors().map(Path::to_owned));
    }
    lock_paths(paths)
}

#[cfg(windows)]
fn lock_paths(paths: BTreeSet<PathBuf>) -> FileResult<Guards> {
    use std::os::windows::fs::OpenOptionsExt;
    let mut handles = Vec::with_capacity(paths.len());
    for path in paths {
        let metadata =
            fs::symlink_metadata(&path).map_err(|e| io_error("Inspect copy guard", &path, e))?;
        if reparse(&metadata) {
            return Err(format!(
                "Copy path changed to a link: {}",
                display_path(&path)
            ));
        }
        let handle = OpenOptions::new()
            .access_mode(0)
            .share_mode(3)
            .custom_flags(0x0220_0000)
            .open(&path)
            .map_err(|e| io_error("Lock copy path against replacement", &path, e))?;
        handles.push((path, handle));
    }
    Ok(handles)
}

pub fn read_bytes(path: &Path) -> FileResult<Vec<u8>> {
    let mut output = Vec::new();
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut output))
        .map_err(|e| io_error("Read", path, e))?;
    Ok(output)
}
