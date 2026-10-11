use crate::{
    archives,
    changes::{self, Snapshot},
    files::{self, FileResult, Fingerprint, Outcome, Scope},
    journal::Journal,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const LIMIT: usize = 32 * 1_048_576;
type Members = BTreeMap<String, Option<Vec<u8>>>;

pub enum Request {
    Create {
        sources: Vec<PathBuf>,
        target: PathBuf,
    },
    Extract {
        archive: PathBuf,
        prefix: String,
        selected: Vec<String>,
        destination: PathBuf,
        move_entries: bool,
    },
    Insert {
        archive: PathBuf,
        prefix: String,
        sources: Vec<PathBuf>,
        move_sources: bool,
    },
}

pub struct Plan {
    pub id: u64,
    pub label: &'static str,
    key: String,
    snapshots: Vec<Snapshot>,
    folders: Vec<(PathBuf, Fingerprint)>,
    parent: PathBuf,
    parent_identity: Fingerprint,
    target: Option<PathBuf>,
    replacement: bool,
    stage: PathBuf,
    zip: Option<Vec<u8>>,
    outputs: Vec<(PathBuf, Option<Vec<u8>>)>,
    member_rows: Vec<String>,
    remove_sources: bool,
}

fn cancelled(cancel: &AtomicBool) -> FileResult<()> {
    if cancel.load(Ordering::Acquire) {
        Err("ZIP operation cancelled; no source deletion before verified destination.".into())
    } else {
        Ok(())
    }
}

pub fn read_entry(
    scope: &Scope,
    archive: &Path,
    name: &str,
    cancel: impl Fn() -> bool,
) -> FileResult<Vec<u8>> {
    let snapshot = Snapshot::read(scope, archive, LIMIT)?;
    archives::inventory_bytes(&snapshot.bytes, &cancel)?;
    let result = entry_bytes(&snapshot.bytes, name, &cancel)?;
    if files::fingerprint(&snapshot.path)? != snapshot.identity {
        return Err("ZIP changed during entry preview.".into());
    }
    Ok(result)
}

fn entry_bytes(bytes: &[u8], name: &str, cancel: impl Fn() -> bool) -> FileResult<Vec<u8>> {
    archives::validate_name(name)?;
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| format!("Read ZIP: {error}"))?;
    let mut entry = archive
        .by_name(name)
        .map_err(|error| format!("Read exact ZIP entry: {error}"))?;
    if entry.is_dir() || entry.size() > 16 * 1_048_576 {
        return Err("ZIP entry is a folder or exceeds 16 MiB.".into());
    }
    let expected = entry.size();
    let mut result = Vec::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        if cancel() {
            return Err("ZIP entry read cancelled.".into());
        }
        let count = entry
            .read(&mut buffer)
            .map_err(|error| format!("Decode ZIP entry / CRC verification: {error}"))?;
        if count == 0 {
            break;
        }
        if result.len() + count > expected as usize {
            return Err("ZIP entry expanded beyond its declared bound.".into());
        }
        result.extend_from_slice(&buffer[..count]);
    }
    if result.len() as u64 != expected {
        return Err("ZIP entry length verification failed.".into());
    }
    Ok(result)
}

fn materialize(snapshot: &Snapshot, cancel: &AtomicBool) -> FileResult<Members> {
    let inventory = archives::inventory_bytes(&snapshot.bytes, || cancel.load(Ordering::Acquire))?;
    let mut total = 0;
    let mut members = Members::new();
    for entry in inventory.entries {
        cancelled(cancel)?;
        total += entry.size;
        if total > LIMIT as u64 {
            return Err(
                "ZIP effects are bounded to 32 MiB expanded data; source unchanged.".into(),
            );
        }
        let data = if entry.directory {
            None
        } else {
            Some(entry_bytes(&snapshot.bytes, &entry.path, || {
                cancel.load(Ordering::Acquire)
            })?)
        };
        members.insert(entry.path, data);
    }
    Ok(members)
}

struct LimitedBuffer(Cursor<Vec<u8>>);
impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.position().saturating_add(bytes.len() as u64) > LIMIT as u64 {
            return Err(std::io::Error::other("ZIP output exceeds 32 MiB"));
        }
        self.0.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
impl Seek for LimitedBuffer {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(position)
    }
}

fn encode(members: &Members, cancel: &AtomicBool) -> FileResult<Vec<u8>> {
    let mut writer = zip::ZipWriter::new(LimitedBuffer(Cursor::new(Vec::new())));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in members {
        cancelled(cancel)?;
        if let Some(data) = data {
            writer
                .start_file(name, options)
                .map_err(|error| format!("Start ZIP member: {error}"))?;
            for chunk in data.chunks(64 * 1024) {
                cancelled(cancel)?;
                writer
                    .write_all(chunk)
                    .map_err(|error| format!("Encode ZIP member: {error}"))?;
            }
        } else {
            writer
                .add_directory(format!("{name}/"), options)
                .map_err(|error| format!("Encode ZIP directory: {error}"))?;
        }
    }
    let bytes = writer
        .finish()
        .map_err(|error| format!("Finish bounded ZIP: {error}"))?
        .0
        .into_inner();
    archives::inventory_bytes(&bytes, || cancel.load(Ordering::Acquire))?;
    Ok(bytes)
}

type Gathered = (Members, Vec<Snapshot>, Vec<(PathBuf, Fingerprint)>);

fn gather(
    scope: &Scope,
    sources: &[PathBuf],
    prefix: &str,
    cancel: &AtomicBool,
) -> FileResult<Gathered> {
    let mut members = Members::new();
    let mut snapshots = Vec::new();
    let mut folders = Vec::new();
    let mut stack = Vec::new();
    let mut total = 0;
    for source in sources {
        let path = scope.mutation_source(source)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("ZIP source filename must be Unicode.")?;
        stack.push((path.clone(), format!("{prefix}{name}")));
    }
    if stack.is_empty() {
        return Err("Select files or folders to archive.".into());
    }
    let mut folded = BTreeSet::new();
    let mut seen_paths = BTreeSet::new();
    while let Some((path, name)) = stack.pop() {
        cancelled(cancel)?;
        let name = archives::validate_name(&name)?;
        if !folded.insert(name.to_lowercase()) || folded.len() > 10_000 {
            return Err("Duplicate/case-ambiguous source or ZIP entry limit exceeded.".into());
        }
        let path = scope.mutation_source(&path)?;
        if !seen_paths.insert(path.clone()) {
            return Err("Overlapping ZIP source selections are refused.".into());
        }
        let identity = files::fingerprint(&path)?;
        if identity.directory {
            folders.push((path.clone(), identity));
            members.insert(name.clone(), None);
            for entry in
                fs::read_dir(&path).map_err(|error| format!("Enumerate ZIP source: {error}"))?
            {
                let entry = entry.map_err(|error| format!("Read ZIP source: {error}"))?;
                let child = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| "ZIP member filename must be Unicode.")?;
                stack.push((entry.path(), format!("{name}/{child}")));
            }
        } else {
            let snapshot = Snapshot::read(scope, &path, 16 * 1_048_576)?;
            total += snapshot.bytes.len();
            if total > LIMIT {
                return Err("ZIP source data exceeds the 32 MiB operation bound.".into());
            }
            members.insert(name, Some(snapshot.bytes.clone()));
            snapshots.push(snapshot);
        }
    }
    Ok((members, snapshots, folders))
}

fn destination(scope: &Scope, root: &Path, relative: &str) -> FileResult<PathBuf> {
    archives::validate_name(relative)?;
    let mut path = scope.resolve(root)?;
    for part in relative.split('/') {
        path.push(part);
    }
    if path
        .try_exists()
        .map_err(|error| format!("Inspect exact ZIP destination: {error}"))?
    {
        return Err(format!(
            "ZIP destination already exists; no overwrite/merge: {}",
            files::display_path(&path)
        ));
    }
    Ok(path)
}

pub fn prepare(scope: &Scope, id: u64, request: Request, cancel: &AtomicBool) -> FileResult<Plan> {
    cancelled(cancel)?;
    let key = changes::identity()?;
    let mut member_rows = Vec::new();
    let (label, snapshots, folders, parent, target, replacement, zip, outputs, remove_sources) =
        match request {
            Request::Create { sources, target } => {
                let parent = scope.resolve(
                    target
                        .parent()
                        .ok_or("ZIP destination parent is missing.")?,
                )?;
                let name = target
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or("ZIP output name must be Unicode.")?;
                if !name.to_ascii_lowercase().ends_with(".zip") || name.contains(['/', '\\']) {
                    return Err("Choose a single .zip filename.".into());
                }
                let target = destination(scope, &parent, name)?;
                let (members, snapshots, folders) = gather(scope, &sources, "", cancel)?;
                member_rows.extend(members.keys().map(|name| format!("Add ZIP member: {name}")));
                if folders.iter().any(|(folder, _)| target.starts_with(folder)) {
                    return Err(
                        "Do not create a ZIP inside one of its selected source folders.".into(),
                    );
                }
                (
                    "Create ZIP",
                    snapshots,
                    folders,
                    parent,
                    Some(target),
                    false,
                    Some(encode(&members, cancel)?),
                    Vec::new(),
                    false,
                )
            }
            Request::Extract {
                archive,
                prefix,
                selected,
                destination: root,
                move_entries,
            } => {
                let snapshot = Snapshot::read(scope, &archive, LIMIT)?;
                let mut members = materialize(&snapshot, cancel)?;
                if !prefix.is_empty() {
                    if !prefix.ends_with('/') {
                        return Err(
                            "Virtual ZIP prefix must end with a directory separator.".into()
                        );
                    }
                    archives::validate_name(prefix.trim_end_matches('/'))?;
                }
                if selected.is_empty() {
                    return Err("Choose ZIP members, not the parent row.".into());
                }
                for name in &selected {
                    if !name.starts_with(&prefix) || !members.contains_key(name) {
                        return Err(
                            "Selected virtual path does not bind to the current ZIP directory."
                                .into(),
                        );
                    }
                }
                let parent = scope.resolve(&root)?;
                let mut planned = BTreeMap::new();
                let chosen: Vec<_> = members
                    .keys()
                    .filter(|name| {
                        selected.iter().any(|selected| {
                            *name == selected || name.starts_with(&format!("{selected}/"))
                        })
                    })
                    .cloned()
                    .collect();
                member_rows.extend(chosen.iter().map(|name| {
                    format!(
                        "{} ZIP member: {name}",
                        if move_entries { "Move out" } else { "Copy out" }
                    )
                }));
                for name in &chosen {
                    let relative = name
                        .strip_prefix(&prefix)
                        .ok_or("ZIP prefix binding failed.")?;
                    planned.insert(
                        relative.to_owned(),
                        members
                            .get(name)
                            .cloned()
                            .ok_or("ZIP selection disappeared.")?,
                    );
                    let parts: Vec<_> = relative.split('/').collect();
                    for length in 1..parts.len() {
                        planned.entry(parts[..length].join("/")).or_insert(None);
                    }
                }
                let outputs = planned
                    .into_iter()
                    .map(|(relative, bytes)| Ok((destination(scope, &parent, &relative)?, bytes)))
                    .collect::<FileResult<Vec<_>>>()?;
                if outputs.iter().any(|(path, _)| path == &snapshot.path) {
                    return Err("Cannot extract over the source archive.".into());
                }
                let zip = if move_entries {
                    for name in chosen {
                        members.remove(&name);
                    }
                    Some(encode(&members, cancel)?)
                } else {
                    None
                };
                let target = move_entries.then(|| snapshot.path.clone());
                (
                    if move_entries {
                        "Move ZIP members to folder"
                    } else {
                        "Copy ZIP members to folder"
                    },
                    vec![snapshot],
                    Vec::new(),
                    parent,
                    target,
                    move_entries,
                    zip,
                    outputs,
                    false,
                )
            }
            Request::Insert {
                archive,
                prefix,
                sources,
                move_sources,
            } => {
                let original = Snapshot::read(scope, &archive, LIMIT)?;
                let mut members = materialize(&original, cancel)?;
                if !prefix.is_empty() {
                    if !prefix.ends_with('/') {
                        return Err(
                            "Virtual ZIP prefix must end with a directory separator.".into()
                        );
                    }
                    let name = archives::validate_name(prefix.trim_end_matches('/'))?;
                    if members.get(&name) != Some(&None) {
                        return Err("ZIP insertion directory is missing.".into());
                    }
                }
                let (added, mut snapshots, folders) = gather(scope, &sources, &prefix, cancel)?;
                if snapshots.iter().any(|s| s.path == original.path)
                    || folders
                        .iter()
                        .any(|(path, _)| original.path.starts_with(path))
                {
                    return Err("An archive cannot contain its own source archive/tree.".into());
                }
                let mut folded: BTreeSet<_> =
                    members.keys().map(|name| name.to_lowercase()).collect();
                for (name, data) in added {
                    if !folded.insert(name.to_lowercase()) {
                        return Err(format!("ZIP member already exists; no overwrite: {name}"));
                    }
                    member_rows.push(format!("Add ZIP member: {name}"));
                    members.insert(name, data);
                }
                if members.len() > 10_000
                    || members.values().flatten().map(Vec::len).sum::<usize>() > LIMIT
                {
                    return Err("Rebuilt ZIP exceeds its bounded member/data limits.".into());
                }
                let parent = original
                    .path
                    .parent()
                    .ok_or("ZIP has no parent.")?
                    .to_owned();
                let target = original.path.clone();
                snapshots.insert(0, original);
                (
                    if move_sources {
                        "Move files into ZIP"
                    } else {
                        "Copy files into ZIP"
                    },
                    snapshots,
                    folders,
                    parent,
                    Some(target),
                    true,
                    Some(encode(&members, cancel)?),
                    Vec::new(),
                    move_sources,
                )
            }
        };
    let parent_identity = files::fingerprint(&parent)?;
    if !parent_identity.directory {
        return Err("ZIP destination must be a real folder.".into());
    }
    let stage_parent = if replacement {
        target
            .as_ref()
            .and_then(|t| t.parent())
            .ok_or("ZIP replacement parent missing.")?
    } else {
        &parent
    };
    let stage = stage_parent.join(format!(".tomas-commander-{key}.zip-stage"));
    Ok(Plan {
        id,
        label,
        key,
        snapshots,
        folders,
        parent,
        parent_identity,
        target,
        replacement,
        stage,
        zip,
        outputs,
        member_rows,
        remove_sources,
    })
}

impl Plan {
    pub fn summary(&self) -> String {
        let publication = if self.replacement {
            "Only the verified original archive is replaced."
        } else {
            "No existing archive is replaced."
        };
        let sources = if self.remove_sources {
            "Real sources are removed only after archive publication and verification."
        } else if self.replacement && !self.outputs.is_empty() {
            "Selected ZIP members are removed only after extraction and verification."
        } else {
            "Sources are preserved."
        };
        let stage = if self.zip.is_some() {
            files::display_path(&self.stage)
        } else {
            "None; extraction creates only the listed new targets.".into()
        };
        let mut summary = format!(
            "{}\n{} files / {} folders frozen; {} exact extraction targets.\nNo overwrites or folder merging. {publication} {sources}\nStaging: {stage}\nPartial created paths remain on error/cancel; no blind retry or automatic deletion.\n",
            self.label,
            self.snapshots.len(),
            self.folders.len(),
            self.outputs.len()
        );
        if let Some(target) = &self.target {
            summary.push_str(&format!(
                "Archive target: {}\n",
                files::display_path(target)
            ));
        }
        summary
    }

    pub fn row_count(&self) -> usize {
        self.snapshots.len() + self.folders.len() + self.outputs.len() + self.member_rows.len()
    }

    pub fn row(&self, index: usize) -> String {
        let (label, path) = if index < self.snapshots.len() {
            ("Source", &self.snapshots[index].path)
        } else if index < self.snapshots.len() + self.folders.len() {
            (
                "Source folder",
                &self.folders[index - self.snapshots.len()].0,
            )
        } else if index < self.snapshots.len() + self.folders.len() + self.outputs.len() {
            (
                "New target",
                &self.outputs[index - self.snapshots.len() - self.folders.len()].0,
            )
        } else {
            return self.member_rows
                [index - self.snapshots.len() - self.folders.len() - self.outputs.len()]
            .clone();
        };
        format!("{label}: {}", files::display_path(path))
    }

    pub fn execute(
        &self,
        scope: &Scope,
        journal: &Journal,
        approved: bool,
        cancel: &AtomicBool,
    ) -> Outcome {
        let mut outcome = Outcome {
            id: self.id,
            completed: Vec::new(),
            created: Vec::new(),
            incomplete: self
                .snapshots
                .iter()
                .map(|s| s.path.clone())
                .chain(self.folders.iter().map(|(p, _)| p.clone()))
                .collect(),
            cancelled: false,
            error: None,
        };
        if !approved || cancel.load(Ordering::Acquire) {
            outcome.error = Some("Explicit exact ZIP plan approval is required.".into());
            outcome.cancelled = cancel.load(Ordering::Acquire);
            return outcome;
        }
        let mut affected = vec![self.parent.clone(), self.stage.clone()];
        affected.extend(self.snapshots.iter().map(|s| s.path.clone()));
        affected.extend(self.folders.iter().map(|(p, _)| p.clone()));
        affected.extend(self.outputs.iter().map(|(p, _)| p.clone()));
        affected.extend(self.target.iter().cloned());
        let mut snapshot = format!("operation={}\nid={}\n", self.label, self.id);
        for path in &affected {
            snapshot.push_str(&format!("source={}\n", crate::journal::encode_path(path)));
        }
        for source in &self.snapshots {
            snapshot.push_str(&format!("identity={:?}\n", source.identity));
        }
        for member in &self.member_rows {
            snapshot.push_str(&format!("member={member}\n"));
        }
        let record = match journal.claim_snapshot(&self.key, &snapshot, &affected) {
            Ok(record) => record,
            Err(error) => {
                outcome.error = Some(error);
                return outcome;
            }
        };
        let result = (|| {
            cancelled(cancel)?;
            if scope.resolve(&self.parent)? != self.parent
                || files::fingerprint(&self.parent)? != self.parent_identity
            {
                return Err("ZIP destination folder changed after planning.".into());
            }
            let mut locks = Vec::new();
            for source in &self.snapshots {
                locks.push(source.validate(scope)?);
            }
            for (path, identity) in &self.folders {
                if scope.mutation_source(path)? != *path || files::fingerprint(path)? != *identity {
                    return Err("ZIP source folder changed after planning.".into());
                }
            }
            for (path, _) in &self.outputs {
                if path
                    .try_exists()
                    .map_err(|error| format!("Validate ZIP target: {error}"))?
                {
                    return Err("ZIP extraction target changed; no overwrite.".into());
                }
            }
            let mut outputs: Vec<_> = self.outputs.iter().collect();
            outputs.sort_by_key(|(path, data)| (data.is_some(), path.components().count()));
            for (path, data) in outputs {
                cancelled(cancel)?;
                if let Some(data) = data {
                    let parent =
                        scope.resolve(path.parent().ok_or("Extraction parent missing.")?)?;
                    if parent != path.parent().ok_or("Extraction parent missing.")? {
                        return Err("Extraction parent binding changed.".into());
                    }
                    write_new(path, data, cancel, &mut outcome, None)?;
                    outcome.completed.push(path.clone());
                } else {
                    let parent = path.parent().ok_or("Extraction folder parent missing.")?;
                    if scope.resolve(parent)? != parent {
                        return Err("Extraction folder parent binding changed.".into());
                    }
                    fs::create_dir(path).map_err(|error| {
                        format!("Create exact ZIP folder without merge: {error}")
                    })?;
                    outcome.created.push(path.clone());
                }
            }
            if let Some(bytes) = &self.zip {
                let security_source = self
                    .target
                    .as_deref()
                    .filter(|_| self.replacement)
                    .unwrap_or(&self.parent);
                write_new(
                    &self.stage,
                    bytes,
                    cancel,
                    &mut outcome,
                    Some(security_source),
                )?;
                cancelled(cancel)?;
                let target = self
                    .target
                    .as_ref()
                    .ok_or("ZIP publish target is missing.")?;
                if self.replacement {
                    let original = self
                        .snapshots
                        .first()
                        .ok_or("ZIP original snapshot missing.")?;
                    drop(locks.remove(0));
                    if files::fingerprint(&original.path)? != original.identity {
                        return Err("ZIP changed immediately before replacement.".into());
                    }
                    changes::replace(target, &self.stage)?;
                } else {
                    publish_new(&self.stage, target)?;
                }
                outcome.created.retain(|p| p != &self.stage);
                if !self.replacement {
                    outcome.created.push(target.clone());
                }
                let saved = Snapshot::read(scope, target, LIMIT)?;
                if saved.bytes != *bytes {
                    return Err("Published ZIP verification failed; inspect actual paths, no source deletion.".into());
                }
                archives::inventory_bytes(&saved.bytes, || cancel.load(Ordering::Acquire))?;
                outcome.completed.push(target.clone());
                outcome.incomplete.retain(|p| p != target);
            }
            drop(locks);
            if self.remove_sources {
                for source in self.snapshots.iter().skip(1) {
                    cancelled(cancel)?;
                    drop(source.validate(scope)?);
                    fs::remove_file(&source.path).map_err(|error| {
                        format!("Verified ZIP saved but original file removal failed: {error}")
                    })?;
                    outcome.completed.push(source.path.clone());
                    outcome.incomplete.retain(|p| p != &source.path);
                }
                let mut folders: Vec<_> = self.folders.iter().collect();
                folders.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
                for (path, original) in folders {
                    cancelled(cancel)?;
                    let actual = files::fingerprint(path)?;
                    #[cfg(windows)]
                    if actual.file_id != original.file_id {
                        return Err("Source folder identity changed before move cleanup.".into());
                    }
                    if !actual.directory || actual.created != original.created {
                        return Err("Source folder changed before move cleanup.".into());
                    }
                    fs::remove_dir(path).map_err(|error| {
                        format!(
                            "ZIP saved; source folder not empty or inaccessible, retained: {error}"
                        )
                    })?;
                    outcome.completed.push(path.clone());
                    outcome.incomplete.retain(|p| p != path);
                }
            }
            outcome.incomplete.clear();
            Ok(())
        })();
        if let Err(error) = result {
            outcome.error = Some(error);
        }
        outcome.cancelled = cancel.load(Ordering::Acquire) && outcome.error.is_some();
        Journal::finish(record, outcome)
    }
}

fn write_new(
    path: &Path,
    bytes: &[u8],
    cancel: &AtomicBool,
    outcome: &mut Outcome,
    security_source: Option<&Path>,
) -> FileResult<()> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let mut file = if let Some(source) = security_source {
        changes::secure_stage(source, path)?
    } else {
        options
            .open(path)
            .map_err(|error| format!("Create ZIP effect without overwrite: {error}"))?
    };
    outcome.created.push(path.to_owned());
    for chunk in bytes.chunks(64 * 1024) {
        cancelled(cancel)?;
        file.write_all(chunk)
            .map_err(|error| format!("Write ZIP effect: {error}"))?;
    }
    file.sync_all()
        .map_err(|error| format!("Flush ZIP effect: {error}"))?;
    file.rewind()
        .map_err(|error| format!("Rewind ZIP effect: {error}"))?;
    let mut verified = Vec::new();
    Read::by_ref(&mut file)
        .take(bytes.len() as u64 + 1)
        .read_to_end(&mut verified)
        .map_err(|error| format!("Read back ZIP effect: {error}"))?;
    if verified != bytes {
        return Err("Exact ZIP output byte verification failed.".into());
    }
    Ok(())
}

#[cfg(windows)]
fn publish_new(stage: &Path, target: &Path) -> FileResult<()> {
    use std::os::windows::ffi::OsStrExt;
    let stage: Vec<u16> = stage.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        windows::Win32::Storage::FileSystem::MoveFileExW(
            windows_core::PCWSTR(stage.as_ptr()),
            windows_core::PCWSTR(target.as_ptr()),
            windows::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| format!("Publish ZIP without overwrite: {error}"))
}
#[cfg(not(windows))]
fn publish_new(_stage: &Path, _target: &Path) -> FileResult<()> {
    Err("ZIP publishing currently requires Windows.".into())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn real_zip_create_copy_move_both_directions_and_replay_preserve_exact_bytes() {
        let root =
            std::env::temp_dir().join(format!("tc-zip-effects-{}", changes::identity().unwrap()));
        let source = root.join("Příliš folder");
        fs::create_dir_all(source.join("empty")).unwrap();
        fs::write(source.join("one.txt"), b"exact one").unwrap();
        let out = root.join("out");
        fs::create_dir(&out).unwrap();
        let scope = Scope::fixture(&root).unwrap();
        let journal = Journal::new(root.join("journal"));
        journal.initialize().unwrap();
        let cancel = AtomicBool::new(false);
        let archive = root.join("sample.zip");
        let plan = prepare(
            &scope,
            1,
            Request::Create {
                sources: vec![source.clone()],
                target: archive.clone(),
            },
            &cancel,
        )
        .unwrap();
        assert!(
            plan.execute(&scope, &journal, false, &cancel)
                .error
                .is_some()
        );
        assert!(!archive.exists());
        let result = plan.execute(&scope, &journal, true, &cancel);
        assert!(result.error.is_none(), "{result:?}");
        assert_eq!(
            read_entry(&scope, &archive, "Příliš folder/one.txt", || false).unwrap(),
            b"exact one"
        );
        assert!(
            plan.execute(&scope, &journal, true, &cancel)
                .error
                .unwrap()
                .contains("Claim operation once")
        );
        let plan = prepare(
            &scope,
            2,
            Request::Extract {
                archive: archive.clone(),
                prefix: "".into(),
                selected: vec!["Příliš folder".into()],
                destination: out.clone(),
                move_entries: false,
            },
            &cancel,
        )
        .unwrap();
        let before = fs::read(&archive).unwrap();
        let result = plan.execute(&scope, &journal, true, &cancel);
        assert!(result.error.is_none(), "{result:?}");
        assert_eq!(
            fs::read(out.join("Příliš folder\\one.txt")).unwrap(),
            b"exact one"
        );
        assert!(out.join("Příliš folder\\empty").is_dir());
        assert_eq!(fs::read(&archive).unwrap(), before);
        assert!(
            prepare(
                &scope,
                3,
                Request::Extract {
                    archive: archive.clone(),
                    prefix: "".into(),
                    selected: vec!["Příliš folder".into()],
                    destination: out.clone(),
                    move_entries: false,
                },
                &cancel
            )
            .is_err()
        );

        let extra = root.join("additional.md");
        fs::write(&extra, b"# exact added").unwrap();
        let plan = prepare(
            &scope,
            4,
            Request::Insert {
                archive: archive.clone(),
                prefix: "".into(),
                sources: vec![extra.clone()],
                move_sources: true,
            },
            &cancel,
        )
        .unwrap();
        let result = plan.execute(&scope, &journal, true, &cancel);
        assert!(result.error.is_none(), "{result:?}");
        assert!(!extra.exists());
        assert_eq!(
            read_entry(&scope, &archive, "additional.md", || false).unwrap(),
            b"# exact added"
        );
        let plan = prepare(
            &scope,
            5,
            Request::Extract {
                archive: archive.clone(),
                prefix: "".into(),
                selected: vec!["additional.md".into()],
                destination: out.clone(),
                move_entries: true,
            },
            &cancel,
        )
        .unwrap();
        let result = plan.execute(&scope, &journal, true, &cancel);
        assert!(result.error.is_none(), "{result:?}");
        assert_eq!(
            fs::read(out.join("additional.md")).unwrap(),
            b"# exact added"
        );
        assert!(read_entry(&scope, &archive, "additional.md", || false).is_err());
        assert_eq!(
            read_entry(&scope, &archive, "Příliš folder/one.txt", || false).unwrap(),
            b"exact one"
        );
        let copy_only = root.join("copy-only.txt");
        fs::write(&copy_only, b"copy keeps source").unwrap();
        let plan = prepare(
            &scope,
            6,
            Request::Insert {
                archive: archive.clone(),
                prefix: "".into(),
                sources: vec![copy_only.clone()],
                move_sources: false,
            },
            &cancel,
        )
        .unwrap();
        let result = plan.execute(&scope, &journal, true, &cancel);
        assert!(result.error.is_none(), "{result:?}");
        assert_eq!(fs::read(&copy_only).unwrap(), b"copy keeps source");
        assert_eq!(
            read_entry(&scope, &archive, "copy-only.txt", || false).unwrap(),
            b"copy keeps source"
        );
        let moved_folder = root.join("Moved folder");
        fs::create_dir(&moved_folder).unwrap();
        fs::create_dir(moved_folder.join("empty")).unwrap();
        fs::write(moved_folder.join("child.txt"), b"moved folder bytes").unwrap();
        assert!(
            prepare(
                &scope,
                7,
                Request::Insert {
                    archive: archive.clone(),
                    prefix: "".into(),
                    sources: vec![moved_folder.clone(), moved_folder.join("child.txt")],
                    move_sources: true
                },
                &cancel
            )
            .is_err()
        );
        let plan = prepare(
            &scope,
            7,
            Request::Insert {
                archive: archive.clone(),
                prefix: "".into(),
                sources: vec![moved_folder.clone()],
                move_sources: true,
            },
            &cancel,
        )
        .unwrap();
        let result = plan.execute(&scope, &journal, true, &cancel);
        assert!(result.error.is_none(), "{result:?}");
        assert!(!moved_folder.exists());
        assert_eq!(
            read_entry(&scope, &archive, "Moved folder/child.txt", || false).unwrap(),
            b"moved folder bytes"
        );
        let plan = prepare(
            &scope,
            8,
            Request::Extract {
                archive: archive.clone(),
                prefix: "".into(),
                selected: vec!["Moved folder".into()],
                destination: out.clone(),
                move_entries: true,
            },
            &cancel,
        )
        .unwrap();
        let result = plan.execute(&scope, &journal, true, &cancel);
        assert!(result.error.is_none(), "{result:?}");
        assert_eq!(
            fs::read(out.join("Moved folder\\child.txt")).unwrap(),
            b"moved folder bytes"
        );
        assert!(out.join("Moved folder\\empty").is_dir());
        assert!(read_entry(&scope, &archive, "Moved folder/child.txt", || false).is_err());
        assert!(journal.pending().unwrap().is_empty());

        fs::remove_file(copy_only).unwrap();
        fs::remove_file(out.join("Moved folder\\child.txt")).unwrap();
        fs::remove_dir(out.join("Moved folder\\empty")).unwrap();
        fs::remove_dir(out.join("Moved folder")).unwrap();
        fs::remove_file(out.join("additional.md")).unwrap();
        fs::remove_file(out.join("Příliš folder\\one.txt")).unwrap();
        fs::remove_dir(out.join("Příliš folder\\empty")).unwrap();
        fs::remove_dir(out.join("Příliš folder")).unwrap();
        fs::remove_dir(&out).unwrap();
        fs::remove_file(source.join("one.txt")).unwrap();
        fs::remove_dir(source.join("empty")).unwrap();
        fs::remove_dir(&source).unwrap();
        fs::remove_file(&archive).unwrap();
        for entry in fs::read_dir(root.join("journal")).unwrap() {
            fs::remove_file(entry.unwrap().path()).unwrap();
        }
        fs::remove_dir(root.join("journal")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn crc_cancel_external_source_and_destination_changes_refuse_without_effects() {
        let root =
            std::env::temp_dir().join(format!("tc-zip-guards-{}", changes::identity().unwrap()));
        fs::create_dir(&root).unwrap();
        let source = root.join("one.txt");
        fs::write(&source, b"original").unwrap();
        let scope = Scope::fixture(&root).unwrap();
        let journal = Journal::new(root.join("journal"));
        journal.initialize().unwrap();
        let cancel = AtomicBool::new(false);
        let archive = root.join("sample.zip");
        let plan = prepare(
            &scope,
            1,
            Request::Create {
                sources: vec![source.clone()],
                target: archive.clone(),
            },
            &cancel,
        )
        .unwrap();
        fs::write(&source, b"modified").unwrap();
        assert!(
            plan.execute(&scope, &journal, true, &cancel)
                .error
                .is_some()
        );
        assert!(!archive.exists());
        let plan = prepare(
            &scope,
            2,
            Request::Create {
                sources: vec![source.clone()],
                target: archive.clone(),
            },
            &cancel,
        )
        .unwrap();
        cancel.store(true, Ordering::Release);
        assert!(plan.execute(&scope, &journal, true, &cancel).cancelled);
        assert!(!archive.exists());
        cancel.store(false, Ordering::Release);
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "bad.txt",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(b"bad crc").unwrap();
        let mut bytes = writer.finish().unwrap().into_inner();
        let name_len = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
        let extra_len = u16::from_le_bytes([bytes[28], bytes[29]]) as usize;
        bytes[30 + name_len + extra_len] ^= 1;
        fs::write(&archive, &bytes).unwrap();
        assert!(
            read_entry(&scope, &archive, "bad.txt", || false)
                .unwrap_err()
                .contains("CRC")
        );
        assert!(
            prepare(
                &scope,
                3,
                Request::Extract {
                    archive: archive.clone(),
                    prefix: "".into(),
                    selected: vec!["bad.txt".into()],
                    destination: root.clone(),
                    move_entries: false
                },
                &cancel
            )
            .is_err()
        );
        assert_eq!(fs::read(&source).unwrap(), b"modified");
        fs::remove_file(&source).unwrap();
        fs::remove_file(&archive).unwrap();
        for entry in fs::read_dir(root.join("journal")).unwrap() {
            fs::remove_file(entry.unwrap().path()).unwrap();
        }
        fs::remove_dir(root.join("journal")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
