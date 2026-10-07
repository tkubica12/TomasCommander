#![cfg(windows)]

use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::AtomicBool,
    time::{SystemTime, UNIX_EPOCH},
};
use tomas_commander::{
    files::{self, Operation, Scope, Sort},
    preferences::Preferences,
};

struct Fixture {
    root: PathBuf,
    owned: Vec<PathBuf>,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let base = std::env::var_os("TC_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = base.join(format!(
            "tomascommander-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        Self {
            owned: vec![root.clone()],
            root,
        }
    }
    fn directory(&mut self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir(&path).unwrap();
        self.owned.push(path.clone());
        path
    }
    fn file(&mut self, relative: &str, data: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        fs::write(&path, data).unwrap();
        self.owned.push(path.clone());
        path
    }
    fn scope(&self) -> Scope {
        Scope::fixture(&self.root).unwrap()
    }
    fn own(&mut self, paths: &[PathBuf]) {
        self.owned.extend_from_slice(paths);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.owned
            .sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        self.owned.dedup();
        let mut errors = Vec::new();
        for path in &self.owned {
            match fs::symlink_metadata(path) {
                Ok(metadata) => {
                    use std::os::windows::fs::MetadataExt;
                    let result = if metadata.file_attributes() & 0x10 != 0 {
                        fs::remove_dir(path)
                    } else {
                        fs::remove_file(path)
                    };
                    if let Err(error) = result {
                        errors.push(format!("{}: {error}", path.display()));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => errors.push(format!("{}: {error}", path.display())),
            }
        }
        if !errors.is_empty() {
            if std::thread::panicking() {
                eprintln!("Fixture cleanup failed: {}", errors.join("; "));
            } else {
                panic!("Fixture cleanup failed: {}", errors.join("; "));
            }
        }
    }
}

#[test]
fn actual_recursive_copy_preserves_bytes_and_source() {
    let mut fixture = Fixture::new("copy");
    let source = fixture.directory("Source");
    fixture.directory("Source\\Nested");
    fixture.file(
        "Source\\Nested\\Czech-file.txt",
        "Dovolená Srbsko".as_bytes(),
    );
    fixture.file("Source\\presentation.pptx", &vec![0x5a; 190_000]);
    let destination = fixture.directory("Destination");
    let scope = fixture.scope();
    let plan = files::plan(
        &scope,
        1,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(&destination),
    )
    .unwrap();
    let result = files::execute(&scope, &plan, true, &AtomicBool::new(false));
    fixture.own(&result.created);
    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(
        fs::read(source.join("presentation.pptx")).unwrap(),
        fs::read(destination.join("Source\\presentation.pptx")).unwrap()
    );
    assert_eq!(
        fs::read(destination.join("Source\\Nested\\Czech-file.txt")).unwrap(),
        "Dovolená Srbsko".as_bytes()
    );
    assert!(source.exists());
}

#[test]
fn denied_approval_and_precancel_have_no_effects() {
    let mut fixture = Fixture::new("approval");
    let source = fixture.file("source.txt", b"unchanged");
    let destination = fixture.directory("Destination");
    let scope = fixture.scope();
    let plan = files::plan(
        &scope,
        2,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(&destination),
    )
    .unwrap();
    let result = files::execute(&scope, &plan, false, &AtomicBool::new(false));
    assert!(result.error.unwrap().contains("approval"));
    let result = files::execute(&scope, &plan, true, &AtomicBool::new(true));
    assert!(result.cancelled);
    assert!(result.created.is_empty());
    assert_eq!(fs::read(source).unwrap(), b"unchanged");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}

#[test]
fn stale_source_and_destination_refuse_before_mutation() {
    let mut fixture = Fixture::new("stale");
    let source = fixture.file("source.txt", b"before");
    let destination = fixture.directory("Destination");
    let scope = fixture.scope();
    let plan = files::plan(
        &scope,
        3,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(&destination),
    )
    .unwrap();
    fs::write(&source, b"changed-content").unwrap();
    let result = files::execute(&scope, &plan, true, &AtomicBool::new(false));
    assert!(result.error.unwrap().contains("changed"));
    assert!(result.created.is_empty());
    let plan = files::plan(
        &scope,
        4,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(&destination),
    )
    .unwrap();
    fixture.file("Destination\\source.txt", b"sentinel");
    let result = files::execute(&scope, &plan, true, &AtomicBool::new(false));
    assert!(result.error.unwrap().contains("changed"));
    assert_eq!(
        fs::read(destination.join("source.txt")).unwrap(),
        b"sentinel"
    );
    assert!(
        files::plan(&scope, 5, Operation::Copy, &[source], Some(&destination))
            .unwrap_err()
            .contains("Conflict")
    );
}

#[test]
fn added_child_invalidates_the_approved_tree() {
    let mut fixture = Fixture::new("tree-change");
    let source = fixture.directory("Source");
    fixture.file("Source\\one.txt", b"one");
    let destination = fixture.directory("Destination");
    let scope = fixture.scope();
    let plan = files::plan(&scope, 6, Operation::Copy, &[source], Some(&destination)).unwrap();
    fixture.file("Source\\new.txt", b"new");
    let result = files::execute(&scope, &plan, true, &AtomicBool::new(false));
    assert!(result.error.is_some());
    assert!(result.created.is_empty());
}

#[test]
fn actual_move_preserves_content_and_refuses_overwrite() {
    let mut fixture = Fixture::new("move");
    let source = fixture.file("source.txt", b"real-move");
    let destination = fixture.directory("Destination");
    let scope = fixture.scope();
    let plan = files::plan(
        &scope,
        7,
        Operation::Move,
        std::slice::from_ref(&source),
        Some(&destination),
    )
    .unwrap();
    let result = files::execute(&scope, &plan, true, &AtomicBool::new(false));
    fixture.own(&result.created);
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!source.exists());
    assert_eq!(
        fs::read(destination.join("source.txt")).unwrap(),
        b"real-move"
    );
    let second = fixture.file("second.txt", b"do-not-overwrite");
    assert!(
        tomas_commander::platform::move_no_replace(&second, &destination.join("source.txt"))
            .is_err()
    );
    assert_eq!(fs::read(&second).unwrap(), b"do-not-overwrite");
}

#[test]
fn boundary_and_parent_child_selections_are_refused() {
    let mut fixture = Fixture::new("scope");
    let source = fixture.directory("Source");
    let child = fixture.file("Source\\child.txt", b"fixture");
    let destination = fixture.directory("Destination");
    let scope = fixture.scope();
    assert!(scope.resolve(fixture.root.parent().unwrap()).is_err());
    assert!(
        files::plan(
            &scope,
            8,
            Operation::Copy,
            &[source.clone(), child],
            Some(&destination)
        )
        .is_err()
    );
    assert!(
        files::plan(
            &scope,
            9,
            Operation::Copy,
            std::slice::from_ref(&source),
            Some(&source)
        )
        .is_err()
    );
    assert!(
        files::plan(
            &scope,
            10,
            Operation::Recycle,
            &[fixture.root.clone()],
            None
        )
        .is_err()
    );
}

#[test]
fn real_windows_junction_cannot_escape_fixture_scope() {
    let mut fixture = Fixture::new("junction");
    let mut outside = Fixture::new("outside");
    let target = outside.directory("Target");
    outside.file("Target\\sentinel.txt", b"preserve");
    let link = fixture.root.join("Link");
    let output = Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Junction creation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fixture.owned.push(link.clone());
    let destination = fixture.directory("Destination");
    let scope = fixture.scope();
    assert!(scope.resolve(&link).is_err());
    assert!(
        files::plan(&scope, 11, Operation::Copy, &[link], Some(&destination))
            .unwrap_err()
            .contains("reparse")
    );
    assert_eq!(fs::read(target.join("sentinel.txt")).unwrap(), b"preserve");
}

#[test]
fn actual_preferences_roundtrip_and_invalid_file_preservation() {
    let mut fixture = Fixture::new("preferences");
    let directory = fixture.directory("Settings");
    let path = directory.join("preferences.txt");
    let preferences = Preferences {
        dark: false,
        accent: 2,
        favorites: vec![fixture.root.join("Česká složka")],
    };
    preferences.save(&path).unwrap();
    fixture.owned.push(path.clone());
    let loaded = Preferences::load(&path).unwrap();
    assert!(!loaded.dark);
    assert_eq!(loaded.accent, 2);
    assert_eq!(loaded.favorites, preferences.favorites);
    preferences.save(&path).unwrap();
    fs::write(&path, "unrelated-existing-file").unwrap();
    assert!(Preferences::load(&path).is_err());
    assert!(preferences.save(&path).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "unrelated-existing-file");
    assert_eq!(fs::read_dir(directory).unwrap().count(), 1);
}

#[test]
fn directory_sort_and_large_listing_are_real() {
    let mut fixture = Fixture::new("listing");
    fixture.directory("A-folder");
    for index in 0..1000 {
        fixture.file(
            &format!("file-{index:04}.txt"),
            format!("fixture-{index}").as_bytes(),
        );
    }
    let started = std::time::Instant::now();
    let mut listing = files::list_directory(&fixture.scope(), &fixture.root).unwrap();
    println!(
        "1001 real entries listed in {:.2} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    assert!(listing.warnings.is_empty());
    assert_eq!(listing.entries.len(), 1001);
    assert!(listing.entries[0].directory);
    files::sort_entries(&mut listing.entries, Sort::Size);
    assert!(listing.entries[0].directory);
}

#[test]
fn scans_and_planning_honor_cancellation_on_real_fixtures() {
    let mut fixture = Fixture::new("scan-cancel");
    let source = fixture.file("source.txt", b"preserve");
    let destination = fixture.directory("Destination");
    let scope = fixture.scope();
    assert!(
        files::list_directory_cancellable(&scope, &fixture.root, || true)
            .unwrap_err()
            .contains("superseded")
    );
    assert!(
        files::plan_cancellable(
            &scope,
            13,
            Operation::Copy,
            std::slice::from_ref(&source),
            Some(&destination),
            &AtomicBool::new(true)
        )
        .unwrap_err()
        .contains("cancelled")
    );
    assert_eq!(fs::read(source).unwrap(), b"preserve");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}

#[test]
fn multiple_sources_copy_and_locked_source_failure_are_explicit() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut fixture = Fixture::new("multiple");
    let first = fixture.file("one.txt", b"one");
    let second = fixture.file("two.txt", b"two");
    let destination = fixture.directory("Destination");
    let scope = fixture.scope();
    let plan = files::plan(
        &scope,
        14,
        Operation::Copy,
        &[first.clone(), second.clone()],
        Some(&destination),
    )
    .unwrap();
    let result = files::execute(&scope, &plan, true, &AtomicBool::new(false));
    fixture.own(&result.created);
    assert!(result.error.is_none());
    assert_eq!(result.completed.len(), 2);
    assert_eq!(fs::read(destination.join("one.txt")).unwrap(), b"one");
    assert_eq!(fs::read(destination.join("two.txt")).unwrap(), b"two");
    let denied = fixture.directory("Denied");
    let plan = files::plan(
        &scope,
        15,
        Operation::Copy,
        std::slice::from_ref(&first),
        Some(&denied),
    )
    .unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&first)
        .unwrap();
    let result = files::execute(&scope, &plan, true, &AtomicBool::new(false));
    fixture.own(&result.created);
    assert!(
        result.error.is_some(),
        "A locked source must produce a visible error."
    );
    assert!(result.completed.is_empty());
    drop(lock);
    assert_eq!(fs::read(first).unwrap(), b"one");
    assert_eq!(fs::read(second).unwrap(), b"two");
}

#[test]
fn cancellation_during_real_copy_reports_partial_destination_without_deleting_it() {
    use std::sync::Arc;
    let mut fixture = Fixture::new("partial");
    let source = fixture.file("large.bin", &vec![0x5a; 64 * 1024 * 1024]);
    let destination = fixture.directory("Destination");
    let target = destination.join("large.bin");
    fixture.owned.push(target.clone());
    let scope = fixture.scope();
    let plan = files::plan(
        &scope,
        16,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(&destination),
    )
    .unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let worker = std::thread::spawn(move || files::execute(&scope, &plan, true, &worker_cancelled));
    while !worker.is_finished() {
        if target.exists() {
            cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            break;
        }
        std::thread::yield_now();
    }
    let result = worker.join().unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(
        result.cancelled,
        "Cancellation must occur while the actual large copy is in progress."
    );
    assert_eq!(result.created, vec![target.canonicalize().unwrap()]);
    assert!(fs::metadata(&target).unwrap().len() < 64 * 1024 * 1024);
    assert_eq!(fs::metadata(source).unwrap().len(), 64 * 1024 * 1024);
}

#[test]
#[ignore = "Native Recycle Bin test with exact synthetic-item recovery; run explicitly"]
fn recycle_and_restore_only_the_owned_synthetic_item() {
    let mut fixture = Fixture::new("recycle");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("tc-owned-recycle-{nonce}.txt");
    let source = fixture.file(&name, b"restore-this-fixture-only");
    let scope = fixture.scope();
    let plan = files::plan(
        &scope,
        12,
        Operation::Recycle,
        std::slice::from_ref(&source),
        None,
    )
    .unwrap();
    let result = files::execute(&scope, &plan, true, &AtomicBool::new(false));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!source.exists());
    let parent = source
        .parent()
        .unwrap()
        .display()
        .to_string()
        .replace('\'', "''");
    let name = name.trim_end_matches(".txt").replace('\'', "''");
    let script = format!(
        r#"
$ErrorActionPreference='Stop'
$shell=New-Object -ComObject Shell.Application
$matches=@($shell.Namespace(10).Items() | Where-Object {{ $_.ExtendedProperty('System.Recycle.DeletedFrom') -eq '{parent}' -and $_.Name -like '{name}*' }})
if($matches.Count -ne 1) {{ throw "Expected exactly one owned recycle fixture; found $($matches.Count)." }}
$matches[0].InvokeVerb('undelete')
"#
    );
    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "Exact fixture restoration failed; no other Recycle Bin item was touched."
    );
    for _ in 0..60 {
        if source.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(fs::read(&source).unwrap(), b"restore-this-fixture-only");
}
