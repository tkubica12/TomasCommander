use crate::files::{self, Entry, FileResult, Scope};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub const MAX_EXAMINED: usize = 100_000;
pub const MAX_RESULTS: usize = 10_000;
const MAX_ERRORS: usize = 100;

pub const INTENT_INSTRUCTIONS: &str = "Interpret only the user's Czech/English file-search prompt. Return one JSON object, no markdown, tools, commands or paths. Schema: {\"status\":\"ready|ambiguous|unsupported\",\"terms\":[[\"alternative\"]],\"folders\":[[\"alternative\"]],\"file_types\":[\"image|document|audio|video\"]}. All groups are required (AND); alternatives within a group are OR. Terms match filename or ancestor folder names; folders match ancestor names only. Supply Czech/English synonyms when useful (holiday/dovolena/dovolene, Serbia/Srbsko). Empty file_types means any regular file. Use image for photos. Search names only, never infer image/content understanding. Terms must be plain words/phrases, no paths, regex, wildcard or instructions. At most 6 total groups, 6 alternatives per group, 48 UTF-8 bytes per alternative. Do not omit requested concepts to broaden results. For an unclear request return ambiguous with empty arrays; for mutations, commands, content analysis or unsupported filters return unsupported with empty arrays. Never invent filenames. No extra fields.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    Ready,
    Ambiguous,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileType {
    Image,
    Document,
    Audio,
    Video,
}

impl FileType {
    fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Image => &[
                "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "heic", "avif",
            ],
            Self::Document => &[
                "pdf", "txt", "md", "doc", "docx", "odt", "rtf", "xls", "xlsx", "csv", "ppt",
                "pptx",
            ],
            Self::Audio => &["mp3", "wav", "flac", "m4a", "ogg", "aac"],
            Self::Video => &["mp4", "mkv", "mov", "avi", "webm", "m4v"],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub status: IntentStatus,
    pub terms: Vec<Vec<String>>,
    pub folders: Vec<Vec<String>>,
    pub file_types: Vec<FileType>,
}

impl Intent {
    pub fn parse(text: &str) -> FileResult<Self> {
        if text.len() > 4096 {
            return Err(
                "Search interpretation exceeds its byte ceiling. No local scan started.".into(),
            );
        }
        let intent: Self = serde_json::from_str(text)
            .map_err(|_| "Invalid search interpretation JSON. No scan or automatic retry.")?;
        intent.validate()?;
        Ok(intent)
    }

    pub fn validate(&self) -> FileResult<()> {
        let groups = self.terms.len() + self.folders.len();
        if groups > 6 || self.file_types.len() > 4 {
            return Err("Search interpretation exceeds its group/type limits.".into());
        }
        if self.status != IntentStatus::Ready {
            if groups != 0 || !self.file_types.is_empty() {
                return Err(
                    "Uncertain search interpretation must not contain executable filters.".into(),
                );
            }
            return Ok(());
        }
        if groups == 0 && self.file_types.is_empty() {
            return Err(
                "Empty search interpretation cannot authorize an unrestricted scan.".into(),
            );
        }
        for group in self.terms.iter().chain(&self.folders) {
            if group.is_empty() || group.len() > 6 {
                return Err("Search alternative group is empty or too large.".into());
            }
            for term in group {
                if term.trim() != term
                    || term.is_empty()
                    || term.len() > 48
                    || !term
                        .chars()
                        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-'))
                    || !term.chars().any(char::is_alphanumeric)
                {
                    return Err("Search terms must be bounded plain words, not paths, commands or patterns.".into());
                }
            }
        }
        Ok(())
    }
}

pub fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '\u{0300}'..='\u{036f}'))
        .map(|c| match c {
            'á' | 'ä' => 'a',
            'č' => 'c',
            'ď' => 'd',
            'é' | 'ě' => 'e',
            'í' => 'i',
            'ň' => 'n',
            'ó' | 'ö' => 'o',
            'ř' => 'r',
            'š' => 's',
            'ť' => 't',
            'ú' | 'ů' | 'ü' => 'u',
            'ý' => 'y',
            'ž' => 'z',
            other => other,
        })
        .collect()
}

fn term_score(text: &str, term: &str) -> Option<u16> {
    let text = fold(text);
    let term = fold(term);
    if text == term {
        return Some(30);
    }
    let words: Vec<_> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    let query: Vec<_> = term
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    words
        .windows(query.len())
        .filter_map(|window| {
            if window == query {
                Some(20)
            } else if window
                .iter()
                .zip(&query)
                .all(|(word, part)| part.chars().count() >= 3 && word.starts_with(part))
            {
                Some(10)
            } else {
                None
            }
        })
        .max()
}

fn rank(entry: &Entry, relative: &Path, intent: &Intent) -> Option<(u16, String)> {
    if entry.directory {
        return None;
    }
    let extension = entry
        .path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !intent.file_types.is_empty()
        && !intent
            .file_types
            .iter()
            .any(|kind| kind.extensions().contains(&extension.as_str()))
    {
        return None;
    }
    let folders: Vec<_> = relative
        .parent()?
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let mut score = 0;
    let mut reasons = Vec::new();
    for (groups, folders_only) in [(&intent.terms, false), (&intent.folders, true)] {
        for group in groups {
            let best = group
                .iter()
                .filter_map(|term| {
                    let name = (!folders_only)
                        .then(|| term_score(&entry.name, term))
                        .flatten();
                    let folder = folders
                        .iter()
                        .filter_map(|name| term_score(name, term))
                        .max();
                    name.map(|value| (value + 10, term, "name"))
                        .into_iter()
                        .chain(folder.map(|value| (value, term, "folder")))
                        .max_by_key(|(value, _, _)| *value)
                })
                .max_by_key(|(value, _, _)| *value)?;
            score += best.0;
            reasons.push(format!("{}: {}", best.2, best.1));
        }
    }
    if !intent.file_types.is_empty() {
        reasons.push(format!("type: .{extension}"));
    }
    Some((score, reasons.join(" / ")))
}

#[derive(Clone, Debug)]
pub struct Match {
    pub entry: Entry,
    pub relative: PathBuf,
    pub score: u16,
    pub explanation: String,
    root: PathBuf,
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
    let query = fold(query);
    if query.trim().is_empty() {
        return Err("Enter a filename query before searching.".into());
    }
    walk(
        scope,
        root,
        |entry, _| {
            fold(&entry.name)
                .contains(&query)
                .then(|| (0, "filename contains query".into()))
        },
        false,
        cancel,
    )
}

pub fn discover_intent(
    scope: &Scope,
    root: &Path,
    intent: &Intent,
    cancel: impl Fn() -> bool,
) -> FileResult<Results> {
    intent.validate()?;
    match intent.status {
        IntentStatus::Ambiguous => return Err("Search is ambiguous. Specify filename concepts, folder terms or a file type; no scan started.".into()),
        IntentStatus::Unsupported => return Err("This request needs an unsupported filter or action. Search reads names only; no scan started.".into()),
        IntentStatus::Ready => (),
    }
    let resolved = scope.resolve(root)?;
    if resolved.parent().is_none() {
        return Err("AI search cannot scan a filesystem root. Choose a specific folder.".into());
    }
    walk(
        scope,
        root,
        |entry, relative| rank(entry, relative, intent),
        true,
        cancel,
    )
}

pub fn folder_matches(name: &str, alternatives: &[String]) -> bool {
    alternatives.iter().any(|term| fold(name) == fold(term))
}

pub fn discover_folders(
    scope: &Scope,
    root: &Path,
    alternatives: &[String],
    cancel: impl Fn() -> bool,
) -> FileResult<Results> {
    Intent {
        status: IntentStatus::Ready,
        terms: vec![alternatives.to_vec()],
        folders: vec![],
        file_types: vec![],
    }
    .validate()?;
    walk(
        scope,
        root,
        |entry, _| {
            (entry.directory && folder_matches(&entry.name, alternatives))
                .then(|| (0, "exact folder-name alternative".into()))
        },
        false,
        cancel,
    )
}

fn walk(
    scope: &Scope,
    root: &Path,
    matcher: impl Fn(&Entry, &Path) -> Option<(u16, String)>,
    ranked: bool,
    cancel: impl Fn() -> bool,
) -> FileResult<Results> {
    let root_metadata =
        fs::symlink_metadata(root).map_err(|e| format!("Inspect search root: {e}"))?;
    if files::reparse(&root_metadata) || !root_metadata.is_dir() {
        return Err("Search root must be a real folder, not a link/reparse point.".into());
    }
    let root = scope.resolve(root)?;
    let search_scope = Scope::fixture(&root)?;
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
        match search_scope.resolve(&folder) {
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
                    let relative = entry
                        .path
                        .strip_prefix(&root)
                        .map_err(|e| format!("Search scope changed: {e}"))?
                        .to_owned();
                    if let Some((score, explanation)) = matcher(&entry, &relative) {
                        result.matches.push(Match {
                            #[cfg(windows)]
                            file_id: crate::platform::path_identity(&entry.path)?,
                            entry,
                            relative,
                            score,
                            explanation,
                            root: root.clone(),
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
    result.matches.sort_by(|a, b| {
        if ranked {
            b.score
                .cmp(&a.score)
                .then_with(|| a.relative.cmp(&b.relative))
        } else {
            a.relative.cmp(&b.relative)
        }
    });
    Ok(result)
}

pub fn locate(scope: &Scope, result: &Match) -> FileResult<PathBuf> {
    let path = scope.resolve(&result.entry.path)?;
    let search_scope = Scope::fixture(&result.root)?;
    if search_scope.resolve(&path)? != path {
        return Err("Search result escaped its original root.".into());
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_rejects_untrusted_paths_tools_patterns_and_unbounded_filters() {
        for text in [
            r#"{"status":"ready","terms":[["..\\outside"]],"folders":[],"file_types":[]}"#,
            r#"{"status":"ready","terms":[[".*"]],"folders":[],"file_types":[]}"#,
            r#"{"status":"ready","terms":[["foo"]],"folders":[],"file_types":[],"command":"delete"}"#,
            r#"{"status":"ready","terms":[],"folders":[],"file_types":[]}"#,
            r#"{"status":"ready","terms":[[]],"folders":[],"file_types":[]}"#,
            r#"{"status":"ready","terms":[],"folders":[],"file_types":["exe"]}"#,
            r#"{"status":"ambiguous","terms":[["all"]],"folders":[],"file_types":[]}"#,
            r#"```json {"status":"ready"} ```"#,
        ] {
            assert!(Intent::parse(text).is_err(), "{text}");
        }
        let mut intent = Intent {
            status: IntentStatus::Ready,
            terms: vec![vec!["a".repeat(49)]],
            folders: vec![],
            file_types: vec![],
        };
        assert!(intent.validate().is_err());
        intent.terms = vec![vec!["holiday".into(); 7]];
        assert!(intent.validate().is_err());
        intent.terms = vec![vec!["holiday".into()]; 7];
        assert!(intent.validate().is_err());
    }

    #[test]
    fn token_matching_is_diacritic_aware_and_not_arbitrary_substring_search() {
        assert_eq!(fold("ČERVENEC DOVOLENÁ ŘÍJEN"), "cervenec dovolena rijen");
        assert_eq!(fold("ČEZ"), "cez");
        assert_eq!(fold("C\u{030c}EZ"), "cez");
        assert_eq!(fold("PŘÍLIŠ ŽLUŤOUČKÝ KŮŇ"), "prilis zlutoucky kun");
        assert_eq!(term_score("Srbsko", "srbsko"), Some(30));
        assert_eq!(term_score("my-holiday-photo", "holiday"), Some(20));
        assert_eq!(term_score("Dovolene", "dovolen"), Some(10));
        assert_eq!(term_score("notSerbia", "Serbia"), None);
        assert_eq!(term_score("flight", "fl"), None);
    }
}
