use crate::{
    files::{self, FileResult},
    preferences,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
mod windows;

pub const MAX_ITEMS: usize = 100;
const SCAN_LIMIT: usize = 100_000;
const LOOKBACK: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    Modified,
    Opened,
    Previewed,
    Saved,
    FileAction,
    WindowsRecent,
}

impl Activity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Modified => "Changed",
            Self::Opened => "Opened",
            Self::Previewed => "Viewed",
            Self::Saved => "Saved",
            Self::FileAction => "File operation",
            Self::WindowsRecent => "Windows Recent",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub path: PathBuf,
    pub at_ms: u64,
    pub activity: Activity,
    modified_ms: u64,
    size: u64,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub items: Vec<Item>,
    pub warnings: Vec<String>,
    pub scanning: bool,
    pub roots: Option<Vec<PathBuf>>,
    pub configuration_revision: u64,
}

impl Snapshot {
    fn warn(&mut self, warning: String) {
        if !self.warnings.contains(&warning) && self.warnings.len() < 12 {
            self.warnings.push(warning);
        }
    }

    fn record(&mut self, item: Item) -> bool {
        if let Some(old) = self
            .items
            .iter_mut()
            .find(|old| key(&old.path) == key(&item.path))
        {
            if item.at_ms <= old.at_ms {
                return false;
            }
            *old = item;
        } else {
            self.items.push(item);
        }
        self.items
            .sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then(a.path.cmp(&b.path)));
        self.items.truncate(MAX_ITEMS);
        true
    }

    fn remove(&mut self, path: &Path) -> bool {
        let length = self.items.len();
        let path = key(path);
        self.items.retain(|item| {
            !key(&item.path).starts_with(&(path.clone() + "\\")) && key(&item.path) != path
        });
        length != self.items.len()
    }
}

fn key(path: &Path) -> String {
    files::display_path(path).to_lowercase()
}

fn excluded(path: &Path) -> bool {
    path.components().any(|component| {
        let Component::Normal(name) = component else {
            return false;
        };
        let name = name.to_string_lossy().to_lowercase();
        name.starts_with('.')
            || name.starts_with("~$")
            || matches!(
                name.as_str(),
                "node_modules"
                    | "target"
                    | "dist"
                    | "build"
                    | "bin"
                    | "obj"
                    | "cache"
                    | "caches"
                    | "temp"
                    | "tmp"
                    | "appdata"
                    | "windows"
                    | "program files"
                    | "program files (x86)"
                    | "programdata"
                    | "$recycle.bin"
                    | "system volume information"
            )
    })
}

fn allowed(path: &Path) -> bool {
    path.is_absolute() && !excluded(path) && preferences::document_path(path)
}

fn milliseconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn inspect(
    path: &Path,
    activity: Activity,
    at_ms: Option<u64>,
    roots: Option<&[PathBuf]>,
) -> FileResult<Option<Item>> {
    if !allowed(path) {
        return Ok(None);
    }
    if roots.is_some_and(|roots| {
        !roots.iter().any(|root| {
            key(path).starts_with(&(key(root).trim_end_matches('\\').to_owned() + "\\"))
        })
    }) {
        return Ok(None);
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Recent: inspect {}: {error}",
                files::display_path(path)
            ));
        }
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Ok(None);
    }
    #[cfg(windows)]
    if std::os::windows::fs::MetadataExt::file_attributes(&metadata) & 0x6 != 0 {
        return Ok(None);
    }
    let resolved = fs::canonicalize(path)
        .map_err(|error| format!("Recent: resolve {}: {error}", files::display_path(path)))?;
    if !allowed(&resolved)
        || roots.is_some_and(|roots| !roots.iter().any(|root| resolved.starts_with(root)))
    {
        return Ok(None);
    }
    let modified_ms = milliseconds(metadata.modified().map_err(|error| {
        format!(
            "Recent: read modification time {}: {error}",
            files::display_path(path)
        )
    })?);
    Ok(Some(Item {
        path: resolved,
        at_ms: at_ms.unwrap_or(modified_ms),
        activity,
        modified_ms,
        size: metadata.len(),
    }))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    version: u32,
    items: Vec<Item>,
}

fn load(path: &Path) -> FileResult<Vec<Item>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Recent history: {error}")),
    };
    let mut bytes = Vec::new();
    file.take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Read Recent history: {error}"))?;
    if bytes.len() > 1_048_576 {
        return Err("Recent history exceeds its size limit; not overwritten.".into());
    }
    let history: History = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Invalid Recent history; not overwritten: {error}"))?;
    if history.version != 1
        || history.items.len() > MAX_ITEMS
        || history.items.iter().any(|item| !allowed(&item.path))
    {
        return Err("Unsupported Recent history; not overwritten.".into());
    }
    Ok(history.items)
}

fn save(path: &Path, items: &[Item]) -> FileResult<()> {
    let parent = path
        .parent()
        .ok_or("Recent history needs a parent directory.")?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Create Recent history directory: {error}"))?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec(&History {
        version: 1,
        items: items.to_vec(),
    })
    .map_err(|error| format!("Encode Recent history: {error}"))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Create Recent history transaction: {error}"))?;
    let result = file.write_all(&bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = result.and_then(|()| fs::rename(&temporary, path));
    if let Err(error) = result {
        fs::remove_file(&temporary).map_err(|cleanup| {
            format!("Save Recent history failed: {error}; cleanup failed: {cleanup}")
        })?;
        return Err(format!("Save Recent history: {error}"));
    }
    Ok(())
}

pub struct Config {
    pub roots: Vec<PathBuf>,
    pub history: PathBuf,
    pub legacy: Vec<PathBuf>,
    pub windows_recent: bool,
    pub discover_roots: bool,
    pub restricted: bool,
}

impl Config {
    pub fn for_app(
        settings: &Path,
        fixture: Option<&Path>,
        legacy: Vec<PathBuf>,
        roots: Option<Vec<PathBuf>>,
    ) -> Self {
        Self {
            roots: fixture
                .map(|path| vec![path.to_path_buf()])
                .unwrap_or_else(|| roots.clone().unwrap_or_default()),
            history: settings.with_file_name("recent-activity.json"),
            legacy,
            windows_recent: fixture.is_none(),
            discover_roots: fixture.is_none() && roots.is_none(),
            restricted: fixture.is_some(),
        }
    }
}

enum Request {
    Record(Vec<PathBuf>, Activity),
    Configure(Vec<PathBuf>),
}

pub struct Monitor {
    commands: SyncSender<Request>,
    snapshots: Receiver<Snapshot>,
    stopped: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<FileResult<()>>>,
    disconnected: bool,
}

impl Monitor {
    pub fn start(config: Config, wake: impl Fn() + Send + 'static) -> Self {
        let (commands, requests) = mpsc::sync_channel(128);
        let (sender, snapshots) = mpsc::sync_channel(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let worker = thread::spawn(move || run(config, requests, sender, stop, wake));
        Self {
            commands,
            snapshots,
            stopped,
            worker: Some(worker),
            disconnected: false,
        }
    }

    pub fn record(&self, path: PathBuf, activity: Activity) -> FileResult<()> {
        self.record_many([path], activity)
    }

    pub fn record_many(
        &self,
        paths: impl IntoIterator<Item = PathBuf>,
        activity: Activity,
    ) -> FileResult<()> {
        let paths: Vec<_> = paths
            .into_iter()
            .filter(|path| allowed(path))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(MAX_ITEMS)
            .collect();
        if paths.is_empty() {
            return Ok(());
        }
        self.commands
            .try_send(Request::Record(paths, activity))
            .map_err(|error| format!("Recent activity was not queued: {error}"))
    }

    pub fn configure(&self, roots: Vec<PathBuf>) -> FileResult<()> {
        if roots.len() > 16
            || roots
                .iter()
                .any(|root| !root.is_absolute() || excluded(root))
        {
            return Err("Recent needs at most 16 absolute document folders, excluding system/cache folders.".into());
        }
        self.commands
            .try_send(Request::Configure(roots))
            .map_err(|error| format!("Recent configuration was not queued: {error}"))
    }

    pub fn shutdown(&mut self) -> FileResult<()> {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "Recent worker panicked during shutdown.".to_string())??;
        }
        Ok(())
    }

    pub fn poll(&mut self) -> FileResult<Option<Snapshot>> {
        let mut latest = None;
        loop {
            match self.snapshots.try_recv() {
                Ok(snapshot) => latest = Some(snapshot),
                Err(mpsc::TryRecvError::Empty) => return Ok(latest),
                Err(mpsc::TryRecvError::Disconnected) if !self.disconnected => {
                    self.disconnected = true;
                    return Err(
                        "Recent monitoring stopped; restart the application to resume tracking."
                            .into(),
                    );
                }
                Err(mpsc::TryRecvError::Disconnected) => return Ok(latest),
            }
        }
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        // Never wait for filesystem work on the UI thread.
        self.worker.take();
    }
}

fn scan(snapshot: &mut Snapshot, roots: &[PathBuf], start: &[PathBuf], stop: &AtomicBool) {
    scan_limited(snapshot, roots, start, stop, SCAN_LIMIT);
}

fn scan_limited(
    snapshot: &mut Snapshot,
    roots: &[PathBuf],
    start: &[PathBuf],
    stop: &AtomicBool,
    limit: usize,
) {
    let started = Instant::now();
    let cutoff = milliseconds(
        SystemTime::now()
            .checked_sub(LOOKBACK)
            .unwrap_or(UNIX_EPOCH),
    );
    let mut folders = start.to_vec();
    let mut visited = BTreeSet::new();
    let mut examined = 0;
    while let Some(folder) = folders.pop() {
        if !visited.insert(folder.clone()) {
            continue;
        }
        if stop.load(Ordering::Acquire) {
            return;
        }
        if examined >= limit || started.elapsed() >= Duration::from_secs(5) {
            snapshot.warn("Recent startup scan is partial (100,000 entries / 5 seconds); live notifications remain active.".into());
            return;
        }
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) => {
                snapshot.warn(format!(
                    "Recent: cannot scan {}: {error}",
                    files::display_path(&folder)
                ));
                continue;
            }
        };
        for entry in entries {
            if stop.load(Ordering::Acquire) {
                return;
            }
            examined += 1;
            if examined >= limit || started.elapsed() >= Duration::from_secs(5) {
                snapshot.warn("Recent startup scan is partial (100,000 entries / 5 seconds); live notifications remain active.".into());
                return;
            }
            let result = (|| -> FileResult<()> {
                let entry = entry.map_err(|error| format!("Recent directory entry: {error}"))?;
                let path = entry.path();
                if excluded(&path) {
                    return Ok(());
                }
                let kind = entry
                    .file_type()
                    .map_err(|error| format!("Recent entry type: {error}"))?;
                if kind.is_symlink() {
                    return Ok(());
                }
                if kind.is_dir() {
                    let resolved = fs::canonicalize(&path)
                        .map_err(|error| format!("Recent folder: {error}"))?;
                    if roots.iter().any(|root| resolved.starts_with(root))
                        && !folders.contains(&resolved)
                    {
                        // Junctions must not expand or cycle the monitored tree.
                        if !files::reparse(
                            &entry
                                .metadata()
                                .map_err(|error| format!("Recent folder attributes: {error}"))?,
                        ) || windows_cloud_directory(&path)?
                        {
                            folders.push(resolved);
                        }
                    }
                } else if let Some(item) = inspect(&path, Activity::Modified, None, Some(roots))?
                    && item.at_ms >= cutoff
                {
                    snapshot.record(item);
                }
                Ok(())
            })();
            if let Err(error) = result {
                snapshot.warn(error);
            }
        }
    }
}

fn windows_cloud_directory(path: &Path) -> FileResult<bool> {
    #[cfg(windows)]
    {
        windows::cloud_directory(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(false)
    }
}

fn run(
    config: Config,
    requests: Receiver<Request>,
    sender: SyncSender<Snapshot>,
    stop: Arc<AtomicBool>,
    wake: impl Fn(),
) -> FileResult<()> {
    let mut snapshot = Snapshot {
        scanning: true,
        ..Default::default()
    };
    let mut writable = true;
    let old = match load(&config.history) {
        Ok(items) => items,
        Err(error) => {
            snapshot.warn(error);
            writable = false;
            Vec::new()
        }
    };
    let _ = sender.try_send(snapshot.clone());
    wake();
    let mut roots = config.roots;
    #[cfg(windows)]
    if config.discover_roots {
        match windows::default_roots() {
            Ok(paths) => roots.extend(paths),
            Err(error) => snapshot.warn(error),
        }
    }
    roots.sort_by_key(|path| key(path));
    roots.dedup_by(|a, b| key(a) == key(b));
    let configured_roots = roots.clone();
    roots = roots
        .into_iter()
        .filter_map(|path| match fs::canonicalize(&path) {
            Ok(path) => Some(path),
            Err(error) => {
                snapshot.warn(format!(
                    "Recent root {} unavailable: {error}",
                    files::display_path(&path)
                ));
                None
            }
        })
        .collect();
    roots.sort();
    roots.dedup();
    let candidates = roots.clone();
    roots.retain(|path| {
        !candidates
            .iter()
            .any(|parent| parent != path && path.starts_with(parent))
    });
    #[cfg(windows)]
    let _apartment = match windows::Apartment::new() {
        Ok(apartment) => Some(apartment),
        Err(error) => {
            snapshot.warn(error);
            None
        }
    };
    #[cfg(windows)]
    let shell = if config.windows_recent && _apartment.is_some() {
        match windows::recent_folder() {
            Ok(path) => Some(path),
            Err(error) => {
                snapshot.warn(error);
                None
            }
        }
    } else {
        None
    };
    #[cfg(windows)]
    let mut watches = Vec::new();
    #[cfg(windows)]
    for (root, shell) in roots.iter().map(|root| (root, false)).chain(
        shell
            .iter()
            .filter(|_| !roots.is_empty())
            .map(|root| (root, true)),
    ) {
        match windows::Watch::new(root, !shell) {
            Ok(watch) => watches.push((watch, shell)),
            Err(error) => snapshot.warn(error),
        }
    }
    for old in old {
        match inspect(
            &old.path,
            old.activity,
            Some(old.at_ms),
            config.restricted.then_some(roots.as_slice()),
        ) {
            Ok(Some(item)) => {
                snapshot.record(item);
            }
            Ok(None) => {}
            Err(error) => snapshot.warn(error),
        }
    }
    for path in config.legacy {
        match inspect(
            &path,
            Activity::Modified,
            None,
            config.restricted.then_some(roots.as_slice()),
        ) {
            Ok(Some(item)) => {
                snapshot.record(item);
            }
            Ok(None) => {}
            Err(error) => snapshot.warn(error),
        }
    }
    scan(&mut snapshot, &roots, &roots, &stop);
    #[cfg(windows)]
    if let Some(shell) = &shell
        && !roots.is_empty()
        && let Err(error) = windows::import_shell(shell, &roots, &mut snapshot, &stop)
    {
        snapshot.warn(error);
    }
    snapshot.scanning = false;
    snapshot.roots = Some(configured_roots);
    let mut changed = true;
    let mut unsaved = true;
    let mut saved_at = Instant::now();
    let mut pending: BTreeMap<(PathBuf, bool), Instant> = BTreeMap::new();
    let mut rescan = false;
    while !stop.load(Ordering::Acquire) {
        for request in requests.try_iter() {
            let Request::Record(paths, activity) = request else {
                let Request::Configure(paths) = request else {
                    unreachable!()
                };
                let result = (|| -> FileResult<()> {
                    if config.restricted {
                        return Err("Recent roots cannot be changed in fixture mode.".into());
                    }
                    let mut updated = Vec::new();
                    for path in paths {
                        let path = fs::canonicalize(&path).map_err(|error| {
                            format!(
                                "Recent configuration unchanged: {}: {error}",
                                files::display_path(&path)
                            )
                        })?;
                        if !path.is_dir() || excluded(&path) {
                            return Err("Recent configuration unchanged: every root must be a document directory.".into());
                        }
                        updated.push(path);
                    }
                    updated.sort();
                    updated.dedup();
                    let candidates = updated.clone();
                    updated.retain(|path| {
                        !candidates
                            .iter()
                            .any(|parent| parent != path && path.starts_with(parent))
                    });
                    #[cfg(windows)]
                    {
                        let mut replacement = Vec::new();
                        for (path, is_shell) in updated.iter().map(|path| (path, false)).chain(
                            shell
                                .iter()
                                .filter(|_| !updated.is_empty())
                                .map(|path| (path, true)),
                        ) {
                            replacement.push((windows::Watch::new(path, !is_shell)?, is_shell));
                        }
                        watches = replacement;
                    }
                    roots = updated;
                    pending.clear();
                    snapshot.roots = Some(roots.clone());
                    scan(&mut snapshot, &roots, &roots, &stop);
                    #[cfg(windows)]
                    if let Some(shell) = &shell
                        && !roots.is_empty()
                        && let Err(error) =
                            windows::import_shell(shell, &roots, &mut snapshot, &stop)
                    {
                        snapshot.warn(error);
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    snapshot.warn(error);
                } else {
                    unsaved = true;
                }
                snapshot.configuration_revision += 1;
                changed = true;
                continue;
            };
            for path in paths {
                match inspect(
                    &path,
                    activity,
                    Some(milliseconds(SystemTime::now())),
                    config.restricted.then_some(roots.as_slice()),
                ) {
                    Ok(Some(item)) => {
                        changed |= snapshot.record(item);
                        unsaved = true;
                    }
                    Ok(None) if !path.exists() => {
                        changed |= snapshot.remove(&path);
                        unsaved |= changed;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        snapshot.warn(error);
                        changed = true;
                    }
                }
            }
        }
        #[cfg(windows)]
        for (watch, shell) in &mut watches {
            match watch.poll() {
                Ok(Some(events)) => {
                    for (path, removed) in events {
                        if removed && !*shell {
                            changed |= snapshot.remove(&path);
                            unsaved |= changed;
                        } else if pending.len() < 4096 && (*shell || !excluded(&path)) {
                            pending.insert((path, *shell), Instant::now());
                        } else if pending.len() >= 4096 {
                            snapshot.warn("Recent notification burst exceeded its queue; bounded reconciliation requested.".into());
                            rescan = true;
                            changed = true;
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    snapshot.warn(error);
                    rescan = true;
                    changed = true;
                }
            }
        }
        let ready: Vec<_> = pending
            .iter()
            .filter(|(_, time)| time.elapsed() >= Duration::from_millis(500))
            .map(|(path, _)| path.clone())
            .collect();
        for (path, shell_entry) in ready {
            pending.remove(&(path.clone(), shell_entry));
            #[cfg(windows)]
            if shell_entry {
                if let Err(error) = windows::import_link(&path, &roots, &mut snapshot) {
                    snapshot.warn(error);
                }
                changed = true;
                unsaved = true;
                continue;
            }
            match inspect(
                &path,
                Activity::Modified,
                Some(milliseconds(SystemTime::now())),
                Some(&roots),
            ) {
                Ok(Some(item)) => {
                    let same = snapshot.items.iter().any(|old| {
                        key(&old.path) == key(&item.path)
                            && old.modified_ms == item.modified_ms
                            && old.size == item.size
                    });
                    if !same {
                        changed |= snapshot.record(item);
                        unsaved = true;
                    }
                }
                Ok(None) => {
                    if path.is_dir() && !excluded(&path) {
                        match fs::symlink_metadata(&path) {
                            Ok(metadata) if !metadata.file_type().is_symlink() => {
                                let traversable = if files::reparse(&metadata) {
                                    windows_cloud_directory(&path)
                                } else {
                                    Ok(true)
                                };
                                match traversable {
                                    Ok(true) => {
                                        scan(&mut snapshot, &roots, &[path], &stop);
                                        changed = true;
                                        unsaved = true;
                                    }
                                    Ok(false) => {}
                                    Err(error) => {
                                        snapshot.warn(error);
                                        changed = true;
                                    }
                                }
                            }
                            Ok(_) => {}
                            Err(error) => {
                                snapshot.warn(format!("Inspect changed Recent folder: {error}"));
                                changed = true;
                            }
                        }
                    } else {
                        changed |= snapshot.remove(&path);
                        unsaved |= changed;
                    }
                }
                Err(error) => {
                    snapshot.warn(error);
                    changed = true;
                }
            }
        }
        if rescan {
            // Reconcile deletions/renames and overflow without scanning on the UI thread.
            let existing: Vec<_> = snapshot
                .items
                .iter()
                .map(|item| item.path.clone())
                .collect();
            for path in existing {
                if !path.exists() {
                    snapshot.remove(&path);
                }
            }
            scan(&mut snapshot, &roots, &roots, &stop);
            #[cfg(windows)]
            if let Some(shell) = &shell
                && !roots.is_empty()
                && let Err(error) = windows::import_shell(shell, &roots, &mut snapshot, &stop)
            {
                snapshot.warn(error);
            }
            rescan = false;
            changed = true;
            unsaved = true;
        }
        if unsaved && writable && saved_at.elapsed() >= Duration::from_secs(1) {
            if let Err(error) = save(&config.history, &snapshot.items) {
                snapshot.warn(error);
                writable = false;
                changed = true;
            }
            unsaved = false;
            saved_at = Instant::now();
        }
        if changed {
            match sender.try_send(snapshot.clone()) {
                Ok(()) => {
                    changed = false;
                    wake();
                }
                Err(mpsc::TrySendError::Full(_)) => wake(),
                Err(mpsc::TrySendError::Disconnected(_)) => break,
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    for request in requests.try_iter() {
        if let Request::Record(paths, activity) = request {
            for path in paths {
                if let Some(item) = inspect(
                    &path,
                    activity,
                    Some(milliseconds(SystemTime::now())),
                    config.restricted.then_some(roots.as_slice()),
                )? {
                    snapshot.record(item);
                    unsaved = true;
                }
            }
        }
    }
    if unsaved && writable {
        save(&config.history, &snapshot.items)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_defaults_custom_empty_roots_and_fixture_authority_remain_distinct() {
        let settings = PathBuf::from(r"D:\settings\preferences.txt");
        let defaults = Config::for_app(&settings, None, Vec::new(), None);
        assert!(defaults.discover_roots && defaults.windows_recent && !defaults.restricted);
        let empty = Config::for_app(&settings, None, Vec::new(), Some(Vec::new()));
        assert!(!empty.discover_roots && empty.roots.is_empty());
        let root = PathBuf::from(r"D:\synthetic");
        let fixture = Config::for_app(
            &settings,
            Some(&root),
            Vec::new(),
            Some(vec![PathBuf::from(r"C:\private")]),
        );
        assert_eq!(fixture.roots, vec![root]);
        assert!(!fixture.discover_roots && !fixture.windows_recent && fixture.restricted);
    }

    pub(super) struct Fixture {
        pub root: PathBuf,
    }
    impl Fixture {
        pub fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::current_dir()
                .unwrap()
                .join(format!("recent-tests-{}-{nonce}", std::process::id()));
            fs::create_dir(&root).unwrap();
            Self {
                root: fs::canonicalize(root).unwrap(),
            }
        }
        pub fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            path
        }
        fn config(&self) -> Config {
            Config {
                roots: vec![self.root.clone()],
                history: self.root.join("recent-activity.json"),
                legacy: Vec::new(),
                windows_recent: false,
                discover_roots: false,
                restricted: true,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    fn wait(monitor: &mut Monitor, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let started = Instant::now();
        let mut latest = Snapshot::default();
        loop {
            if let Some(snapshot) = monitor.poll().unwrap() {
                latest = snapshot;
                if predicate(&latest) {
                    return latest;
                }
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "Recent timeout: {:?} / {:?}",
                latest.items,
                latest.warnings
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn recent_catalog_is_deduplicated_newest_first_bounded_and_roundtrips() {
        let fixture = Fixture::new();
        let path = fixture.write("first.xlsx", b"synthetic sheet");
        let first = inspect(&path, Activity::Modified, Some(10), None)
            .unwrap()
            .unwrap();
        let mut snapshot = Snapshot::default();
        assert!(snapshot.record(first.clone()));
        let mut latest = first.clone();
        latest.at_ms = 20;
        latest.activity = Activity::Opened;
        assert!(snapshot.record(latest.clone()));
        assert!(!snapshot.record(first));
        assert_eq!(snapshot.items, vec![latest.clone()]);
        for index in 0..150 {
            let mut item = latest.clone();
            item.path = fixture.root.join(format!("{index}.md"));
            item.at_ms = 30 + index;
            snapshot.record(item);
        }
        assert_eq!(snapshot.items.len(), MAX_ITEMS);
        assert_eq!(snapshot.items[0].at_ms, 179);
        assert!(
            snapshot
                .items
                .windows(2)
                .all(|pair| pair[0].at_ms >= pair[1].at_ms)
        );
        let history = fixture.root.join("history.json");
        save(&history, &snapshot.items).unwrap();
        assert_eq!(load(&history).unwrap(), snapshot.items);
        save(&history, &[]).unwrap();
        assert!(load(&history).unwrap().is_empty());
    }

    #[test]
    fn recent_scan_filters_cache_temp_hidden_and_old_documents_and_stops_at_budget() {
        let fixture = Fixture::new();
        for name in [
            "note.md",
            "slides.pptx",
            "sheet.xlsx",
            "report.pdf",
            "page.html",
            "plain.txt",
            "data.csv",
            "bundle.zip",
            "uppercase.ZIP",
        ] {
            fixture.write(name, b"synthetic");
        }
        for name in [
            "target/cache.md",
            "node_modules/pkg/readme.md",
            ".git/hidden.md",
            "build/generated.html",
            "~$draft.docx",
            "image.png",
            "cache.txt.tmp",
            "bundle.7z",
        ] {
            fixture.write(name, b"exclude");
        }
        let old = fixture.write("old.md", b"old");
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::now() - LOOKBACK - Duration::from_secs(60)),
            )
            .unwrap();
        let stop = AtomicBool::new(false);
        let roots = [fixture.root.clone()];
        let mut snapshot = Snapshot::default();
        scan(&mut snapshot, &roots, &roots, &stop);
        assert_eq!(snapshot.items.len(), 9, "{:?}", snapshot.items);
        assert!(snapshot.warnings.is_empty(), "{:?}", snapshot.warnings);
        let mut bounded = Snapshot::default();
        scan_limited(&mut bounded, &roots, &roots, &stop, 2);
        assert!(bounded.items.len() <= 1);
        assert!(
            bounded
                .warnings
                .iter()
                .any(|warning| warning.contains("partial"))
        );
        let mut cancelled = Snapshot::default();
        scan(&mut cancelled, &roots, &roots, &AtomicBool::new(true));
        assert!(cancelled.items.is_empty());
    }

    #[test]
    fn recent_scope_excludes_outside_records_and_migrates_without_fabricated_open_times() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        let forbidden = outside.write("outside.md", b"not in scope");
        let path = fixture.write("legacy.docx", b"synthetic");
        let expected = inspect(&path, Activity::Modified, None, None)
            .unwrap()
            .unwrap()
            .at_ms;
        let mut config = fixture.config();
        config.legacy = vec![path.clone(), forbidden.clone()];
        let mut monitor = Monitor::start(config, || {});
        let snapshot = wait(&mut monitor, |snapshot| {
            !snapshot.scanning && snapshot.roots.is_some()
        });
        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(snapshot.items[0].at_ms, expected);
        assert_eq!(snapshot.items[0].activity, Activity::Modified);
        monitor.record(forbidden.clone(), Activity::Opened).unwrap();
        monitor.record(path.clone(), Activity::Previewed).unwrap();
        let snapshot = wait(&mut monitor, |snapshot| {
            snapshot
                .items
                .first()
                .is_some_and(|item| item.activity == Activity::Previewed)
        });
        assert_eq!(snapshot.items.len(), 1);
        monitor.record(path.clone(), Activity::Opened).unwrap();
        monitor.shutdown().unwrap();
        let history = load(&fixture.config().history).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].activity, Activity::Opened);
        let mut restarted = Monitor::start(fixture.config(), || {});
        let snapshot = wait(&mut restarted, |snapshot| {
            !snapshot.scanning && snapshot.roots.is_some()
        });
        assert_eq!(snapshot.items[0], history[0]);
        restarted.shutdown().unwrap();
    }

    #[test]
    fn recent_invalid_history_is_reported_and_never_overwritten() {
        let fixture = Fixture::new();
        let path = fixture.write("note.txt", b"synthetic");
        let history = fixture.write("recent-activity.json", b"invalid history");
        let mut monitor = Monitor::start(fixture.config(), || {});
        let snapshot = wait(&mut monitor, |snapshot| {
            snapshot
                .warnings
                .iter()
                .any(|warning| warning.contains("not overwritten"))
        });
        assert!(!snapshot.warnings.is_empty());
        monitor.record(path, Activity::Opened).unwrap();
        monitor.shutdown().unwrap();
        assert_eq!(fs::read(history).unwrap(), b"invalid history");
    }

    #[cfg(windows)]
    #[test]
    fn recent_native_monitor_observes_create_write_atomic_replace_rename_delete_and_stops() {
        let fixture = Fixture::new();
        let first = fixture.write("first.md", b"first");
        let mut monitor = Monitor::start(fixture.config(), || {});
        let initial = wait(&mut monitor, |snapshot| {
            !snapshot.scanning && snapshot.roots.is_some()
        });
        let original_time = initial.items[0].at_ms;
        thread::sleep(Duration::from_millis(20));
        fs::write(&first, b"updated outside Commander").unwrap();
        wait(&mut monitor, |snapshot| {
            snapshot
                .items
                .iter()
                .any(|item| item.path == first && item.at_ms > original_time)
        });
        let second = fixture.write("second.ZIP", b"synthetic");
        wait(&mut monitor, |snapshot| {
            snapshot
                .items
                .first()
                .is_some_and(|item| item.path == second)
        });
        let replacement = fixture.write("replace.tmp", b"atomic replacement");
        fs::rename(replacement, &second).unwrap();
        wait(&mut monitor, |snapshot| {
            snapshot
                .items
                .iter()
                .any(|item| item.path == second && item.size == 18)
        });
        let renamed = fixture.root.join("renamed.zip");
        fs::rename(&second, &renamed).unwrap();
        let snapshot = wait(&mut monitor, |snapshot| {
            snapshot.items.iter().any(|item| item.path == renamed)
                && snapshot.items.iter().all(|item| item.path != second)
        });
        assert_eq!(snapshot.items.len(), 2);
        fs::remove_file(&renamed).unwrap();
        wait(&mut monitor, |snapshot| {
            snapshot.items.iter().all(|item| item.path != renamed)
        });
        let stopped = Instant::now();
        monitor.shutdown().unwrap();
        assert!(stopped.elapsed() < Duration::from_secs(2));
        assert!(
            load(&fixture.config().history)
                .unwrap()
                .iter()
                .all(|item| item.path != renamed)
        );
    }

    #[cfg(windows)]
    #[test]
    fn recent_reconfigures_live_and_rejects_invalid_configuration_without_losing_watch() {
        let fixture = Fixture::new();
        let second = Fixture::new();
        let mut config = fixture.config();
        config.restricted = false;
        let mut monitor = Monitor::start(config, || {});
        wait(&mut monitor, |snapshot| {
            !snapshot.scanning && snapshot.roots.is_some()
        });
        monitor
            .configure(vec![second.root.join("missing")])
            .unwrap();
        let rejected = wait(&mut monitor, |snapshot| {
            snapshot.configuration_revision == 1
        });
        assert_eq!(
            rejected.roots.as_ref().unwrap(),
            &vec![fixture.root.clone()]
        );
        assert!(
            rejected
                .warnings
                .iter()
                .any(|warning| warning.contains("unchanged"))
        );
        let still_watched = fixture.write("still-watched.md", b"synthetic");
        wait(&mut monitor, |snapshot| {
            snapshot.items.iter().any(|item| item.path == still_watched)
        });
        monitor.configure(vec![second.root.clone()]).unwrap();
        wait(&mut monitor, |snapshot| {
            snapshot.configuration_revision == 2
                && snapshot.roots.as_ref() == Some(&vec![second.root.clone()])
        });
        let now_watched = second.write("now-watched.md", b"synthetic");
        let no_longer_watched = fixture.write("no-longer-watched.md", b"synthetic");
        let snapshot = wait(&mut monitor, |snapshot| {
            snapshot.items.iter().any(|item| item.path == now_watched)
        });
        assert!(
            !snapshot
                .items
                .iter()
                .any(|item| item.path == no_longer_watched)
        );
        monitor.shutdown().unwrap();
    }
}
