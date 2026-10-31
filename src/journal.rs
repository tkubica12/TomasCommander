use crate::files::{self, FileResult, Outcome, Plan, Scope};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

pub(crate) const HEADER: &str = "TomasCommander operation journal v1\n";
const MAX_RECORD_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RECORDS: usize = 10_000;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

pub struct Journal {
    root: PathBuf,
}

impl Journal {
    #[cfg(feature = "native-desktop-tests")]
    pub fn desktop_rejection_probe(
        scope: &Scope,
        root: PathBuf,
        source: &Path,
        destination: &Path,
    ) -> FileResult<String> {
        if root
            .try_exists()
            .map_err(|error| format!("Inspect rejection journal: {error}"))?
        {
            return Err(
                "Rejection probe journal already exists; inspect it, do not retry or overwrite."
                    .into(),
            );
        }
        let journal = Self::new(root);
        journal.initialize()?;
        let plan = files::plan(
            scope,
            1,
            files::Operation::Copy,
            &[source.to_owned()],
            Some(destination),
        )?;
        let original = files::fingerprint(source)?;
        let cancel = AtomicBool::new(false);
        let mut report = String::from(
            "TEST ONLY / synthetic snapshots and claimed receipt / real rejection paths / zero fixture effects\n",
        );
        for (label, candidate, approved, expected) in [
            (
                "Unapproved exact plan",
                plan.clone(),
                false,
                "Explicit plan approval",
            ),
            (
                "Injected stale source snapshot",
                plan.desktop_test_stale_snapshot(false)?,
                true,
                "Source changed",
            ),
            (
                "Injected stale destination snapshot",
                plan.desktop_test_stale_snapshot(true)?,
                true,
                "Destination folder changed",
            ),
            (
                "Injected existing target binding",
                plan.desktop_test_existing_target()?,
                true,
                "Destination changed; no overwrite",
            ),
        ] {
            let outcome = files::execute(scope, &candidate, approved, &cancel);
            let error = outcome
                .error
                .as_deref()
                .ok_or_else(|| format!("{label}: expected refusal, got no error."))?;
            if !error.contains(expected)
                || !outcome.created.is_empty()
                || !outcome.completed.is_empty()
            {
                return Err(format!("{label}: rejection invariant failed: {outcome:?}"));
            }
            report.push_str(&format!("{label}: {error}\n"));
        }
        // Seed only diagnostic journal metadata, never a successful operation.
        let mut receipt = journal.claim(&plan)?;
        receipt.write_all(b"\nerror=\"TEST ONLY synthetic claimed receipt; no operation executed\"\nfinished\n")
            .and_then(|()| receipt.sync_all())
            .map_err(|error| format!("Persist diagnostic replay receipt: {error}"))?;
        drop(receipt);
        let replay = journal.execute(scope, &plan, true, &cancel);
        let error = replay
            .error
            .as_deref()
            .ok_or("Duplicate replay probe did not refuse.")?;
        if !error.contains("Claim operation once")
            || !replay.created.is_empty()
            || !replay.completed.is_empty()
        {
            return Err(format!(
                "Duplicate replay rejection invariant failed: {replay:?}"
            ));
        }
        report.push_str(&format!(
            "Injected already-claimed receipt / replay: {error}\n"
        ));
        if files::fingerprint(source)? != original
            || destination
                .join(source.file_name().ok_or("Probe source has no filename.")?)
                .try_exists()
                .map_err(|error| format!("Read back rejection target: {error}"))?
        {
            return Err("Rejection probe source/target read-back failed.".into());
        }
        Ok(report)
    }

    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn initialize(&self) -> FileResult<()> {
        fs::create_dir_all(&self.root).map_err(|e| format!("Create operation journal: {e}"))?;
        if files::reparse(
            &fs::symlink_metadata(&self.root).map_err(|e| format!("Inspect journal: {e}"))?,
        ) {
            return Err("Operation journal cannot be a link. Mutations are stopped.".into());
        }
        Ok(())
    }

    pub(crate) fn records(&self) -> FileResult<Vec<(PathBuf, String)>> {
        if !self
            .root
            .try_exists()
            .map_err(|e| format!("Inspect journal: {e}"))?
        {
            return Ok(Vec::new());
        }
        if files::reparse(
            &fs::symlink_metadata(&self.root).map_err(|e| format!("Inspect journal: {e}"))?,
        ) {
            return Err("Operation journal cannot be a link. Mutations are stopped.".into());
        }
        let mut records = Vec::new();
        let mut total_bytes = 0;
        for entry in fs::read_dir(&self.root).map_err(|e| format!("Read operation journal: {e}"))? {
            let path = entry
                .map_err(|e| format!("Read operation record: {e}"))?
                .path();
            if path
                .extension()
                .is_none_or(|extension| extension != "operation")
            {
                continue;
            }
            if records.len() == MAX_RECORDS {
                return Err(
                    "Operation journal limit reached; archive reviewed records before mutating."
                        .into(),
                );
            }
            let metadata = fs::symlink_metadata(&path)
                .map_err(|e| format!("Inspect operation record: {e}"))?;
            total_bytes += metadata.len();
            if total_bytes > MAX_TOTAL_BYTES {
                return Err("Operation journal size limit reached; archive reviewed records before mutating.".into());
            }
            if !metadata.is_file() || files::reparse(&metadata) || metadata.len() > MAX_RECORD_BYTES
            {
                return Err(format!(
                    "Invalid operation record; inspect {} before mutating.",
                    files::display_path(&path)
                ));
            }
            let mut text = String::new();
            File::open(&path)
                .and_then(|file| file.take(MAX_RECORD_BYTES + 1).read_to_string(&mut text))
                .map_err(|e| {
                    format!("Read operation record {}: {e}", files::display_path(&path))
                })?;
            if !text.starts_with(HEADER) || text.len() as u64 > MAX_RECORD_BYTES {
                return Err(format!(
                    "Damaged operation record; inspect {} before mutating.",
                    files::display_path(&path)
                ));
            }
            records.push((path, text));
        }
        Ok(records)
    }

    pub fn pending(&self) -> FileResult<Vec<String>> {
        self.records()?
            .into_iter()
            .filter(|(_, text)| !text.ends_with("finished\n"))
            .map(|(path, text)| {
                let paths = recorded_paths(&text)?;
                Ok(format!(
                    "Interrupted operation: {}\n{} recorded paths (first 20):\n{}\nRead back sources and destinations before a new approval. This record is never replayed. After manual reconciliation, archive this exact record outside the journal.",
                    files::display_path(&path),
                    paths.len(),
                    paths.iter().take(20).map(|path| files::display_path(path)).collect::<Vec<_>>().join("\n")
                ))
            })
            .collect()
    }

    fn claim(&self, plan: &Plan) -> FileResult<File> {
        self.claim_snapshot(
            plan.execution_key(),
            &plan.journal_snapshot(),
            &plan.affected_paths(),
        )
    }

    pub(crate) fn claim_snapshot(
        &self,
        key: &str,
        snapshot: &str,
        affected: &[PathBuf],
    ) -> FileResult<File> {
        self.initialize()?;
        // Serialize claim/check across instances; an abandoned lock handle is released by Windows.
        let lock_path = self.root.join("journal.lock");
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0).custom_flags(0x0020_0000);
        }
        let _lock = options
            .open(&lock_path)
            .map_err(|e| format!("Lock operation journal; no changes made: {e}"))?;
        let records = self.records()?;
        for (path, text) in &records {
            if !text.ends_with("finished\n") {
                let recorded = recorded_paths(text)?;
                if recorded.is_empty()
                    || recorded.iter().any(|old| {
                        affected
                            .iter()
                            .any(|new| new.starts_with(old) || old.starts_with(new))
                    })
                {
                    return Err(format!(
                        "Unresolved operation overlaps this plan; read back and reconcile {}. No blind retry or automatic rollback.",
                        files::display_path(path)
                    ));
                }
            }
        }
        if let Some(original) = snapshot
            .lines()
            .find_map(|line| line.strip_prefix("undo_of="))
        {
            // Checked under the lock: one effective undo per operation, across instances.
            let marker = format!("\nundo_of={original}\n");
            let mut found = false;
            for (path, text) in &records {
                if path.file_stem().and_then(|stem| stem.to_str()) == Some(original) {
                    found = text.ends_with("finished\n");
                } else if text.contains(&marker) {
                    if !text.ends_with("finished\n") {
                        return Err(format!(
                            "An undo of this operation was interrupted; reconcile {} first. No changes made.",
                            files::display_path(path)
                        ));
                    }
                    if crate::undo::undo_changed(text) {
                        return Err("This operation was already undone; no changes made.".into());
                    }
                }
            }
            if !found {
                return Err(
                    "The undone operation record is missing or unfinished; no changes made.".into(),
                );
            }
        }
        let path = self.root.join(format!("{key}.operation"));
        let mut record = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| {
                format!(
                    "Claim operation once; inspect {} before retrying: {e}",
                    files::display_path(&path)
                )
            })?;
        // The start time bounds Recycle Bin matches for a later undo.
        let snapshot = format!("{HEADER}{snapshot}started={}\n", crate::undo::now());
        if snapshot.len() as u64 > MAX_RECORD_BYTES {
            return Err(
                "Approved plan exceeds the durable journal limit; no mutation performed.".into(),
            );
        }
        record
            .write_all(snapshot.as_bytes())
            .and_then(|()| record.sync_all())
            .map_err(|e| format!("Persist approved plan; no mutation performed: {e}"))?;
        Ok(record)
    }

    pub fn execute(
        &self,
        scope: &Scope,
        plan: &Plan,
        approved: bool,
        cancelled: &AtomicBool,
    ) -> Outcome {
        self.execute_with(scope, plan, approved, cancelled, &files::Hooks::default())
    }

    /// `execute` with the file-operation test seams.
    #[doc(hidden)]
    pub fn execute_with(
        &self,
        scope: &Scope,
        plan: &Plan,
        approved: bool,
        cancelled: &AtomicBool,
        hooks: &files::Hooks,
    ) -> Outcome {
        if !approved {
            return files::execute_with(scope, plan, false, cancelled, hooks);
        }
        let record = match self.claim(plan) {
            Ok(record) => record,
            Err(error) => {
                return Outcome {
                    id: plan.id(),
                    completed: Vec::new(),
                    created: Vec::new(),
                    incomplete: plan.sources().to_vec(),
                    cancelled: false,
                    error: Some(error),
                    operation: Some(plan.operation()),
                    strategy: plan.move_strategy(),
                    ..Default::default()
                };
            }
        };
        Self::finish(
            record,
            files::execute_with(scope, plan, true, cancelled, hooks),
        )
    }

    pub(crate) fn finish(mut record: File, mut outcome: Outcome) -> Outcome {
        let mut result = format!("\ncancelled={}\n", outcome.cancelled);
        for path in &outcome.completed {
            result.push_str(&format!("completed={}\n", encode_path(path)));
        }
        for path in &outcome.created {
            result.push_str(&format!("created={}\n", encode_path(path)));
        }
        for path in &outcome.incomplete {
            result.push_str(&format!("incomplete={}\n", encode_path(path)));
        }
        for (key, paths) in [
            ("replaced", &outcome.replaced),
            ("merged", &outcome.merged),
            ("skipped", &outcome.skipped),
            ("recycled", &outcome.recycled),
            ("kept", &outcome.kept),
        ] {
            for path in paths {
                result.push_str(&format!("{key}={}\n", encode_path(path)));
            }
        }
        let error = outcome
            .error
            .as_ref()
            .map(|error| format!("error={error:?}\n"))
            .unwrap_or_default();
        // Undo data never pushes a record past the journal limit; it is dropped instead.
        let used = record
            .metadata()
            .map_or(MAX_RECORD_BYTES, |metadata| metadata.len())
            + (result.len() + error.len() + "finished\n".len()) as u64;
        result.push_str(
            &outcome
                .effects
                .journal_lines(MAX_RECORD_BYTES.saturating_sub(used + 256)),
        );
        result.push_str(&error);
        if let Err(error) = record
            .write_all(result.as_bytes())
            .and_then(|()| record.sync_all())
        {
            let previous = outcome
                .error
                .take()
                .map(|error| format!("{error}\n"))
                .unwrap_or_default();
            outcome.error = Some(format!(
                "{previous}Could not persist operation outcome: {error}. Outcome is uncertain; inspect the journal and actual paths before retrying."
            ));
            return outcome;
        }
        // Publish completion only after the exact partial/successful outcome has been flushed.
        let finished = record
            .metadata()
            .map(|metadata| metadata.len())
            .and_then(|length| {
                if let Err(error) = record
                    .write_all(b"finished\n")
                    .and_then(|()| record.sync_all())
                {
                    record
                        .set_len(length)
                        .and_then(|()| record.sync_all())
                        .map_err(|rollback| {
                            std::io::Error::other(format!(
                                "{error}; completion marker removal also failed: {rollback}"
                            ))
                        })?;
                    return Err(error);
                }
                Ok(())
            });
        if let Err(error) = finished {
            let previous = outcome
                .error
                .take()
                .map(|error| format!("{error}\n"))
                .unwrap_or_default();
            outcome.error = Some(format!(
                "{previous}Could not settle operation journal: {error}. Inspect actual paths before retrying."
            ));
        }
        outcome
    }
}

pub(crate) fn encode_path(path: &Path) -> String {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str()
            .encode_wide()
            .map(|unit| format!("{unit:04x}"))
            .collect()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy()
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

pub(crate) fn decode_path(value: &str) -> FileResult<PathBuf> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        if !value.is_ascii() || !value.len().is_multiple_of(4) {
            return Err("Damaged journal path; mutations are stopped.".into());
        }
        let units = (0..value.len())
            .step_by(4)
            .map(|index| {
                u16::from_str_radix(&value[index..index + 4], 16)
                    .map_err(|e| format!("Damaged journal path: {e}"))
            })
            .collect::<FileResult<Vec<_>>>()?;
        Ok(std::ffi::OsString::from_wide(&units).into())
    }
    #[cfg(not(windows))]
    {
        if !value.is_ascii() || !value.len().is_multiple_of(2) {
            return Err("Damaged journal path; mutations are stopped.".into());
        }
        let bytes = (0..value.len())
            .step_by(2)
            .map(|index| {
                u8::from_str_radix(&value[index..index + 2], 16)
                    .map_err(|e| format!("Damaged journal path: {e}"))
            })
            .collect::<FileResult<Vec<_>>>()?;
        String::from_utf8(bytes)
            .map(PathBuf::from)
            .map_err(|e| format!("Damaged journal path: {e}"))
    }
}

fn recorded_paths(text: &str) -> FileResult<Vec<PathBuf>> {
    text.lines()
        .filter_map(|line| {
            line.strip_prefix("source=")
                .or_else(|| line.strip_prefix("target="))
        })
        .map(decode_path)
        .collect()
}
