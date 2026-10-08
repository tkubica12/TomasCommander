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
    pub favorites: Vec<PathBuf>,
    pub recent_documents: Vec<PathBuf>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            dark: true,
            accent: 0,
            favorites: Vec::new(),
            recent_documents: Vec::new(),
        }
    }
}

impl Preferences {
    pub const MAX_RECENT: usize = 20;

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
            Some("TomasCommander preferences v1" | "TomasCommander preferences v2")
        ) {
            return Err(
                "Unrecognized preferences format; saving is disabled to preserve the file.".into(),
            );
        }
        let mut preferences = Self::default();
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
            } else if let Some(value) = line.strip_prefix("favorite=") {
                preferences.favorites.push(PathBuf::from(value));
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
        Self::load(path)?;
        let parent = path.parent().ok_or("Preferences path has no parent.")?;
        fs::create_dir_all(parent).map_err(|e| format!("Create preferences directory: {e}"))?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("System clock: {e}"))?
            .as_nanos();
        let temporary = parent.join(format!("preferences-{nonce}.tmp"));
        let mut text = format!(
            "TomasCommander preferences v2\ndark={}\naccent={}\n",
            self.dark, self.accent
        );
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

fn document_path(path: &Path) -> bool {
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
            )
        })
}
