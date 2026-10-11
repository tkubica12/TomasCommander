use crate::{
    changes,
    files::{self, FileResult, Fingerprint, Outcome, Scope},
    journal::Journal,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub fn validate_name(name: &str) -> FileResult<()> {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if name.is_empty()
        || name.encode_utf16().count() > 255
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        || [
            "COM\u{00b9}",
            "COM\u{00b2}",
            "COM\u{00b3}",
            "LPT\u{00b9}",
            "LPT\u{00b2}",
            "LPT\u{00b3}",
        ]
        .contains(&stem.as_str())
    {
        return Err("Use one valid Windows filename, not a path, reserved device name or trailing dot/space.".into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_names_include_device_aliases_and_utf16_limits() {
        for name in [
            "NUL .txt",
            "COM\u{00b9}.txt",
            "LPT\u{00b2}",
            "CON",
            "AUX.log",
        ] {
            assert!(validate_name(name).is_err(), "{name}");
        }
        assert!(validate_name(&"\u{1f600}".repeat(128)).is_err());
        assert!(validate_name(&"\u{1f600}".repeat(127)).is_ok());
        assert!(validate_name("P\u{0159}\u{00ed}li\u{0161} folder.txt").is_ok());
    }
}

#[derive(Clone)]
pub struct Plan {
    pub id: u64,
    key: String,
    source: Option<PathBuf>,
    target: PathBuf,
    parent: PathBuf,
    parent_identity: Fingerprint,
    source_identity: Option<Fingerprint>,
}

impl Plan {
    pub fn target(&self) -> &Path {
        &self.target
    }

    pub fn prepare(
        scope: &Scope,
        id: u64,
        folder: &Path,
        source: Option<&Path>,
        name: &str,
    ) -> FileResult<Self> {
        validate_name(name)?;
        let parent = scope.resolve(folder)?;
        let parent_identity = files::fingerprint(&parent)?;
        if !parent_identity.directory {
            return Err("Destination must be a folder.".into());
        }
        let target = parent.join(name);
        if target
            .try_exists()
            .map_err(|e| format!("Inspect exact name target: {e}"))?
        {
            return Err(
                "That name already exists; no overwrite, merge or case-only rename.".into(),
            );
        }
        let source = source.map(|path| scope.mutation_source(path)).transpose()?;
        if source
            .as_ref()
            .is_some_and(|path| path.parent() != Some(parent.as_path()))
        {
            return Err("Rename must stay in the displayed source folder.".into());
        }
        let source_identity = source
            .as_ref()
            .map(|path| files::fingerprint(path))
            .transpose()?;
        Ok(Self {
            id,
            key: changes::identity()?,
            source,
            target,
            parent,
            parent_identity,
            source_identity,
        })
    }

    pub fn label(&self) -> &'static str {
        if self.source.is_some() {
            "Rename"
        } else {
            "New folder"
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "{}\nSource: {}\nExact target: {}\nNo overwrite or merge. Rename stays in the same folder.",
            self.label(),
            self.source
                .as_ref()
                .map_or_else(|| "(new empty folder)".into(), |p| files::display_path(p)),
            files::display_path(&self.target)
        )
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
            completed: vec![],
            created: vec![],
            incomplete: vec![self.source.clone().unwrap_or_else(|| self.target.clone())],
            error: None,
            cancelled: false,
        };
        if !approved || cancel.load(Ordering::Acquire) {
            outcome.error = Some(
                "Explicit exact-plan approval is required and cancellation must be clear.".into(),
            );
            outcome.cancelled = cancel.load(Ordering::Acquire);
            return outcome;
        }
        let snapshot = format!(
            "operation={}\nid={}\nsource={}\ntarget={}\nparent_identity={:?}\nsource_identity={:?}\n",
            self.label(),
            self.id,
            crate::journal::encode_path(self.source.as_deref().unwrap_or(&self.target)),
            crate::journal::encode_path(&self.target),
            self.parent_identity,
            self.source_identity
        );
        let paths = vec![
            self.source.clone().unwrap_or_else(|| self.target.clone()),
            self.target.clone(),
        ];
        let record = match journal.claim_snapshot(&self.key, &snapshot, &paths) {
            Ok(record) => record,
            Err(error) => {
                outcome.error = Some(error);
                return outcome;
            }
        };
        let result = (|| -> FileResult<()> {
            #[cfg(windows)]
            let _guards = files::lock_mutation_parents(&self.parent)?;
            if scope.resolve(&self.parent)? != self.parent
                || files::fingerprint(&self.parent)? != self.parent_identity
            {
                return Err("Source/destination folder changed since approval.".into());
            }
            if self
                .target
                .try_exists()
                .map_err(|e| format!("Inspect target: {e}"))?
            {
                return Err("Target now exists; no overwrite.".into());
            }
            if cancel.load(Ordering::Acquire) {
                return Err("Name operation cancelled before effects.".into());
            }
            if let Some(source) = &self.source {
                let identity = self
                    .source_identity
                    .as_ref()
                    .ok_or("Missing approved source identity.")?;
                if scope.mutation_source(source)? != *source
                    || files::fingerprint(source)? != *identity
                {
                    return Err("Source changed since approval.".into());
                }
                #[cfg(windows)]
                crate::platform::move_verified(
                    source,
                    &self.target,
                    identity,
                    self.parent_identity.file_id,
                )?;
                #[cfg(not(windows))]
                crate::platform::move_no_replace(source, &self.target)?;
                outcome.created.push(self.target.clone());
                if files::fingerprint(&self.target)? != *identity
                    || source
                        .try_exists()
                        .map_err(|e| format!("Read back rename: {e}"))?
                {
                    return Err(
                        "Rename read-back is uncertain; inspect both paths before retrying.".into(),
                    );
                }
            } else {
                fs::create_dir(&self.target)
                    .map_err(|e| format!("Create exact folder without merge: {e}"))?;
                outcome.created.push(self.target.clone());
                if !files::fingerprint(&self.target)?.directory
                    || scope.resolve(&self.target)? != self.target
                {
                    return Err(
                        "New folder read-back failed; inspect exact target before retrying.".into(),
                    );
                }
            }
            outcome.completed = outcome.incomplete.clone();
            outcome.incomplete.clear();
            Ok(())
        })();
        if let Err(error) = result {
            outcome.error = Some(error);
        }
        outcome.cancelled = cancel.load(Ordering::Acquire) && outcome.completed.is_empty();
        Journal::finish(record, outcome)
    }
}
