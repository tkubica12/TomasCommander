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

fn hash_inventory(root: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile", "-NonInteractive", "-Command",
            r#"$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $root=(Get-Item -LiteralPath $env:TC_HASH_ROOT).FullName; Get-ChildItem -LiteralPath $root -Recurse -Force | Sort-Object FullName | ForEach-Object { if ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Hash inventory never follows links.' }; $relative=$_.FullName.Substring($root.Length+1); if ($_.PSIsContainer) { "$relative`tDIRECTORY" } else { $stream=[IO.File]::OpenRead($_.FullName); $sha=[Security.Cryptography.SHA256]::Create(); try { $hash=[BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-',''); "$relative`tFILE $($_.Length) $hash" } finally { $stream.Dispose(); $sha.Dispose() } } }"#,
        ])
        .env("TC_HASH_ROOT", root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Hash inventory failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines = String::from_utf8(output.stdout).unwrap();
    let inventory: std::collections::BTreeMap<_, _> = lines
        .lines()
        .map(|line| {
            let (path, state) = line.split_once('\t').expect("Exact hash inventory format");
            (path.to_owned(), state.to_owned())
        })
        .collect();
    println!("SHA256 INVENTORY {}: {inventory:?}", root.display());
    inventory
}

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
fn display_paths_hide_only_supported_windows_prefixes_without_changing_identity() {
    assert_eq!(
        files::display_path(std::path::Path::new(r"\\?\C:\Users\Česká\file.txt")),
        r"C:\Users\Česká\file.txt"
    );
    assert_eq!(
        files::display_path(std::path::Path::new(r"\\?\UNC\server\share\folder")),
        r"\\server\share\folder"
    );
    assert_eq!(
        files::display_path(std::path::Path::new(r"C:\plain path")),
        r"C:\plain path"
    );
    assert_eq!(
        files::display_path(std::path::Path::new(r"\\?\Volume{example}\")),
        r"\\?\Volume{example}\"
    );
    let fixture = Fixture::new("display-path");
    let resolved = fixture.scope().resolve(&fixture.root).unwrap();
    assert_eq!(
        fixture
            .scope()
            .resolve(&PathBuf::from(files::display_path(&resolved)))
            .unwrap(),
        resolved
    );
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
        recent_documents: Vec::new(),
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
fn recursive_folder_metadata_and_bidirectional_sort_use_real_files() {
    let mut fixture = Fixture::new("aggregates");
    let folder = fixture.directory("Folder");
    fixture.directory("Folder\\Nested");
    let old = fixture.file("Folder\\old.txt", b"12");
    let latest = fixture.file("Folder\\Nested\\latest.txt", b"34567");
    fixture.file("standalone.txt", b"abc");
    fixture.directory("Empty");
    let before = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    let after = before + std::time::Duration::from_secs(10_000);
    fs::File::options()
        .write(true)
        .open(old)
        .unwrap()
        .set_modified(before)
        .unwrap();
    fs::File::options()
        .write(true)
        .open(latest)
        .unwrap()
        .set_modified(after)
        .unwrap();
    let scope = fixture.scope();
    let stats = files::directory_stats(&scope, &folder, || false).unwrap();
    assert_eq!(stats.size, 7);
    assert_eq!(stats.modified, Some(after));
    assert!(!stats.partial());
    assert!(
        files::directory_stats(&scope, &folder, || true)
            .unwrap_err()
            .contains("superseded")
    );
    let mut listing = files::list_directory(&scope, &fixture.root).unwrap();
    for entry in &mut listing.entries {
        if entry.directory {
            entry.aggregate = Some(files::directory_stats(&scope, &entry.path, || false));
        }
    }
    files::sort_entries_ordered(&mut listing.entries, Sort::Size, false);
    assert_eq!(
        listing
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Empty", "standalone.txt", "Folder"]
    );
    files::sort_entries_ordered(&mut listing.entries, Sort::Size, true);
    assert_eq!(listing.entries[0].name, "Folder");
    files::sort_entries_ordered(&mut listing.entries, Sort::Modified, false);
    assert_eq!(listing.entries[1].name, "Folder");
    files::sort_entries_ordered(&mut listing.entries, Sort::Modified, true);
    assert_eq!(listing.entries[1].name, "Folder");
}

#[test]
fn recent_documents_are_bounded_deduplicated_and_old_preferences_load() {
    let mut fixture = Fixture::new("recent");
    let settings = fixture.file(
        "preferences.txt",
        b"TomasCommander preferences v1\ndark=true\naccent=0\n",
    );
    let mut preferences = Preferences::load(&settings).unwrap();
    assert!(preferences.recent_documents.is_empty());
    assert!(!preferences.record_document_open(fixture.root.join("application.exe")));
    for index in 0..30 {
        assert!(
            preferences.record_document_open(fixture.root.join(format!("document-{index}.PPTX")))
        );
    }
    let newest = fixture.root.join("document-20.PPTX");
    preferences.record_document_open(newest.clone());
    assert_eq!(preferences.recent_documents.len(), Preferences::MAX_RECENT);
    assert_eq!(preferences.recent_documents[0], newest);
    assert_eq!(
        preferences
            .recent_documents
            .iter()
            .filter(|path| path == &&newest)
            .count(),
        1
    );
    preferences.save(&settings).unwrap();
    let loaded = Preferences::load(&settings).unwrap();
    assert_eq!(loaded.recent_documents, preferences.recent_documents);
    assert!(
        fs::read_to_string(settings)
            .unwrap()
            .starts_with("TomasCommander preferences v2\n")
    );
}

#[test]
fn folder_metadata_reports_actual_unreadable_descendants_as_partial() {
    let mut fixture = Fixture::new("aggregate-denied");
    fixture.file("readable.txt", b"visible");
    let denied = fixture.directory("Denied");
    let hidden = fixture.file("Denied\\hidden.txt", b"not counted");
    let sid = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value",
        ])
        .output()
        .unwrap();
    assert!(sid.status.success());
    let sid = String::from_utf8(sid.stdout).unwrap().trim().to_owned();
    assert!(
        Command::new("icacls.exe")
            .arg(&denied)
            .args(["/deny", &format!("*{sid}:(RD)"), "/Q"])
            .status()
            .unwrap()
            .success()
    );
    let unreadable = fs::read_dir(&denied).is_err();
    let stats = files::directory_stats(&fixture.scope(), &fixture.root, || false);
    assert!(
        Command::new("icacls.exe")
            .arg(&denied)
            .args(["/remove:d", &format!("*{sid}"), "/Q"])
            .status()
            .unwrap()
            .success(),
        "Synthetic ACL must be restored before assertions and cleanup."
    );
    assert!(unreadable, "The fixture must exercise real read denial.");
    let stats = stats.unwrap();
    assert!(stats.partial());
    assert_eq!(stats.size, 7);
    assert_eq!(stats.warnings.len(), 1);
    assert!(stats.warnings[0].contains("Read folder"));
    assert!(stats.warnings[0].contains("Denied"));
    assert!(!stats.truncated);
    assert_eq!(fs::read(hidden).unwrap(), b"not counted");
}

#[test]
fn copilot_resolution_requires_a_native_path_without_shell_interpolation() {
    let mut fixture = Fixture::new("cli-resolution");
    let folder = fixture.directory("Tools with spaces & percent%");
    let executable = fixture.file(
        "Tools with spaces & percent%\\copilot.exe",
        b"owned resolver sentinel; never executable",
    );
    assert_eq!(
        tomas_commander::platform::find_copilot([folder]).unwrap(),
        executable
    );
    assert!(tomas_commander::platform::find_copilot([PathBuf::from("relative tools")]).is_err());
    assert!(tomas_commander::platform::find_copilot([fixture.root.clone()]).is_err());
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
fn recursive_search_uses_real_relative_names_and_rejects_stale_results() {
    use tomas_commander::search;
    let mut fixture = Fixture::new("search");
    fixture.directory("One");
    fixture.directory("Two");
    let first = fixture.file("One\\Duplicate Česká.txt", b"first");
    fixture.file("Two\\Duplicate Česká.txt", b"second");
    fixture.file("other.txt", b"not a match");
    let scope = fixture.scope();
    let results = search::discover(&scope, &fixture.root, "DUPLICATE", || false).unwrap();
    assert_eq!(results.matches.len(), 2);
    assert_eq!(
        results.matches[0].relative,
        PathBuf::from("One\\Duplicate Česká.txt")
    );
    assert_eq!(
        results.matches[1].relative,
        PathBuf::from("Two\\Duplicate Česká.txt")
    );
    assert!(results.errors.is_empty());
    assert!(!results.truncated);
    assert_eq!(
        search::locate(&scope, &results.matches[0]).unwrap(),
        first.canonicalize().unwrap()
    );
    fs::write(&first, b"changed").unwrap();
    assert!(search::locate(&scope, &results.matches[0]).is_err());
    assert!(
        search::discover(&scope, &fixture.root, "no-such-name", || false)
            .unwrap()
            .matches
            .is_empty()
    );
    assert!(search::discover(&scope, &fixture.root, "", || false).is_err());
    assert!(
        search::discover(&scope, &fixture.root, "txt", || true)
            .unwrap()
            .cancelled
    );
    let counter = std::cell::Cell::new(0);
    let interrupted = search::discover(&scope, &fixture.root, "txt", || {
        counter.set(counter.get() + 1);
        counter.get() > 4
    })
    .unwrap();
    assert!(interrupted.cancelled);
    assert!(interrupted.examined > 0);
}

#[test]
fn search_skips_junctions_even_when_they_point_inside_scope() {
    let mut fixture = Fixture::new("search-junction");
    let target = fixture.directory("Target");
    fixture.file("Target\\unique.txt", b"one actual match");
    let link = fixture.root.join("Link");
    assert!(
        Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap()
            .status
            .success()
    );
    fixture.owned.push(link);
    let result =
        tomas_commander::search::discover(&fixture.scope(), &fixture.root, "unique", || false)
            .unwrap();
    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.skipped_links, 1);
    let stats = files::directory_stats(&fixture.scope(), &fixture.root, || false).unwrap();
    assert_eq!(stats.size, b"one actual match".len() as u64);
    assert_eq!(stats.skipped_links, 1);
    assert!(!stats.partial());
}

#[test]
fn replaced_source_with_same_bytes_and_times_invalidates_approval_and_search() {
    let mut fixture = Fixture::new("identity");
    let source = fixture.file("source.txt", b"same bytes");
    let replacement = fixture.file("replacement.txt", b"same bytes");
    let original = fs::File::open(&source).unwrap();
    let metadata = original.metadata().unwrap();
    fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(metadata.modified().unwrap())
                .set_accessed(metadata.accessed().unwrap()),
        )
        .unwrap();
    let destination = fixture.directory("Destination");
    let scope = fixture.scope();
    let plan = files::plan(
        &scope,
        20,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(&destination),
    )
    .unwrap();
    let results =
        tomas_commander::search::discover(&scope, &fixture.root, "source.txt", || false).unwrap();
    drop(original);
    fs::remove_file(&source).unwrap();
    fs::rename(&replacement, &source).unwrap();
    let result = files::execute(&scope, &plan, true, &AtomicBool::new(false));
    assert!(result.error.is_some());
    assert!(result.created.is_empty());
    assert!(tomas_commander::search::locate(&scope, &results.matches[0]).is_err());
    assert_eq!(fs::read(source).unwrap(), b"same bytes");
}

#[test]
fn partial_multi_item_failure_reports_exact_completed_and_incomplete_paths() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut fixture = Fixture::new("partial-items");
    let first = fixture.file("a.txt", b"complete");
    let second = fixture.file("z.txt", b"locked");
    let destination = fixture.directory("Destination");
    let scope = fixture.scope();
    let before = hash_inventory(&fixture.root);
    let plan = files::plan(
        &scope,
        21,
        Operation::Copy,
        &[first.clone(), second.clone()],
        Some(&destination),
    )
    .unwrap();
    // Attribute reads remain allowed, but opening file content is denied.
    let lock = fs::OpenOptions::new()
        .write(true)
        .share_mode(2 | 4)
        .open(&second)
        .unwrap();
    let result = files::execute(&scope, &plan, true, &AtomicBool::new(false));
    fixture.own(&result.created);
    assert!(result.error.is_some());
    assert_eq!(result.completed, vec![first.canonicalize().unwrap()]);
    assert_eq!(result.incomplete, vec![second.canonicalize().unwrap()]);
    assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"complete");
    assert!(!destination.join("z.txt").exists());
    drop(lock);
    assert_eq!(fs::read(second).unwrap(), b"locked");
    let after = hash_inventory(&fixture.root);
    assert_eq!(after["a.txt"], before["a.txt"]);
    assert_eq!(after["z.txt"], before["z.txt"]);
    assert_eq!(after["Destination\\a.txt"], before["a.txt"]);
    assert!(!after.contains_key("Destination\\z.txt"));
    assert!(
        files::plan(&scope, 22, Operation::Copy, &[first], Some(&destination))
            .unwrap_err()
            .contains("Conflict")
    );
}

#[test]
fn actual_acl_permission_failure_is_explicit_and_preserves_source() {
    let mut fixture = Fixture::new("acl");
    let source = fixture.file("source.txt", b"preserve");
    let destination = fixture.directory("Denied");
    let before = hash_inventory(&fixture.root);
    let sid = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value",
        ])
        .output()
        .unwrap();
    assert!(sid.status.success());
    let sid = String::from_utf8(sid.stdout).unwrap().trim().to_owned();
    let deny = format!("*{sid}:(WD)");
    let status = Command::new("icacls.exe")
        .arg(&destination)
        .args(["/deny", &deny, "/Q"])
        .status()
        .unwrap();
    assert!(status.success());
    let scope = fixture.scope();
    let result = files::plan(
        &scope,
        23,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(&destination),
    )
    .map(|plan| files::execute(&scope, &plan, true, &AtomicBool::new(false)));
    let restored = Command::new("icacls.exe")
        .arg(&destination)
        .args(["/remove:d", &format!("*{sid}"), "/Q"])
        .status()
        .unwrap();
    assert!(
        restored.success(),
        "Synthetic ACL must be restored before cleanup."
    );
    let outcome = result.unwrap();
    fixture.own(&outcome.created);
    assert!(
        outcome.error.is_some(),
        "Actual denied file creation must be visible."
    );
    assert!(outcome.completed.is_empty());
    assert!(outcome.created.is_empty());
    assert_eq!(fs::read(source).unwrap(), b"preserve");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    assert_eq!(hash_inventory(&fixture.root), before);
}

#[test]
fn search_limits_are_explicit_and_large_folder_baselines_are_measured() {
    let mut fixture = Fixture::new("search-baseline");
    for index in 0..10_020 {
        fixture.file(&format!("match-{index:05}.txt"), b"baseline");
    }
    let scope = fixture.scope();
    for trial in 1..=3 {
        let started = std::time::Instant::now();
        let listing = files::list_directory(&scope, &fixture.root).unwrap();
        let listing_ms = started.elapsed().as_secs_f64() * 1000.0;
        let started = std::time::Instant::now();
        let matches = listing
            .entries
            .iter()
            .filter(|entry| entry.folded.contains("019"))
            .count();
        let filter_ms = started.elapsed().as_secs_f64() * 1000.0;
        let started = std::time::Instant::now();
        let results =
            tomas_commander::search::discover(&scope, &fixture.root, "match", || false).unwrap();
        let search_ms = started.elapsed().as_secs_f64() * 1000.0;
        println!(
            "BASELINE warm trial {trial}: {} files, listing {listing_ms:.2} ms, filter {filter_ms:.2} ms ({matches} names), recursive search {search_ms:.2} ms; no acceptance thresholds.",
            listing.entries.len()
        );
        assert_eq!(listing.entries.len(), 10_020);
        assert_eq!(results.matches.len(), tomas_commander::search::MAX_RESULTS);
        assert!(results.truncated);
        assert!(!results.cancelled);
    }
    let destination = fixture.directory("Destination");
    let sources: Vec<_> = (0..20)
        .map(|index| fixture.root.join(format!("match-{index:05}.txt")))
        .collect();
    let started = std::time::Instant::now();
    let plan = files::plan(&scope, 26, Operation::Copy, &sources, Some(&destination)).unwrap();
    let planning_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = std::time::Instant::now();
    let result = files::execute(&scope, &plan, true, &AtomicBool::new(false));
    fixture.own(&result.created);
    assert!(result.error.is_none());
    for source in sources {
        assert_eq!(
            fs::read(destination.join(source.file_name().unwrap())).unwrap(),
            b"baseline"
        );
    }
    println!(
        "BASELINE 20 exclusive eight-byte copies: planning {planning_ms:.2} ms, validated execution plus sync/read-back {:.2} ms; not a throughput benchmark.",
        started.elapsed().as_secs_f64() * 1000.0
    );
}

#[test]
#[ignore = "Subprocess helper; invoked only by the scoped interruption test"]
fn interrupted_copy_worker() {
    let root =
        PathBuf::from(std::env::var_os("TC_INTERRUPTION_ROOT").expect("Owned test root required"));
    let scope = Scope::fixture(&root).unwrap();
    let plan = files::plan(
        &scope,
        24,
        Operation::Copy,
        &[root.join("large.bin")],
        Some(&root.join("Destination")),
    )
    .unwrap();
    let journal = tomas_commander::journal::Journal::new(root.join("Journal"));
    let result = journal.execute(&scope, &plan, true, &AtomicBool::new(false));
    assert!(result.error.is_none());
}

#[test]
fn actual_process_interruption_requires_readback_and_refuses_blind_retry() {
    let mut fixture = Fixture::new("process-interruption");
    let content = vec![0x37; 128 * 1024 * 1024];
    let source = fixture.file("large.bin", &content);
    drop(content);
    let destination = fixture.directory("Destination");
    let journal_path = fixture.directory("Journal");
    let before_source = hash_inventory(&fixture.root);
    let target = destination.join("large.bin");
    fixture.owned.push(target.clone());
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "interrupted_copy_worker"])
        .env("TC_INTERRUPTION_ROOT", &fixture.root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut observed_bytes = None;
    while std::time::Instant::now() < deadline {
        if let Ok(metadata) = fs::metadata(&target)
            && metadata.len() > 0
            && metadata.len() < 128 * 1024 * 1024
        {
            observed_bytes = Some(metadata.len());
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
    let output = child.wait_with_output().unwrap();
    let records: Vec<_> = fs::read_dir(&journal_path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    fixture.own(&records);
    assert!(
        observed_bytes.is_some(),
        "No partial copy observed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!output.status.success());
    let bytes = fs::read(&target).unwrap();
    assert!(!bytes.is_empty() && bytes.len() < 128 * 1024 * 1024);
    assert!(bytes.iter().all(|byte| *byte == 0x37));
    assert_eq!(fs::metadata(&source).unwrap().len(), 128 * 1024 * 1024);
    let scope = fixture.scope();
    let journal = tomas_commander::journal::Journal::new(journal_path);
    assert_eq!(journal.pending().unwrap().len(), 1);
    let alternate = fixture.directory("Alternate");
    let retry = files::plan(
        &scope,
        25,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(&alternate),
    )
    .unwrap();
    let retry = journal.execute(&scope, &retry, true, &AtomicBool::new(false));
    assert!(retry.error.unwrap().contains("Unresolved operation"));
    assert!(retry.created.is_empty());
    let after = hash_inventory(&fixture.root);
    assert_eq!(after["large.bin"], before_source["large.bin"]);
    assert!(after["Destination\\large.bin"].starts_with(&format!("FILE {} ", bytes.len())));
    assert!(
        files::plan(&scope, 25, Operation::Copy, &[source], Some(&destination))
            .unwrap_err()
            .contains("Conflict")
    );
    println!(
        "INTERRUPTION read-back: {} retained partial bytes; source intact; fresh retry conflicts rather than overwriting.",
        bytes.len()
    );
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
    let before_source = hash_inventory(&fixture.root);
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
    let after = hash_inventory(&fixture.root);
    assert_eq!(after["large.bin"], before_source["large.bin"]);
    assert!(
        after["Destination\\large.bin"]
            .starts_with(&format!("FILE {} ", fs::metadata(&target).unwrap().len()))
    );
    assert!(fs::read(target).unwrap().iter().all(|byte| *byte == 0x5a));
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
    let before = hash_inventory(&fixture.root);
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
    assert!(hash_inventory(&fixture.root).is_empty());
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
    assert_eq!(hash_inventory(&fixture.root), before);
}

#[test]
fn complete_recursive_multi_item_hash_matrix_preserves_unapproved_state() {
    let mut fixture = Fixture::new("hash-matrix");
    let source = fixture.directory("Source");
    let alpha = fixture.directory("Source\\Alpha");
    fixture.directory("Source\\Alpha\\Nested");
    fixture.directory("Source\\Alpha\\Empty");
    fixture.file("Source\\Alpha\\Nested\\duplicate.txt", b"nested duplicate");
    fixture.file("Source\\Alpha\\duplicate.txt", b"top duplicate");
    fixture.file(
        "Source\\Alpha\\Česká s mezerou.txt",
        "Skutečný obsah".as_bytes(),
    );
    let loose = fixture.file("Source\\notes.md", b"# owned markdown\n");
    let destination = fixture.directory("Destination");
    let moved = fixture.directory("Moved");
    let sentinel = fixture.file("unapproved-sentinel.txt", b"never mutate");
    let before = hash_inventory(&fixture.root);
    let source_hashes = hash_inventory(&source);
    let scope = fixture.scope();
    let plan = files::plan(
        &scope,
        101,
        Operation::Copy,
        &[alpha.clone(), loose.clone()],
        Some(&destination),
    )
    .unwrap();
    for (approved, cancelled) in [(false, false), (true, true)] {
        let outcome = files::execute(&scope, &plan, approved, &AtomicBool::new(cancelled));
        assert!(outcome.created.is_empty());
        assert_eq!(hash_inventory(&fixture.root), before);
    }
    let outcome = files::execute(&scope, &plan, true, &AtomicBool::new(false));
    fixture.own(&outcome.created);
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert!(outcome.incomplete.is_empty());
    assert_eq!(hash_inventory(&source), source_hashes);
    assert_eq!(hash_inventory(&destination), source_hashes);
    let after_copy = hash_inventory(&fixture.root);
    assert!(
        files::plan(
            &scope,
            102,
            Operation::Copy,
            &[alpha, loose],
            Some(&destination)
        )
        .unwrap_err()
        .contains("Conflict")
    );
    assert_eq!(hash_inventory(&fixture.root), after_copy);
    let plan = files::plan(
        &scope,
        103,
        Operation::Move,
        &[destination.join("Alpha"), destination.join("notes.md")],
        Some(&moved),
    )
    .unwrap();
    let denied = files::execute(&scope, &plan, false, &AtomicBool::new(false));
    assert!(denied.error.is_some());
    assert_eq!(hash_inventory(&fixture.root), after_copy);
    let outcome = files::execute(&scope, &plan, true, &AtomicBool::new(false));
    fixture.own(&outcome.created);
    fixture.own(&[
        moved.join("Alpha\\Nested"),
        moved.join("Alpha\\Empty"),
        moved.join("Alpha\\Nested\\duplicate.txt"),
        moved.join("Alpha\\duplicate.txt"),
        moved.join("Alpha\\Česká s mezerou.txt"),
    ]);
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(outcome.completed.len(), 2);
    assert!(outcome.incomplete.is_empty());
    assert!(hash_inventory(&destination).is_empty());
    assert_eq!(hash_inventory(&moved), source_hashes);
    assert_eq!(hash_inventory(&source), source_hashes);
    assert_eq!(fs::read(sentinel).unwrap(), b"never mutate");
}

#[test]
fn journal_initialization_before_planning_preserves_destination_stale_checks() {
    let mut fixture = Fixture::new("journal-parent");
    fixture.directory("Inputs");
    let source = fixture.file("Inputs\\duplicate.txt", b"owned native copy");
    let second = fixture.file("Inputs\\second.txt", b"do not copy stale plan");
    let journal_path = fixture.root.join("preferences.operations");
    fixture.owned.push(journal_path.clone());
    let journal = tomas_commander::journal::Journal::new(journal_path.clone());
    let scope = fixture.scope();
    journal.initialize().unwrap();
    let plan = files::plan(
        &scope,
        109,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(&fixture.root),
    )
    .unwrap();
    let before = hash_inventory(&fixture.root.join("Inputs"));
    let outcome = journal.execute(&scope, &plan, true, &AtomicBool::new(false));
    fixture.own(&outcome.created);
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(hash_inventory(&fixture.root.join("Inputs")), before);
    assert_eq!(
        hash_inventory(&fixture.root)["duplicate.txt"],
        before["duplicate.txt"]
    );
    assert!(journal.pending().unwrap().is_empty());

    let stale = files::plan(
        &scope,
        110,
        Operation::Copy,
        std::slice::from_ref(&second),
        Some(&fixture.root),
    )
    .unwrap();
    fixture.file("unapproved.txt", b"preserve destination change");
    let refused = journal.execute(&scope, &stale, true, &AtomicBool::new(false));
    assert!(
        refused
            .error
            .unwrap()
            .contains("Destination folder changed")
    );
    assert!(refused.created.is_empty());
    assert!(!fixture.root.join("second.txt").exists());
    assert_eq!(hash_inventory(&fixture.root.join("Inputs")), before);
    fixture.own(
        &fs::read_dir(journal_path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>(),
    );
}

#[test]
fn durable_claim_refuses_replay_after_restart_and_records_cancelled_readback() {
    let mut fixture = Fixture::new("journal");
    let source = fixture.file("Česká source.txt", b"durable source");
    let destination = fixture.directory("Destination");
    let journal_path = fixture.directory("Journal");
    let journal = tomas_commander::journal::Journal::new(journal_path.clone());
    let scope = fixture.scope();
    let plan = files::plan(
        &scope,
        110,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(&destination),
    )
    .unwrap();
    let outcome = journal.execute(&scope, &plan, true, &AtomicBool::new(false));
    fixture.own(&outcome.created);
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert!(journal.pending().unwrap().is_empty());
    fs::remove_file(destination.join("Česká source.txt")).unwrap();
    let restarted = tomas_commander::journal::Journal::new(journal_path.clone());
    let replay = restarted.execute(&scope, &plan.clone(), true, &AtomicBool::new(false));
    assert!(replay.error.unwrap().contains("Claim operation once"));
    assert!(replay.created.is_empty());
    let fresh = files::plan(
        &scope,
        111,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(&destination),
    )
    .unwrap();
    let cancelled = restarted.execute(&scope, &fresh, true, &AtomicBool::new(true));
    assert!(cancelled.cancelled);
    assert!(restarted.pending().unwrap().is_empty());
    let alternative = fixture.directory("Alternative");
    let retry = files::plan(
        &scope,
        112,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(&alternative),
    )
    .unwrap();
    let fresh_approval = restarted.execute(&scope, &retry, true, &AtomicBool::new(false));
    fixture.own(&fresh_approval.created);
    assert!(fresh_approval.error.is_none(), "{:?}", fresh_approval.error);
    assert_eq!(fs::read(source).unwrap(), b"durable source");
    let records = fs::read_dir(journal_path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    fixture.own(&records);
}

#[test]
fn copy_guards_reject_actual_parent_replacement_and_content_write_races() {
    use std::{
        os::windows::fs::OpenOptionsExt,
        sync::{Arc, atomic::Ordering},
    };
    let mut fixture = Fixture::new("copy-race");
    let folder = fixture.directory("Source");
    let source = fixture.file("Source\\large.bin", &vec![0x68; 128 * 1024 * 1024]);
    let destination = fixture.directory("Destination");
    let target = destination.join("large.bin");
    fixture.owned.push(target.clone());
    let source_before = hash_inventory(&folder);
    let scope = fixture.scope();
    let plan = files::plan(
        &scope,
        120,
        Operation::Copy,
        std::slice::from_ref(&source),
        Some(&destination),
    )
    .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let worker = std::thread::spawn(move || files::execute(&scope, &plan, true, &worker_cancel));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !worker.is_finished() && std::time::Instant::now() < deadline {
        if fs::metadata(&target).is_ok_and(|metadata| metadata.len() > 0) {
            break;
        }
        std::thread::yield_now();
    }
    let writer = fs::OpenOptions::new()
        .write(true)
        .share_mode(7)
        .open(&source);
    let renamed_source = fixture.root.join("RenamedSource");
    let renamed_destination = fixture.root.join("RenamedDestination");
    let source_rename = fs::rename(&folder, &renamed_source);
    let destination_rename = fs::rename(&destination, &renamed_destination);
    cancel.store(true, Ordering::Relaxed);
    let result = worker.join().unwrap();
    fixture.own(&[
        renamed_source.clone(),
        renamed_destination.clone(),
        renamed_source.join("large.bin"),
        renamed_destination.join("large.bin"),
    ]);
    assert!(
        writer.is_err(),
        "Content write must be denied while the approved source is being copied."
    );
    assert!(
        source_rename.is_err(),
        "Source ancestors must not be replaced during copying."
    );
    assert!(
        destination_rename.is_err(),
        "Destination ancestors must not be replaced during copying."
    );
    assert!(
        result.cancelled,
        "The adversarial attempts must occur while copying, not after completion."
    );
    assert_eq!(hash_inventory(&folder), source_before);
    assert!(fs::read(target).unwrap().iter().all(|byte| *byte == 0x68));
    println!(
        "RACE GUARDS: source writes and both ancestor renames rejected during live copy; source SHA256 unchanged."
    );
}

#[test]
fn corrupted_durable_record_fails_closed_without_mutating_sources_or_targets() {
    let mut fixture = Fixture::new("journal-corrupt");
    let source = fixture.file("source.txt", b"preserve");
    let destination = fixture.directory("Destination");
    let journal_path = fixture.directory("Journal");
    fixture.file(
        "Journal\\unknown.operation",
        b"interrupted malformed record",
    );
    let journal = tomas_commander::journal::Journal::new(journal_path.clone());
    let scope = fixture.scope();
    let plan = files::plan(&scope, 121, Operation::Copy, &[source], Some(&destination)).unwrap();
    let before = hash_inventory(&fixture.root);
    assert!(journal.pending().unwrap_err().contains("Damaged"));
    let result = journal.execute(&scope, &plan, true, &AtomicBool::new(false));
    assert!(result.error.unwrap().contains("Damaged"));
    fixture.own(&[journal_path.join("journal.lock")]);
    let after = hash_inventory(&fixture.root);
    assert_eq!(after["source.txt"], before["source.txt"]);
    assert!(hash_inventory(&destination).is_empty());
}

#[test]
fn handle_move_refuses_replaced_source_and_destination_identities() {
    let mut fixture = Fixture::new("handle-move-stale");
    let source = fixture.file("source.txt", b"same");
    let other = fixture.file("replacement.txt", b"same");
    let destination = fixture.directory("Destination");
    let source_identity = files::fingerprint(&source).unwrap();
    let destination_id = tomas_commander::platform::path_identity(&destination).unwrap();
    fs::remove_file(&source).unwrap();
    fs::rename(&other, &source).unwrap();
    let before = hash_inventory(&fixture.root);
    let error = tomas_commander::platform::move_verified(
        &source,
        &destination.join("source.txt"),
        &source_identity,
        destination_id,
    )
    .unwrap_err();
    assert!(error.contains("identity changed"));
    assert_eq!(hash_inventory(&fixture.root), before);
    let alternate = fixture.directory("Alternate");
    let wrong_parent = tomas_commander::platform::path_identity(&alternate).unwrap();
    let actual_source = files::fingerprint(&source).unwrap();
    let error = tomas_commander::platform::move_verified(
        &source,
        &destination.join("source.txt"),
        &actual_source,
        wrong_parent,
    )
    .unwrap_err();
    assert!(error.contains("identity changed"));
    assert!(source.exists());
    assert!(hash_inventory(&destination).is_empty());
}

#[test]
fn handle_move_refuses_changed_content_even_when_source_identity_is_unchanged() {
    let mut fixture = Fixture::new("handle-move-content");
    let source = fixture.file("source.txt", b"approved bytes");
    let destination = fixture.directory("Destination");
    let approved = files::fingerprint(&source).unwrap();
    let source_id = tomas_commander::platform::path_identity(&source).unwrap();
    let destination_id = tomas_commander::platform::path_identity(&destination).unwrap();
    fs::write(
        &source,
        b"changed after validation, before opening the move handle",
    )
    .unwrap();
    assert_eq!(
        tomas_commander::platform::path_identity(&source).unwrap(),
        source_id
    );
    let before = hash_inventory(&fixture.root);
    let error = tomas_commander::platform::move_verified(
        &source,
        &destination.join("source.txt"),
        &approved,
        destination_id,
    )
    .unwrap_err();
    assert!(error.contains("source changed"));
    assert_eq!(hash_inventory(&fixture.root), before);
}
