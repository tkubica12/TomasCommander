use crate::files::{self, DirectoryStats, FileResult, Fingerprint, Scope};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_WATCHES: usize = 128;

#[derive(Default)]
pub struct DirectoryCache {
    state: Mutex<State>,
    #[cfg(test)]
    scans: std::sync::atomic::AtomicUsize,
}

#[derive(Default)]
struct State {
    entries: BTreeMap<PathBuf, Cached>,
    sequence: u64,
    pending: Changes,
}

struct Cached {
    watch: ChangeWatch,
    identity: Fingerprint,
    generation: u64,
    last_used: u64,
    stats: Option<DirectoryStats>,
}

#[derive(Default)]
pub struct Changes {
    pub paths: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub all: bool,
}

impl State {
    fn next(&mut self) -> FileResult<u64> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("Directory cache generation exhausted; restart the app.")?;
        Ok(self.sequence)
    }

    fn poll(&mut self, only: Option<&Path>) -> FileResult<Changes> {
        let mut changes = Changes::default();
        let mut remove = Vec::new();
        for (path, entry) in &mut self.entries {
            if only.is_some_and(|only| only != path) {
                continue;
            }
            match entry.watch.changed().and_then(|changed| {
                files::fingerprint(path).map(|identity| (changed, identity != entry.identity))
            }) {
                Ok((false, false)) => (),
                Ok((_, true)) => {
                    changes.paths.push(path.clone());
                    remove.push(path.clone());
                }
                Ok((true, false)) => {
                    entry.stats = None;
                    changes.paths.push(path.clone());
                }
                Err(error) => {
                    changes.paths.push(path.clone());
                    changes.warnings.push(format!(
                        "Directory monitoring lost for {}: {error}. Cached values discarded.",
                        files::display_path(path)
                    ));
                    remove.push(path.clone());
                }
            }
        }
        for path in &changes.paths {
            let generation = self.next()?;
            if let Some(entry) = self.entries.get_mut(path) {
                entry.generation = generation;
            }
        }
        for path in remove {
            self.entries.remove(&path);
        }
        for path in &changes.paths {
            if !self.pending.all && !self.pending.paths.contains(path) {
                if self.pending.paths.len() == MAX_WATCHES * 2 {
                    self.pending.paths.clear();
                    self.pending.all = true;
                } else {
                    self.pending.paths.push(path.clone());
                }
            }
        }
        for warning in &changes.warnings {
            if self.pending.warnings.len() < MAX_WATCHES {
                self.pending.warnings.push(warning.clone());
            }
        }
        Ok(changes)
    }
}

impl DirectoryCache {
    pub fn poll(&self) -> FileResult<Changes> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Directory cache lock failed; cached values are unavailable.")?;
        state.poll(None)?;
        Ok(std::mem::take(&mut state.pending))
    }

    pub fn clear(&self) -> FileResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Directory cache lock failed; restart before using cached values.")?;
        state.entries.clear();
        state.pending.all = true;
        Ok(())
    }

    pub fn calculate(
        &self,
        scope: &Scope,
        path: &Path,
        stale: impl Fn() -> bool,
    ) -> FileResult<DirectoryStats> {
        let path = scope.resolve(path)?;
        for _ in 0..2 {
            if stale() {
                return Err("Folder calculation was superseded.".into());
            }
            let generation = {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| "Directory cache lock failed.")?;
                let changes = state.poll(Some(&path))?;
                if !changes.warnings.is_empty() {
                    return Err(changes.warnings.join("\n"));
                }
                let used = state.next()?;
                if let Some(entry) = state.entries.get_mut(&path) {
                    entry.last_used = used;
                    if let Some(stats) = &entry.stats {
                        return Ok(stats.clone());
                    }
                    entry.generation
                } else {
                    let watch = match ChangeWatch::new(&path) {
                        Ok(watch) => watch,
                        Err(error) => {
                            drop(state);
                            #[cfg(test)]
                            self.scans
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let mut stats = files::directory_stats(scope, &path, &stale)?;
                            stats.warnings.push(format!(
                                "Live cache unavailable: {error}. Refresh to recheck these values."
                            ));
                            return Ok(stats);
                        }
                    };
                    let identity = files::fingerprint(&path)?;
                    if state.entries.len() >= MAX_WATCHES {
                        let oldest = state
                            .entries
                            .iter()
                            .min_by_key(|(_, entry)| entry.last_used)
                            .map(|(path, _)| path.clone());
                        if let Some(oldest) = oldest {
                            state.entries.remove(&oldest);
                        }
                    }
                    state.entries.insert(
                        path.clone(),
                        Cached {
                            watch,
                            identity,
                            generation: used,
                            last_used: used,
                            stats: None,
                        },
                    );
                    used
                }
            };
            #[cfg(test)]
            self.scans
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let stats = files::directory_stats(scope, &path, &stale)?;
            if stale() {
                return Err("Folder calculation was superseded.".into());
            }
            let mut state = self
                .state
                .lock()
                .map_err(|_| "Directory cache lock failed.")?;
            let changes = state.poll(Some(&path))?;
            if !changes.warnings.is_empty() {
                return Err(changes.warnings.join("\n"));
            }
            if let Some(entry) = state.entries.get_mut(&path)
                && entry.generation == generation
            {
                if !stats.partial() {
                    entry.stats = Some(stats.clone());
                }
                return Ok(stats);
            }
        }
        Err("Folder kept changing during calculation; refresh after writes finish.".into())
    }
}

#[cfg(windows)]
struct ChangeWatch(windows::Win32::Foundation::HANDLE);

// Notification handles are thread-independent; access and closure use the cache mutex.
#[cfg(windows)]
unsafe impl Send for ChangeWatch {}

#[cfg(windows)]
impl ChangeWatch {
    fn new(path: &Path) -> FileResult<Self> {
        use std::{os::windows::ffi::OsStrExt, path::Prefix};
        use windows::{
            Win32::Storage::FileSystem::{
                FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_DIR_NAME,
                FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
                FILE_NOTIFY_CHANGE_SECURITY, FILE_NOTIFY_CHANGE_SIZE, FindFirstChangeNotificationW,
                GetDriveTypeW,
            },
            core::PCWSTR,
        };
        let disk = match path.components().next() {
            Some(std::path::Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(disk) | Prefix::VerbatimDisk(disk) => disk,
                _ => return Err("Remote paths require explicit refresh, not live cache.".into()),
            },
            _ => return Err("Directory monitoring needs an absolute local path.".into()),
        };
        let root: Vec<u16> = format!("{}:\\", char::from(disk))
            .encode_utf16()
            .chain(Some(0))
            .collect();
        if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } != 3 {
            return Err("Only fixed local drives use live aggregate caching.".into());
        }
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let filters = FILE_NOTIFY_CHANGE_FILE_NAME
            | FILE_NOTIFY_CHANGE_DIR_NAME
            | FILE_NOTIFY_CHANGE_SIZE
            | FILE_NOTIFY_CHANGE_LAST_WRITE
            | FILE_NOTIFY_CHANGE_ATTRIBUTES
            | FILE_NOTIFY_CHANGE_SECURITY;
        unsafe { FindFirstChangeNotificationW(PCWSTR(name.as_ptr()), true, filters) }
            .map(Self)
            .map_err(|error| format!("Start directory notifications: {error}"))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Write,
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct Fixture {
        root: PathBuf,
        owned: Vec<PathBuf>,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "tomas-cache-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            Self {
                root,
                owned: Vec::new(),
            }
        }

        fn directory(&mut self, name: &str) -> PathBuf {
            let path = self.root.join(name);
            fs::create_dir(&path).unwrap();
            self.owned.push(path.clone());
            path
        }

        fn file(&mut self, name: &str, contents: &[u8]) -> PathBuf {
            let path = self.root.join(name);
            write(&path, contents);
            self.owned.push(path.clone());
            path
        }

        fn scope(&self) -> Scope {
            Scope::fixture(&self.root).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            for path in self.owned.iter().rev() {
                let result = if path.is_dir() {
                    fs::remove_dir(path)
                } else if path.exists() {
                    fs::remove_file(path)
                } else {
                    continue;
                };
                result.unwrap();
            }
            fs::remove_dir(&self.root).unwrap();
        }
    }

    fn write(path: &Path, contents: &[u8]) {
        let mut file = fs::File::create(path).unwrap();
        file.write_all(contents).unwrap();
        file.sync_all().unwrap();
    }

    fn wait_for_change(cache: &DirectoryCache, path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let changes = cache.poll().unwrap();
            assert!(changes.warnings.is_empty(), "{:?}", changes.warnings);
            if changes.paths.contains(&path.to_path_buf()) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Missing native notification for {}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn native_cache_reuses_and_invalidates_size_and_latest_descendant_time() {
        let mut fixture = Fixture::new();
        fixture.directory("child");
        fixture.directory(r"child\nested");
        let file = fixture.file(r"child\nested\leaf.txt", b"initial");
        let scope = fixture.scope();
        let root = scope.resolve(&fixture.root.join("child")).unwrap();
        let cache = DirectoryCache::default();
        let first = cache.calculate(&scope, &root, || false).unwrap();
        assert!(!first.partial(), "{:?}", first.warnings);
        let scans = cache.scans.load(Ordering::Relaxed);
        let second = cache.calculate(&scope, &root, || false).unwrap();
        assert_eq!(first.size, second.size);
        assert_eq!(first.modified, second.modified);
        assert_eq!(
            scans,
            cache.scans.load(Ordering::Relaxed),
            "Warm lookup rescanned"
        );
        std::thread::sleep(Duration::from_millis(20));
        write(&file, b"changed much longer contents");
        wait_for_change(&cache, &root);
        let changed = cache.calculate(&scope, &root, || false).unwrap();
        let reference = files::directory_stats(&scope, &root, || false).unwrap();
        assert_eq!(changed.size, reference.size);
        assert_eq!(changed.modified, reference.modified);
        assert_ne!(changed.size, first.size);
        assert!(changed.modified > first.modified);
        let created = fixture.file(r"child\nested\new.txt", b"new");
        wait_for_change(&cache, &root);
        assert_eq!(
            cache.calculate(&scope, &root, || false).unwrap().size,
            changed.size + 3
        );
        let renamed = created.with_file_name("renamed.txt");
        fs::rename(&created, &renamed).unwrap();
        fixture.owned.push(renamed.clone());
        wait_for_change(&cache, &root);
        cache.calculate(&scope, &root, || false).unwrap();
        fs::remove_file(&renamed).unwrap();
        wait_for_change(&cache, &root);
        assert_eq!(
            cache.calculate(&scope, &root, || false).unwrap().size,
            changed.size
        );
        cache.clear().unwrap();
        assert!(cache.poll().unwrap().all);
        assert!(cache.state.lock().unwrap().entries.is_empty());
        cache.calculate(&scope, &root, || false).unwrap();
        assert!(cache.scans.load(Ordering::Relaxed) > scans);
    }

    #[test]
    fn cache_detects_changes_during_scan_and_preserves_pending_ui_invalidation() {
        use std::cell::Cell;
        let mut fixture = Fixture::new();
        fixture.directory("child");
        fixture.directory(r"child\nested");
        let file = fixture.file(r"child\nested\leaf.txt", b"before");
        let scope = fixture.scope();
        let path = scope.resolve(&fixture.root.join("child")).unwrap();
        let cache = DirectoryCache::default();
        let calls = Cell::new(0);
        let result = cache
            .calculate(&scope, &path, || {
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    write(&file, b"after mutation while calculation is pending");
                }
                false
            })
            .unwrap();
        assert_eq!(result.size, fs::metadata(&file).unwrap().len());
        write(&file, b"another external mutation");
        let fresh = cache.calculate(&scope, &path, || false).unwrap();
        assert_eq!(fresh.size, fs::metadata(&file).unwrap().len());
        assert!(cache.poll().unwrap().paths.contains(&path));
        assert!(cache.calculate(&scope, &path, || true).is_err());
    }

    #[test]
    fn cache_watcher_count_is_bounded_and_restart_has_no_trusted_values() {
        let mut fixture = Fixture::new();
        let scope = fixture.scope();
        let cache = DirectoryCache::default();
        for index in 0..MAX_WATCHES + 2 {
            let path = fixture.directory(&format!("directory-{index}"));
            cache.calculate(&scope, &path, || false).unwrap();
        }
        assert_eq!(cache.state.lock().unwrap().entries.len(), MAX_WATCHES);
        let restarted = DirectoryCache::default();
        assert!(restarted.state.lock().unwrap().entries.is_empty());
    }

    #[test]
    fn replaced_watched_root_discards_identity_and_reports_monitoring_loss() {
        let mut fixture = Fixture::new();
        let original = fixture.directory("child");
        fixture.file(r"child\leaf.txt", b"original");
        let scope = fixture.scope();
        let canonical = scope.resolve(&original).unwrap();
        let cache = DirectoryCache::default();
        cache.calculate(&scope, &canonical, || false).unwrap();
        let moved = fixture.root.join("moved");
        fs::rename(&original, &moved).unwrap();
        fixture.owned.push(moved.clone());
        fixture.owned.push(moved.join("leaf.txt"));
        let changes = cache.poll().unwrap();
        assert!(changes.paths.contains(&canonical));
        assert!(!changes.warnings.is_empty());
        assert!(cache.state.lock().unwrap().entries.is_empty());
        fixture.directory("child");
        fixture.file(r"child\replacement.txt", b"replacement is different");
        let replacement = cache.calculate(&scope, &original, || false).unwrap();
        assert_eq!(replacement.size, b"replacement is different".len() as u64);
        assert!(ChangeWatch::new(Path::new(r"\\server\share\directory")).is_err());
    }

    #[test]
    fn queued_invalidation_overflow_requests_full_refresh() {
        let mut fixture = Fixture::new();
        let path = fixture.directory("child");
        let scope = fixture.scope();
        let path = scope.resolve(&path).unwrap();
        let cache = DirectoryCache::default();
        cache.calculate(&scope, &path, || false).unwrap();
        {
            let mut state = cache.state.lock().unwrap();
            state.pending.paths = (0..MAX_WATCHES * 2)
                .map(|index| PathBuf::from(format!("synthetic-pending-{index}")))
                .collect();
        }
        fixture.file(r"child\new.txt", b"event");
        let changes = cache.poll().unwrap();
        assert!(changes.all);
        assert!(changes.paths.is_empty());
        assert!(
            cache
                .state
                .lock()
                .unwrap()
                .entries
                .get(&path)
                .is_none_or(|entry| entry.stats.is_none())
        );
    }
}
#[cfg(windows)]
impl ChangeWatch {
    fn changed(&self) -> FileResult<bool> {
        use windows::Win32::{
            Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
            Storage::FileSystem::FindNextChangeNotification,
            System::Threading::WaitForSingleObject,
        };
        match unsafe { WaitForSingleObject(self.0, 0) } {
            WAIT_TIMEOUT => Ok(false),
            WAIT_OBJECT_0 => {
                unsafe { FindNextChangeNotification(self.0) }
                    .map_err(|error| format!("Rearm directory notifications: {error}"))?;
                Ok(true)
            }
            WAIT_FAILED => Err(format!(
                "Wait for directory notifications: {}",
                windows::core::Error::from_thread()
            )),
            _ => Err("Unexpected directory notification wait result.".into()),
        }
    }
}

#[cfg(windows)]
impl Drop for ChangeWatch {
    fn drop(&mut self) {
        if let Err(error) =
            unsafe { windows::Win32::Storage::FileSystem::FindCloseChangeNotification(self.0) }
        {
            eprintln!("Close directory notifications: {error}");
        }
    }
}

#[cfg(not(windows))]
struct ChangeWatch;

#[cfg(not(windows))]
impl ChangeWatch {
    fn new(_: &Path) -> FileResult<Self> {
        Err("Live aggregate monitoring requires Windows.".into())
    }

    fn changed(&self) -> FileResult<bool> {
        Err("Live aggregate monitoring requires Windows.".into())
    }
}
