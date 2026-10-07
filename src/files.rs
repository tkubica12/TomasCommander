use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::SystemTime,
};

pub type FileResult<T> = Result<T, String>;

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
    pub link: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub folded: String,
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
    format!("{action} {}: {error}", path.display())
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
            return Err(format!("Outside the fixture boundary: {}", path.display()));
        }
        Ok(resolved)
    }

    pub fn mutation_source(&self, path: &Path) -> FileResult<PathBuf> {
        let metadata = fs::symlink_metadata(path).map_err(|e| io_error("Inspect", path, e))?;
        if reparse(&metadata) {
            return Err(format!(
                "Links/reparse points cannot be modified: {}",
                path.display()
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
    Ok(Listing { entries, warnings })
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
            Self::Recycle => "Recycle",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Fingerprint {
    directory: bool,
    size: u64,
    modified: SystemTime,
}

fn fingerprint(path: &Path) -> FileResult<Fingerprint> {
    let meta = fs::symlink_metadata(path).map_err(|e| io_error("Inspect", path, e))?;
    if reparse(&meta) || !(meta.is_file() || meta.is_dir()) {
        return Err(format!(
            "Unsupported link or special item: {}",
            path.display()
        ));
    }
    Ok(Fingerprint {
        directory: meta.is_dir(),
        size: meta.len(),
        modified: meta
            .modified()
            .map_err(|e| io_error("Read modification time", path, e))?,
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
    pub id: u64,
    pub operation: Operation,
    pub sources: Vec<PathBuf>,
    pub destination: Option<PathBuf>,
    nodes: Vec<PlannedNode>,
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
                    target.display()
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
    Ok(Plan {
        id,
        operation,
        sources,
        destination,
        nodes,
    })
}

#[derive(Debug)]
pub struct Outcome {
    pub id: u64,
    pub completed: Vec<PathBuf>,
    pub created: Vec<PathBuf>,
    pub error: Option<String>,
    pub cancelled: bool,
}

fn validate_plan(scope: &Scope, plan: &Plan) -> FileResult<()> {
    if let Some(destination) = &plan.destination {
        let resolved = scope.resolve(destination)?;
        if resolved != *destination {
            return Err("Destination folder changed; create a new plan.".into());
        }
    }
    for node in &plan.nodes {
        if scope.mutation_source(&node.source)? != node.source
            || fingerprint(&node.source)? != node.identity
        {
            return Err(format!(
                "Source changed; create a new plan: {}",
                node.source.display()
            ));
        }
        if let Some(target) = &node.target
            && target
                .try_exists()
                .map_err(|e| io_error("Inspect destination", target, e))?
        {
            return Err(format!(
                "Destination changed; no overwrite allowed: {}",
                target.display()
            ));
        }
    }
    // Re-enumeration detects added/removed children, not only changed top-level metadata.
    let current = self::plan(
        scope,
        plan.id,
        plan.operation,
        &plan.sources,
        plan.destination.as_deref(),
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
        validate_plan(scope, plan)?;
        match plan.operation {
            Operation::Copy => {
                for node in &plan.nodes {
                    if cancelled.load(Ordering::Relaxed) {
                        outcome.cancelled = true;
                        break;
                    }
                    scope.mutation_source(&node.source)?;
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
                        if reparse(&meta)
                            || meta.len() != node.identity.size
                            || meta
                                .modified()
                                .map_err(|e| io_error("Read source time", &node.source, e))?
                                != node.identity.modified
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
                            .flush()
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
                    let target = plan
                        .destination
                        .as_ref()
                        .ok_or("Move destination is missing.")?
                        .join(source.file_name().ok_or("Source filename is missing.")?);
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
                    crate::platform::recycle(source)?;
                    if source
                        .try_exists()
                        .map_err(|e| io_error("Verify recycling", source, e))?
                    {
                        return Err(format!(
                            "Windows did not remove the source: {}",
                            source.display()
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
    }
    outcome
}

pub fn read_bytes(path: &Path) -> FileResult<Vec<u8>> {
    let mut output = Vec::new();
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut output))
        .map_err(|e| io_error("Read", path, e))?;
    Ok(output)
}
