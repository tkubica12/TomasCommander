use crate::{
    archive_changes, archives,
    changes::{self, Snapshot},
    files::{self, FileResult, Scope},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime},
};

const PREFIX: &str = "tomas-commander-export-";
const MANIFEST: &str = "export.json";
const LIMIT: u64 = 32 * 1_048_576;
const LEASE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u8,
    entries: BTreeMap<String, bool>,
}

pub struct Export {
    pub paths: Vec<PathBuf>,
    root: PathBuf,
    manifest: Manifest,
    retained: bool,
}

impl Export {
    pub fn cleanup(mut self) -> FileResult<()> {
        self.retained = true;
        remove_export(&self.root, &self.manifest)
    }

    pub fn retain(mut self) {
        self.retained = true;
    }
}

impl Drop for Export {
    fn drop(&mut self) {
        if !self.retained
            && let Err(error) = remove_export(&self.root, &self.manifest)
        {
            eprintln!("{error}");
        }
    }
}

fn cancelled(cancel: &AtomicBool) -> FileResult<()> {
    if cancel.load(Ordering::Acquire) {
        Err("Temporary ZIP extraction cancelled.".into())
    } else {
        Ok(())
    }
}

fn parent(scope: &Scope) -> PathBuf {
    scope
        .root()
        .map(Path::to_owned)
        .unwrap_or_else(std::env::temp_dir)
}

pub fn prepare(
    scope: &Scope,
    archive: &Path,
    selected: &[String],
    cancel: &AtomicBool,
) -> FileResult<Export> {
    cancelled(cancel)?;
    let snapshot = Snapshot::read(scope, archive, LIMIT as usize)?;
    let inventory = archives::inventory_bytes(&snapshot.bytes, || cancel.load(Ordering::Acquire))?;
    let selected: BTreeSet<_> = selected.iter().cloned().collect();
    if selected.is_empty() {
        return Err("Choose ZIP members, not the parent row.".into());
    }
    for name in &selected {
        archives::validate_name(name)?;
        if !inventory.entries.iter().any(|entry| &entry.path == name) {
            return Err(format!("ZIP member disappeared: {name}"));
        }
    }
    let roots: Vec<_> = selected
        .iter()
        .filter(|name| {
            !selected
                .iter()
                .any(|parent| *name != parent && name.starts_with(&format!("{parent}/")))
        })
        .collect();
    let mut names = BTreeSet::new();
    let mut mappings = BTreeMap::new();
    for root in roots {
        let basename = root
            .rsplit('/')
            .next()
            .ok_or("ZIP member has no filename.")?;
        let mut name = basename.to_owned();
        let mut suffix = 2;
        while !names.insert(name.to_lowercase()) {
            let stem = Path::new(basename)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(basename);
            let ext = Path::new(basename).extension().and_then(|s| s.to_str());
            name = format!(
                "{stem} ({suffix}){}",
                ext.map_or(String::new(), |ext| format!(".{ext}"))
            );
            archives::validate_name(&name)?;
            suffix += 1;
        }
        mappings.insert(root.clone(), name);
    }
    let mut total = 0;
    let mut outputs = BTreeMap::new();
    for entry in &inventory.entries {
        if let Some((root, basename)) = mappings
            .iter()
            .find(|(root, _)| entry.path == **root || entry.path.starts_with(&format!("{root}/")))
        {
            total += entry.size;
            if total > LIMIT {
                return Err("Temporary ZIP extraction exceeds 32 MiB expanded data.".into());
            }
            let suffix = entry
                .path
                .strip_prefix(root.as_str())
                .ok_or("ZIP root binding failed.")?;
            outputs.insert(format!("{basename}{suffix}"), entry);
        }
    }
    let root = parent(scope).join(format!("{PREFIX}{}", changes::identity()?));
    fs::create_dir(&root).map_err(|e| format!("Create temporary ZIP folder: {e}"))?;
    let mut manifest = Manifest {
        version: 1,
        entries: BTreeMap::from([("items".into(), true)]),
    };
    for (name, entry) in &outputs {
        manifest
            .entries
            .insert(format!("items/{name}"), entry.directory);
    }
    let export = Export {
        paths: mappings
            .values()
            .map(|name| root.join("items").join(name))
            .collect(),
        root: root.clone(),
        manifest,
        retained: false,
    };
    let result = (|| {
        let _lock = snapshot.validate(scope)?;
        let marker = serde_json::to_vec(&export.manifest)
            .map_err(|e| format!("Encode ZIP export manifest: {e}"))?;
        if marker.len() > 2 * 1_048_576 {
            return Err("ZIP export manifest exceeds its cleanup limit.".into());
        }
        let mut file = changes::secure_stage(&snapshot.path, &root.join(MANIFEST))?;
        file.write_all(&marker)
            .and_then(|()| file.sync_all())
            .map_err(|e| format!("Write ZIP export manifest: {e}"))?;
        drop(file);
        fs::create_dir(root.join("items")).map_err(|e| format!("Create ZIP export items: {e}"))?;
        let mut ordered: Vec<_> = outputs.iter().collect();
        ordered.sort_by_key(|(name, entry)| (name.split('/').count(), !entry.directory));
        for (relative, entry) in ordered {
            cancelled(cancel)?;
            let path = relative
                .split('/')
                .fold(root.join("items"), |path, part| path.join(part));
            ensure_plain_parent(&root, &path)?;
            if entry.directory {
                fs::create_dir(&path).map_err(|e| format!("Create exported ZIP directory: {e}"))?;
            } else {
                let bytes = archive_changes::entry_bytes(&snapshot.bytes, &entry.path, || {
                    cancel.load(Ordering::Acquire)
                })?;
                let mut file = changes::secure_stage(&snapshot.path, &path)?;
                file.write_all(&bytes)
                    .and_then(|()| file.sync_all())
                    .map_err(|e| format!("Write exported ZIP member: {e}"))?;
                drop(file);
                if fs::read(&path).map_err(|e| format!("Verify exported ZIP member: {e}"))? != bytes
                {
                    return Err("Temporary ZIP member read-back differs.".into());
                }
            }
        }
        cancelled(cancel)?;
        if files::fingerprint(&snapshot.path)? != snapshot.identity {
            return Err("ZIP changed during extraction; nothing opened or handed off.".into());
        }
        Ok(())
    })();
    if let Err(error) = result {
        return match export.cleanup() {
            Ok(()) => Err(error),
            Err(cleanup) => Err(format!("{error}\n{cleanup}")),
        };
    }
    Ok(export)
}

fn ensure_plain_parent(root: &Path, path: &Path) -> FileResult<()> {
    let mut parent = path.parent();
    while let Some(current) = parent {
        let metadata =
            fs::symlink_metadata(current).map_err(|e| format!("Inspect ZIP export folder: {e}"))?;
        if files::reparse(&metadata) || !metadata.is_dir() {
            return Err("ZIP export folder changed into a link or non-directory.".into());
        }
        if current == root {
            return Ok(());
        }
        parent = current.parent();
    }
    Err("ZIP export path escaped its private folder.".into())
}

fn remove_export(root: &Path, manifest: &Manifest) -> FileResult<()> {
    let mut entries: Vec<_> = manifest.entries.iter().collect();
    if manifest.version != 1 || entries.len() > 20_001 {
        return Err("Unsupported ZIP export cleanup manifest.".into());
    }
    for (relative, _) in &entries {
        archives::validate_name(relative)?;
        if relative.as_str() != "items" && !relative.starts_with("items/") {
            return Err("ZIP export cleanup path is outside the item directory.".into());
        }
    }
    entries.sort_by_key(|(name, _)| std::cmp::Reverse(name.split('/').count()));
    for (relative, directory) in entries {
        let path = relative
            .split('/')
            .fold(root.to_owned(), |path, part| path.join(part));
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("Inspect temporary ZIP cleanup target: {e}")),
        };
        ensure_plain_parent(root, &path)?;
        if files::reparse(&metadata) || metadata.is_dir() != *directory {
            return Err("Temporary ZIP item changed type; cleanup refused.".into());
        }
        let result = if *directory {
            fs::remove_dir(&path)
        } else {
            fs::remove_file(&path)
        };
        result.map_err(|e| {
            format!(
                "Clean temporary ZIP item {}: {e}",
                files::display_path(&path)
            )
        })?;
    }
    let marker = root.join(MANIFEST);
    match fs::symlink_metadata(&marker) {
        Ok(metadata) => {
            if files::reparse(&metadata) || !metadata.is_file() {
                return Err("ZIP export manifest changed type; cleanup refused.".into());
            }
            fs::remove_file(&marker).map_err(|e| format!("Remove ZIP export manifest: {e}"))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("Inspect ZIP export manifest: {e}")),
    }
    fs::remove_dir(root).map_err(|e| {
        format!(
            "Remove temporary ZIP folder {}: {e}",
            files::display_path(root)
        )
    })
}

// Do not remove successful handoffs on drop or exit: apps may read files asynchronously.
pub fn cleanup_expired(scope: &Scope, now: SystemTime) -> FileResult<()> {
    for entry in fs::read_dir(parent(scope)).map_err(|e| format!("Scan ZIP export cache: {e}"))? {
        let entry = entry.map_err(|e| format!("Read ZIP export cache: {e}"))?;
        if !entry.file_name().to_string_lossy().starts_with(PREFIX) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|e| format!("Inspect ZIP export cache: {e}"))?;
        if files::reparse(&metadata) || !metadata.is_dir() {
            continue;
        }
        let marker = entry.path().join(MANIFEST);
        let metadata = match fs::symlink_metadata(&marker) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("Inspect ZIP export manifest: {e}")),
        };
        if files::reparse(&metadata) || !metadata.is_file() || metadata.len() > 2 * 1_048_576 {
            return Err("Unsafe ZIP export manifest; cache cleanup refused.".into());
        }
        if now
            .duration_since(
                metadata
                    .modified()
                    .map_err(|e| format!("Read ZIP export age: {e}"))?,
            )
            .unwrap_or_default()
            < LEASE
        {
            continue;
        }
        let mut bytes = Vec::new();
        fs::File::open(&marker)
            .and_then(|file| file.take(2 * 1_048_576 + 1).read_to_end(&mut bytes))
            .map_err(|e| format!("Read ZIP export manifest: {e}"))?;
        if bytes.len() > 2 * 1_048_576 {
            return Err("ZIP export manifest grew beyond its limit.".into());
        }
        let manifest = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Decode ZIP export manifest: {e}"))?;
        remove_export(&entry.path(), &manifest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn fixture(names: &[(&str, &[u8])]) -> (PathBuf, Scope, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("tc-export-test-{}", changes::identity().unwrap()));
        fs::create_dir(&root).unwrap();
        let archive = root.join("sample.zip");
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in names {
            if name.ends_with('/') {
                writer.add_directory(*name, options).unwrap();
            } else {
                writer.start_file(*name, options).unwrap();
                writer.write_all(bytes).unwrap();
            }
        }
        fs::write(&archive, writer.finish().unwrap().into_inner()).unwrap();
        let scope = Scope::fixture(&root).unwrap();
        (root, scope, archive)
    }

    fn finish(root: PathBuf, archive: PathBuf) {
        fs::remove_file(archive).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn exported_multi_selection_is_real_siblings_with_exact_bytes_and_unchanged_zip() {
        let (root, scope, archive) =
            fixture(&[("a/příloha.txt", b"one"), ("b/příloha.txt", b"two")]);
        let before = fs::read(&archive).unwrap();
        let exported = prepare(
            &scope,
            &archive,
            &["a/příloha.txt".into(), "b/příloha.txt".into()],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(exported.paths.len(), 2);
        assert_eq!(exported.paths[0].parent(), exported.paths[1].parent());
        assert_eq!(fs::read(&exported.paths[0]).unwrap(), b"one");
        assert_eq!(fs::read(&exported.paths[1]).unwrap(), b"two");
        assert_ne!(exported.paths[0].file_name(), exported.paths[1].file_name());
        assert_eq!(fs::read(&archive).unwrap(), before);
        let paths = exported.paths.clone();
        exported.cleanup().unwrap();
        assert!(paths.iter().all(|path| !path.exists()));
        finish(root, archive);
    }

    #[test]
    fn folders_overlap_deduplicates_and_retained_handoff_expires_only_after_lease() {
        let (root, scope, archive) =
            fixture(&[("folder/empty/", b""), ("folder/file.txt", b"exact")]);
        let exported = prepare(
            &scope,
            &archive,
            &["folder".into(), "folder/file.txt".into()],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(exported.paths.len(), 1);
        let path = exported.paths[0].clone();
        assert_eq!(fs::read(path.join("file.txt")).unwrap(), b"exact");
        assert!(path.join("empty").is_dir());
        exported.retain();
        cleanup_expired(&scope, SystemTime::now()).unwrap();
        assert!(path.exists());
        cleanup_expired(&scope, SystemTime::now() + LEASE + Duration::from_secs(60)).unwrap();
        assert!(!path.exists());
        finish(root, archive);
    }

    #[test]
    fn invalid_cancelled_or_crc_failed_exports_never_hand_out_partial_files() {
        let (root, scope, archive) = fixture(&[("file.txt", b"unique fixture bytes")]);
        for selection in [vec![], vec!["../escape".into()], vec!["missing.txt".into()]] {
            assert!(prepare(&scope, &archive, &selection, &AtomicBool::new(false)).is_err());
        }
        assert!(
            prepare(
                &scope,
                &archive,
                &["file.txt".into()],
                &AtomicBool::new(true)
            )
            .is_err()
        );
        let mut bytes = fs::read(&archive).unwrap();
        let offset = bytes
            .windows(b"unique fixture bytes".len())
            .position(|part| part == b"unique fixture bytes")
            .unwrap();
        bytes[offset] ^= 1;
        fs::write(&archive, bytes).unwrap();
        assert!(
            prepare(
                &scope,
                &archive,
                &["file.txt".into()],
                &AtomicBool::new(false)
            )
            .err()
            .unwrap()
            .contains("CRC")
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        finish(root, archive);
    }

    #[test]
    fn cleanup_refuses_manifest_escape_and_preserves_untracked_external_edits() {
        let (root, scope, archive) = fixture(&[("file.txt", b"exact")]);
        let exported = prepare(
            &scope,
            &archive,
            &["file.txt".into()],
            &AtomicBool::new(false),
        )
        .unwrap();
        let escape = Manifest {
            version: 1,
            entries: BTreeMap::from([("../sample.zip".into(), false)]),
        };
        assert!(remove_export(&exported.root, &escape).is_err());
        let extra = exported.root.join("items").join("created-by-other-app.txt");
        fs::write(&extra, b"preserve").unwrap();
        assert!(remove_export(&exported.root, &exported.manifest).is_err());
        assert_eq!(fs::read(&extra).unwrap(), b"preserve");
        fs::remove_file(extra).unwrap();
        exported.cleanup().unwrap();
        finish(root, archive);
    }
}
