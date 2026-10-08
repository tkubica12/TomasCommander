use crate::files::{self, Entry, FileResult, Scope};
use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub const MAX_EXAMINED: usize = 100_000;
pub const MAX_RESULTS: usize = 10_000;
const MAX_ERRORS: usize = 100;

#[derive(Clone, Debug)]
pub struct Match {
    pub entry: Entry,
    pub relative: PathBuf,
    created: SystemTime,
    #[cfg(windows)]
    file_id: (u32, u64),
}

#[derive(Debug, Default)]
pub struct Results {
    pub matches: Vec<Match>,
    pub errors: Vec<String>,
    pub examined: usize,
    pub skipped_links: usize,
    pub cancelled: bool,
    pub truncated: bool,
}

pub fn discover(
    scope: &Scope,
    root: &Path,
    query: &str,
    cancel: impl Fn() -> bool,
) -> FileResult<Results> {
    if query.trim().is_empty() {
        return Err("Enter a filename query before searching.".into());
    }
    let root = scope.resolve(root)?;
    let query = query.to_lowercase();
    let mut result = Results::default();
    let mut stack = vec![root.clone()];
    while let Some(folder) = stack.pop() {
        if cancel() {
            result.cancelled = true;
            break;
        }
        if result.errors.len() >= MAX_ERRORS {
            result.truncated = true;
            break;
        }
        let metadata = match fs::symlink_metadata(&folder) {
            Ok(metadata) => metadata,
            Err(error) => {
                result.errors.push(format!(
                    "Inspect search folder {}: {error}",
                    files::display_path(&folder)
                ));
                continue;
            }
        };
        if files::reparse(&metadata) {
            result.skipped_links += 1;
            continue;
        }
        match scope.resolve(&folder) {
            Ok(resolved) if resolved == folder => (),
            Ok(_) => {
                result.errors.push(format!(
                    "Search folder changed: {}",
                    files::display_path(&folder)
                ));
                continue;
            }
            Err(error) => {
                result.errors.push(error);
                continue;
            }
        }
        let items = match fs::read_dir(&folder) {
            Ok(items) => items,
            Err(error) => {
                result.errors.push(format!(
                    "Read search folder {}: {error}",
                    files::display_path(&folder)
                ));
                if result.errors.len() >= MAX_ERRORS {
                    result.truncated = true;
                    break;
                }
                continue;
            }
        };
        let mut children = Vec::new();
        for item in items {
            if cancel() {
                result.cancelled = true;
                break;
            }
            if result.examined >= MAX_EXAMINED
                || result.matches.len() >= MAX_RESULTS
                || result.errors.len() >= MAX_ERRORS
            {
                result.truncated = true;
                break;
            }
            result.examined += 1;
            let inspected = (|| -> FileResult<_> {
                let item = item.map_err(|e| {
                    format!("Read search item in {}: {e}", files::display_path(&folder))
                })?;
                let path = item.path();
                let metadata = fs::symlink_metadata(&path).map_err(|e| {
                    format!("Inspect search item {}: {e}", files::display_path(&path))
                })?;
                if files::reparse(&metadata) {
                    return Ok(None);
                }
                let name = item.file_name().to_string_lossy().into_owned();
                let entry = Entry {
                    path: path.clone(),
                    folded: name.to_lowercase(),
                    name,
                    directory: metadata.is_dir(),
                    link: false,
                    size: metadata.len(),
                    modified: Some(metadata.modified().map_err(|e| {
                        format!(
                            "Read search modification time {}: {e}",
                            files::display_path(&path)
                        )
                    })?),
                    aggregate: None,
                };
                let created = metadata.created().map_err(|e| {
                    format!(
                        "Read search creation time {}: {e}",
                        files::display_path(&path)
                    )
                })?;
                Ok(Some((entry, created)))
            })();
            match inspected {
                Ok(Some((entry, created))) => {
                    if entry.directory {
                        children.push(entry.path.clone());
                    }
                    if entry.folded.contains(&query) {
                        let relative = entry
                            .path
                            .strip_prefix(&root)
                            .map_err(|e| format!("Search scope changed: {e}"))?
                            .to_owned();
                        result.matches.push(Match {
                            #[cfg(windows)]
                            file_id: crate::platform::path_identity(&entry.path)?,
                            entry,
                            relative,
                            created,
                        });
                    }
                }
                Ok(None) => result.skipped_links += 1,
                Err(error) => result.errors.push(error),
            }
        }
        if result.cancelled || result.truncated {
            break;
        }
        children.sort();
        stack.extend(children.into_iter().rev());
    }
    result.matches.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(result)
}

pub fn locate(scope: &Scope, result: &Match) -> FileResult<PathBuf> {
    let path = scope.resolve(&result.entry.path)?;
    let metadata = fs::symlink_metadata(&result.entry.path)
        .map_err(|e| format!("Inspect search result: {e}"))?;
    #[cfg(windows)]
    if crate::platform::path_identity(&path)? != result.file_id {
        return Err("Search result identity changed. Run the search again.".into());
    }
    if path != result.entry.path
        || files::reparse(&metadata)
        || metadata.is_dir() != result.entry.directory
        || metadata.len() != result.entry.size
        || Some(
            metadata
                .modified()
                .map_err(|e| format!("Read result time: {e}"))?,
        ) != result.entry.modified
        || metadata
            .created()
            .map_err(|e| format!("Read result creation time: {e}"))?
            != result.created
    {
        return Err("Search result changed. Run the search again before activating it.".into());
    }
    Ok(path)
}
