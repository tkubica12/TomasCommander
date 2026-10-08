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
        let resolved = fs::canonicalize(path).map_err(|e| io_error("Resolve", path, e))?;
        if let Some(root) = &self.root
            && !resolved.starts_with(root)
        {
            return Err(format!(
                "Outside the fixture boundary: {}",
                display_path(path)
            ));
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
    let path = scope.resolve(path)?;
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    for result in fs::read_dir(&path).map_err(|e| io_error("Read folder", &path, e))? {
        if stale() {
            return Err("Directory scan was superseded.".into());
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

#[derive(Clone, Debug)]
struct PlannedNode {
    source: PathBuf,
    target: Option<PathBuf>,
    identity: Fingerprint,
}

#[derive(Clone, Debug)]
pub struct Plan {
    id: u64,
    operation: Operation,
    sources: Vec<PathBuf>,
    destination: Option<PathBuf>,
    nodes: Vec<PlannedNode>,
    destination_identity: Option<Fingerprint>,
    execution_key: String,
}

impl Plan {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn operation(&self) -> Operation {
        self.operation
    }

    pub fn sources(&self) -> &[PathBuf] {
        &self.sources
    }

    pub fn destination(&self) -> Option<&Path> {
        self.destination.as_deref()
    }

    pub(crate) fn execution_key(&self) -> &str {
        &self.execution_key
    }

    pub(crate) fn journal_snapshot(&self) -> String {
        let mut snapshot = format!("operation={:?}\nid={}\n", self.operation, self.id);
        if let Some(destination) = &self.destination {
            snapshot.push_str(&format!(
                "destination={}\ndestination_identity={:?}\n",
                crate::journal::encode_path(destination),
                self.destination_identity
            ));
        }
        for node in &self.nodes {
            snapshot.push_str(&format!(
                "source={}\nidentity={:?}\n",
                crate::journal::encode_path(&node.source),
                node.identity
            ));
            if let Some(target) = &node.target {
                snapshot.push_str(&format!("target={}\n", crate::journal::encode_path(target)));
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
    if sources.is_empty() {
        return Err("Select or focus an item first.".into());
    }
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
    let mut nodes = Vec::new();
    let mut targets = BTreeSet::new();
    for source in &sources {
        if let Some(destination) = &destination
            && destination.starts_with(source)
        {
            return Err("A folder cannot be copied or moved into itself.".into());
        }
        let target = destination.as_ref().map(|destination| {
            destination.join(
                source
                    .file_name()
                    .expect("Root sources were rejected above"),
            )
        });
        if let Some(target) = &target {
            let name = target.to_string_lossy().to_lowercase();
            if !targets.insert(name) {
                return Err("Multiple selected items have the same destination name.".into());
            }
            if target
                .try_exists()
                .map_err(|e| io_error("Inspect destination", target, e))?
            {
                return Err(format!(
                    "Conflict; no overwrite allowed: {}",
                    display_path(target)
                ));
            }
        }
        let mut stack = vec![(source.clone(), target)];
        while let Some((source, target)) = stack.pop() {
            if cancelled.load(Ordering::Relaxed) {
                return Err("Planning was cancelled.".into());
            }
            scope.mutation_source(&source)?;
            let identity = fingerprint(&source)?;
            if identity.directory {
                for child in
                    fs::read_dir(&source).map_err(|e| io_error("Read folder", &source, e))?
                {
                    let child = child.map_err(|e| io_error("Read child", &source, e))?;
                    stack.push((
                        child.path(),
                        target.as_ref().map(|path| path.join(child.file_name())),
                    ));
                }
            }
            nodes.push(PlannedNode {
                source,
                target,
                identity,
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
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|e| format!("Create operation identity: {e}"))?
        .as_nanos();
    Ok(Plan {
        id,
        operation,
        sources,
        destination,
        nodes,
        destination_identity,
        execution_key: format!(
            "{timestamp}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ),
    })
}

#[derive(Debug)]
pub struct Outcome {
    pub id: u64,
    pub completed: Vec<PathBuf>,
    pub created: Vec<PathBuf>,
    pub error: Option<String>,
    pub cancelled: bool,
    pub incomplete: Vec<PathBuf>,
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
            && target
                .try_exists()
                .map_err(|e| io_error("Inspect destination", target, e))?
        {
            return Err(format!(
                "Destination changed; no overwrite allowed: {}",
                display_path(target)
            ));
        }
    }
    // Re-enumeration detects added/removed children, not only changed top-level metadata.
    let current = self::plan_cancellable(
        scope,
        plan.id,
        plan.operation,
        &plan.sources,
        plan.destination.as_deref(),
        cancelled,
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
    Ok(())
}

pub fn execute(scope: &Scope, plan: &Plan, approved: bool, cancelled: &AtomicBool) -> Outcome {
    let mut outcome = Outcome {
        id: plan.id,
        completed: Vec::new(),
        created: Vec::new(),
        error: None,
        cancelled: false,
        incomplete: if plan.operation == Operation::Copy {
            plan.nodes.iter().map(|node| node.source.clone()).collect()
        } else {
            plan.sources.clone()
        },
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
        let mut copy_guards = if plan.operation == Operation::Copy {
            Some(lock_copy_paths(plan)?)
        } else {
            None
        };
        validate_plan(scope, plan, cancelled)?;
        match plan.operation {
            Operation::Copy => {
                for node in &plan.nodes {
                    if cancelled.load(Ordering::Relaxed) {
                        outcome.cancelled = true;
                        break;
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
                    if node.identity.directory {
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
                                return Err(
                                    "Created copy folder changed to a link; copy stopped.".into()
                                );
                            }
                            copy_guards
                                .as_mut()
                                .ok_or("Copy guards missing.")?
                                .push(handle);
                        }
                    } else {
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
                            return Err(
                                "Source identity changed while opening it; copy stopped.".into()
                            );
                        }
                        if reparse(&meta)
                            || meta.len() != node.identity.size
                            || meta
                                .modified()
                                .map_err(|e| io_error("Read source time", &node.source, e))?
                                != node.identity.modified
                            || meta.created().map_err(|e| {
                                io_error("Read source creation time", &node.source, e)
                            })? != node.identity.created
                        {
                            return Err("Source changed while opening it; copy stopped.".into());
                        }
                        let mut output = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(target)
                            .map_err(|e| {
                                io_error("Create destination without overwrite", target, e)
                            })?;
                        outcome.created.push(target.clone());
                        let mut buffer = [0u8; 64 * 1024];
                        loop {
                            if cancelled.load(Ordering::Relaxed) {
                                outcome.cancelled = true;
                                break;
                            }
                            let count = source
                                .read(&mut buffer)
                                .map_err(|e| io_error("Read source", &node.source, e))?;
                            if count == 0 {
                                break;
                            }
                            output
                                .write_all(&buffer[..count])
                                .map_err(|e| io_error("Write destination", target, e))?;
                        }
                        output
                            .sync_all()
                            .map_err(|e| io_error("Flush destination", target, e))?;
                        if outcome.cancelled {
                            break;
                        }
                        if output
                            .metadata()
                            .map_err(|e| io_error("Inspect copied file", target, e))?
                            .len()
                            != node.identity.size
                        {
                            return Err(
                                "Copied file length does not match the approved source.".into()
                            );
                        }
                    }
                    outcome.completed.push(node.source.clone());
                }
            }
            Operation::Move => {
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
                    let target = plan
                        .destination
                        .as_ref()
                        .ok_or("Move destination is missing.")?
                        .join(source.file_name().ok_or("Source filename is missing.")?);
                    if cancelled.load(Ordering::Relaxed) {
                        outcome.cancelled = true;
                        break;
                    }
                    #[cfg(windows)]
                    {
                        let identity = &plan
                            .nodes
                            .iter()
                            .find(|node| node.source == *source)
                            .ok_or("Approved move source identity missing.")?
                            .identity;
                        let destination = plan
                            .destination_identity
                            .as_ref()
                            .ok_or("Approved move destination identity missing.")?;
                        crate::platform::move_verified(
                            source,
                            &target,
                            identity,
                            destination.file_id,
                        )?;
                    }
                    #[cfg(not(windows))]
                    crate::platform::move_no_replace(source, &target)?;
                    outcome.created.push(target);
                    outcome.completed.push(source.clone());
                }
            }
            Operation::Recycle => {
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
                    crate::platform::recycle(source)?;
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
    outcome
}

#[cfg(windows)]
fn lock_copy_paths(plan: &Plan) -> FileResult<Vec<File>> {
    use std::os::windows::fs::OpenOptionsExt;
    let mut paths = BTreeSet::new();
    for node in &plan.nodes {
        paths.extend(node.source.ancestors().map(Path::to_owned));
    }
    if let Some(destination) = &plan.destination {
        paths.extend(destination.ancestors().map(Path::to_owned));
    }
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
        handles.push(handle);
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
