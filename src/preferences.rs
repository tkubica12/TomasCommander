use crate::files::FileResult;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug)]
pub struct Preferences {
    pub dark: bool,
    pub accent: usize,
    pub zoom: f32,
    pub favorites: Vec<PathBuf>,
    pub favorite_names: std::collections::BTreeMap<PathBuf, String>,
    pub recent_documents: Vec<PathBuf>,
    pub recent_roots: Option<Vec<PathBuf>>,
    /// Saved tab folders of the left and right pane; `None` opens the launch folder.
    pub tabs: [Option<PaneTabs>; 2],
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            dark: true,
            accent: 0,
            zoom: 1.0,
            favorites: Vec::new(),
            favorite_names: Default::default(),
            recent_documents: Vec::new(),
            recent_roots: None,
            tabs: [None, None],
        }
    }
}

/// One pane's tab folders, left to right, and the active tab.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneTabs {
    pub folders: Vec<PathBuf>,
    pub active: usize,
}

impl PaneTabs {
    pub const MAX: usize = 32;

    /// Keeps persistable folders (absolute Unicode paths), remapping the active tab.
    pub fn persistable(folders: Vec<PathBuf>, active: usize) -> Option<Self> {
        Self::select(folders.into_iter(), active, |folder| {
            (folder.is_absolute() && folder.to_str().is_some()).then_some(folder)
        })
    }

    /// Replaces each folder with its nearest existing ancestor accepted by `resolve`
    /// and drops folders without one. `None` when no tab survives.
    pub fn restore(&self, resolve: impl Fn(&Path) -> Option<PathBuf>) -> Option<Self> {
        Self::select(self.folders.iter(), self.active, |folder| {
            nearest_folder(folder, &resolve)
        })
    }

    /// The tabs to reopen at launch. With a fixture boundary every folder is checked like
    /// [`Self::restore`], limited to existing folders inside the fixture. Otherwise no
    /// filesystem access happens: absolute folders are kept verbatim and the scan worker falls
    /// back to the nearest existing ancestor when a tab's folder loads, so an unreachable network
    /// folder cannot delay launch.
    pub fn restore_in(&self, scope: &crate::files::Scope) -> Option<Self> {
        if scope.root().is_some() {
            return self.restore(|path| existing_folder(scope, path));
        }
        Self::persistable(self.folders.clone(), self.active)
    }

    fn select<T>(
        folders: impl Iterator<Item = T>,
        active: usize,
        mut keep: impl FnMut(T) -> Option<PathBuf>,
    ) -> Option<Self> {
        let mut kept = Vec::new();
        let mut selected = 0;
        for (index, folder) in folders.enumerate() {
            if kept.len() == Self::MAX {
                break;
            }
            if let Some(folder) = keep(folder) {
                // A dropped active tab falls back to the nearest kept tab on its left.
                if index <= active {
                    selected = kept.len();
                }
                kept.push(folder);
            }
        }
        (!kept.is_empty()).then_some(Self {
            folders: kept,
            active: selected,
        })
    }

    fn validate(&self) -> FileResult<()> {
        if self.folders.is_empty()
            || self.folders.len() > Self::MAX
            || self.active >= self.folders.len()
            || self
                .folders
                .iter()
                .any(|folder| !folder.is_absolute() || folder.to_str().is_none())
        {
            return Err(format!(
                "Pane tabs need 1 to {} absolute folders and a valid active tab.",
                Self::MAX
            ));
        }
        Ok(())
    }
}

/// The resolved folder when `path` is an existing directory inside `scope`.
pub fn existing_folder(scope: &crate::files::Scope, path: &Path) -> Option<PathBuf> {
    scope
        .resolve(path)
        .ok()
        .filter(|resolved| resolved.is_dir())
}

/// The first of `path` and its ancestors accepted by `resolve`.
pub fn nearest_folder(path: &Path, resolve: impl Fn(&Path) -> Option<PathBuf>) -> Option<PathBuf> {
    path.ancestors().find_map(resolve)
}

impl Preferences {
    pub fn favorite_name(&self, path: &Path) -> String {
        self.favorite_names.get(path).cloned().unwrap_or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| crate::files::display_path(path))
        })
    }

    pub fn remove_favorite(&mut self, path: &Path) {
        self.favorites.retain(|stored| stored != path);
        self.favorite_names.remove(path);
    }

    pub fn name_favorite(&mut self, path: PathBuf, name: String) -> FileResult<()> {
        if !self.favorites.contains(&path) || name.trim().is_empty() || name.contains(['\r', '\n'])
        {
            return Err(
                "Saved place needs an existing location and a nonempty single-line name.".into(),
            );
        }
        self.favorite_names.insert(path, name.trim().to_owned());
        Ok(())
    }
    pub const MAX_RECENT: usize = 20;
    pub const MIN_ZOOM: f32 = 0.75;
    pub const MAX_ZOOM: f32 = 2.0;

    pub fn valid_zoom(zoom: f32) -> bool {
        zoom.is_finite() && (Self::MIN_ZOOM..=Self::MAX_ZOOM).contains(&zoom)
    }

    pub fn record_document_open(&mut self, path: PathBuf) -> bool {
        if !document_path(&path) {
            return false;
        }
        self.recent_documents.retain(|recent| recent != &path);
        self.recent_documents.insert(0, path);
        self.recent_documents.truncate(Self::MAX_RECENT);
        true
    }

    pub fn load(path: &Path) -> FileResult<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(format!(
                    "Read preferences {}: {error}",
                    crate::files::display_path(path)
                ));
            }
        };
        let mut lines = text.lines();
        if !matches!(
            lines.next(),
            Some(
                "TomasCommander preferences v1"
                    | "TomasCommander preferences v2"
                    | "TomasCommander preferences v3"
                    | "TomasCommander preferences v4"
                    | "TomasCommander preferences v5"
                    | "TomasCommander preferences v6"
            )
        ) {
            return Err(
                "Unrecognized preferences format; saving is disabled to preserve the file.".into(),
            );
        }
        let mut preferences = Self::default();
        let mut tabs_seen = false;
        for line in lines {
            if let Some(value) = line.strip_prefix("dark=") {
                preferences.dark = match value {
                    "true" => true,
                    "false" => false,
                    _ => return Err("Invalid theme preference.".into()),
                };
            } else if let Some(value) = line.strip_prefix("accent=") {
                preferences.accent = value
                    .parse()
                    .map_err(|e| format!("Invalid accent preference: {e}"))?;
                if preferences.accent > 3 {
                    return Err("Accent preference is out of range.".into());
                }
            } else if let Some(value) = line.strip_prefix("zoom=") {
                preferences.zoom = value
                    .parse()
                    .map_err(|e| format!("Invalid zoom preference: {e}"))?;
                if !Self::valid_zoom(preferences.zoom) {
                    return Err("Zoom preference must be between 75% and 200%.".into());
                }
            } else if let Some(value) = line.strip_prefix("favorite=") {
                preferences.favorites.push(PathBuf::from(value));
            } else if let Some(value) = line.strip_prefix("favorite_name=") {
                let (path, name): (PathBuf, String) = serde_json::from_str(value)
                    .map_err(|error| format!("Invalid saved place name: {error}"))?;
                preferences.name_favorite(path, name)?;
            } else if let Some(value) = line.strip_prefix("recent_roots=") {
                if preferences.recent_roots.is_some() {
                    return Err("Duplicate Recent root configuration.".into());
                }
                let roots: Vec<PathBuf> = serde_json::from_str(value)
                    .map_err(|error| format!("Invalid Recent roots: {error}"))?;
                validate_recent_roots(&roots)?;
                preferences.recent_roots = Some(roots);
            } else if let Some(value) = line.strip_prefix("tabs=") {
                if tabs_seen {
                    return Err("Duplicate pane tab configuration.".into());
                }
                tabs_seen = true;
                let tabs: [Option<PaneTabs>; 2] = serde_json::from_str(value)
                    .map_err(|error| format!("Invalid pane tabs: {error}"))?;
                for pane in tabs.iter().flatten() {
                    pane.validate()?;
                }
                preferences.tabs = tabs;
            } else if let Some(value) = line.strip_prefix("recent=") {
                let path = PathBuf::from(value);
                if !document_path(&path) || preferences.recent_documents.len() >= Self::MAX_RECENT {
                    return Err("Invalid recent-document preference.".into());
                }
                if preferences.recent_documents.contains(&path) {
                    return Err("Duplicate recent-document preference.".into());
                }
                preferences.recent_documents.push(path);
            } else if !line.is_empty() {
                return Err(format!("Unknown preference field: {line}"));
            }
        }
        Ok(preferences)
    }

    pub fn save(&self, path: &Path) -> FileResult<()> {
        if self.accent > 3 || self.recent_documents.len() > Self::MAX_RECENT {
            return Err("Accent preference is out of range.".into());
        }
        if !Self::valid_zoom(self.zoom) {
            return Err("Zoom preference must be between 75% and 200%.".into());
        }
        Self::load(path)?;
        let parent = path.parent().ok_or("Preferences path has no parent.")?;
        fs::create_dir_all(parent).map_err(|e| format!("Create preferences directory: {e}"))?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("System clock: {e}"))?
            .as_nanos();
        let temporary = parent.join(format!("preferences-{nonce}.tmp"));
        let mut text = format!(
            "TomasCommander preferences v6\ndark={}\naccent={}\nzoom={}\n",
            self.dark, self.accent, self.zoom
        );
        if let Some(roots) = &self.recent_roots {
            validate_recent_roots(roots)?;
            text.push_str(&format!(
                "recent_roots={}\n",
                serde_json::to_string(roots)
                    .map_err(|error| format!("Encode Recent roots: {error}"))?
            ));
        }
        if self.tabs.iter().any(Option::is_some) {
            for pane in self.tabs.iter().flatten() {
                pane.validate()?;
            }
            text.push_str(&format!(
                "tabs={}\n",
                serde_json::to_string(&self.tabs)
                    .map_err(|error| format!("Encode pane tabs: {error}"))?
            ));
        }
        for (field, paths) in [
            ("favorite", &self.favorites),
            ("recent", &self.recent_documents),
        ] {
            for stored in paths {
                let path = stored
                    .to_str()
                    .ok_or("Stored path cannot be persisted as Unicode text.")?;
                if path.contains(['\n', '\r']) {
                    return Err("Stored path contains a line break.".into());
                }
                text.push_str(&format!("{field}={path}\n"));
            }
        }
        for (path, name) in &self.favorite_names {
            if !self.favorites.contains(path)
                || name.trim().is_empty()
                || name.contains(['\r', '\n'])
            {
                return Err("Invalid saved place name.".into());
            }
            text.push_str(&format!(
                "favorite_name={}\n",
                serde_json::to_string(&(path, name))
                    .map_err(|error| format!("Encode saved place: {error}"))?
            ));
        }
        let result = (|| -> FileResult<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|e| format!("Create preferences transaction: {e}"))?;
            file.write_all(text.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|e| format!("Write preferences: {e}"))?;
            drop(file);
            fs::rename(&temporary, path).map_err(|e| format!("Commit preferences: {e}"))?;
            Ok(())
        })();
        if result.is_err() && temporary.exists() {
            fs::remove_file(&temporary).map_err(|e| {
                format!("Preferences failed and temporary-file cleanup failed: {e}")
            })?;
        }

        result
    }
}

fn validate_recent_roots(roots: &[PathBuf]) -> FileResult<()> {
    if roots.len() > 16 || roots.iter().any(|path| !path.is_absolute()) {
        return Err("Recent roots must be at most 16 absolute folder paths.".into());
    }
    Ok(())
}

pub(crate) fn document_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "ppt"
                    | "pptx"
                    | "pptm"
                    | "doc"
                    | "docx"
                    | "docm"
                    | "odt"
                    | "rtf"
                    | "txt"
                    | "md"
                    | "markdown"
                    | "html"
                    | "htm"
                    | "pdf"
                    | "xls"
                    | "xlsx"
                    | "xlsm"
                    | "ods"
                    | "csv"
                    | "zip"
            )
        })
}
