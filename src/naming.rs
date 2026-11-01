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

/// Advances the rightmost version token, then a month/year, then a numeric suffix.
pub fn next_clone_name(name: &str) -> FileResult<String> {
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Filename is not valid Unicode.")?;
    let extension = path.extension().and_then(|s| s.to_str());
    let lower = stem.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let increment = |start: usize, end: usize| -> FileResult<String> {
        let digits = &stem[start..end];
        let number = digits
            .parse::<u64>()
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or("Version number is too large to increment.")?;
        Ok(format!(
            "{}{:0width$}{}",
            &stem[..start],
            number,
            &stem[end..],
            width = digits.len()
        ))
    };
    let mut version = None;
    for (start, _) in lower.char_indices() {
        if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
            continue;
        }
        for prefix in ["version", "verze", "vers", "ver", "rev", "v", "r"] {
            if !lower[start..].starts_with(prefix) {
                continue;
            }
            let mut number = start + prefix.len();
            while number < bytes.len() && b" -_.".contains(&bytes[number]) {
                number += 1;
            }
            let mut end = number;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > number && (end == bytes.len() || !bytes[end].is_ascii_alphanumeric()) {
                version = Some((number, end));
                break;
            }
        }
    }
    let months = [
        ("january", "jan"),
        ("february", "feb"),
        ("march", "mar"),
        ("april", "apr"),
        ("may", "may"),
        ("june", "jun"),
        ("july", "jul"),
        ("august", "aug"),
        ("september", "sep"),
        ("october", "oct"),
        ("november", "nov"),
        ("december", "dec"),
    ];
    let mut month = None;
    if version.is_none() {
        for (start, _) in lower.char_indices() {
            if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
                continue;
            }
            for (index, (full, short)) in months.iter().enumerate() {
                for token in [*full, *short] {
                    if !lower[start..].starts_with(token) {
                        continue;
                    }
                    let mut year = start + token.len();
                    while year < bytes.len() && b" -_".contains(&bytes[year]) {
                        year += 1;
                    }
                    let mut end = year;
                    while end < bytes.len() && bytes[end].is_ascii_digit() {
                        end += 1;
                    }
                    if matches!(end - year, 2 | 4)
                        && (end == bytes.len() || !bytes[end].is_ascii_alphanumeric())
                    {
                        month = Some((start, token.len(), year, end, index));
                    }
                }
            }
        }
    }
    let next = if let Some((start, end)) = version {
        increment(start, end)?
    } else if let Some((start, length, year, end, index)) = month {
        let token = &stem[start..start + length];
        let (full, short) = months[(index + 1) % 12];
        let replacement = if length == months[index].0.len() {
            full
        } else {
            short
        };
        let replacement = if token.bytes().all(|b| b.is_ascii_uppercase()) {
            replacement.to_ascii_uppercase()
        } else if token.as_bytes()[0].is_ascii_uppercase() {
            format!(
                "{}{}",
                replacement[..1].to_ascii_uppercase(),
                &replacement[1..]
            )
        } else {
            replacement.to_owned()
        };
        let year_text = if index == 11 {
            let value = stem[year..end]
                .parse::<u32>()
                .map_err(|e| format!("Read year: {e}"))?
                + 1;
            format!(
                "{:0width$}",
                value % if end - year == 2 { 100 } else { 10_000 },
                width = end - year
            )
        } else {
            stem[year..end].to_owned()
        };
        format!(
            "{}{replacement}{}{year_text}{}",
            &stem[..start],
            &stem[start + length..year],
            &stem[end..]
        )
    } else {
        let start = stem.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        if start < stem.len() && start > 0 && matches!(bytes[start - 1], b'-' | b'_') {
            increment(start, stem.len())?
        } else {
            format!("{stem}-v2")
        }
    };
    let next = match extension {
        Some(extension) => format!("{next}.{extension}"),
        None => next,
    };
    validate_name(&next)?;
    Ok(next)
}

pub fn suggest_clone_name(source: &Path) -> FileResult<String> {
    let folder = source.parent().ok_or("Clone source has no folder.")?;
    let mut name = source
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Filename is not valid Unicode.")?
        .to_owned();
    for _ in 0..10_000 {
        name = next_clone_name(&name)?;
        match fs::symlink_metadata(folder.join(&name)) {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(name),
            Err(error) => return Err(format!("Inspect Clone destination: {error}")),
        }
    }
    Err("No free Clone name found within 10000 candidates.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_names_advance_versions_months_suffixes_and_keep_extensions() {
        for (source, expected) in [
            ("report-v1.docx", "report-v2.docx"),
            ("report_v009.PDF", "report_v010.PDF"),
            ("ver1.txt", "ver2.txt"),
            ("verze1.txt", "verze2.txt"),
            ("version1.txt", "version2.txt"),
            ("report VERSION-01.xlsx", "report VERSION-02.xlsx"),
            ("report rev_2.txt", "report rev_3.txt"),
            ("report v1 v3.txt", "report v1 v4.txt"),
            ("Budget-Jun26.xlsx", "Budget-Jul26.xlsx"),
            ("Budget-DEC26.xlsx", "Budget-JAN27.xlsx"),
            ("budget-dec99.xlsx", "budget-jan00.xlsx"),
            ("Budget June 2026.xlsx", "Budget July 2026.xlsx"),
            ("Budget December-2026.xlsx", "Budget January-2027.xlsx"),
            ("report-1.txt", "report-2.txt"),
            ("report_01.txt", "report_02.txt"),
            ("report.txt", "report-v2.txt"),
            ("report", "report-v2"),
            (".gitignore", ".gitignore-v2"),
            ("archive.part.txt", "archive.part-v2.txt"),
            ("invoice123.txt", "invoice123-v2.txt"),
            ("deliver1.txt", "deliver1-v2.txt"),
            ("report-Jun26-v1.txt", "report-Jun26-v2.txt"),
            (
                "P\u{0159}\u{00ed}loha-v1.txt",
                "P\u{0159}\u{00ed}loha-v2.txt",
            ),
        ] {
            assert_eq!(next_clone_name(source).unwrap(), expected, "{source}");
        }
        assert!(next_clone_name("report-v18446744073709551615.txt").is_err());
    }

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
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

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
            effects: crate::undo::Effects {
                kind: Some(if self.source.is_some() {
                    crate::undo::Kind::Rename
                } else {
                    crate::undo::Kind::NewFolder
                }),
                ..Default::default()
            },
            ..Default::default()
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
                outcome
                    .effects
                    .moved
                    .push((source.clone(), self.target.clone()));
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
                outcome.effects.created.push(self.target.clone());
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
