use super::*;

#[test]
fn directory_refresh_keeps_size_and_date_until_replacement_arrives() {
    let root = PathBuf::from("fixture");
    let mut pane = Pane::new(root.clone());
    let mut folder = entry("folder");
    folder.directory = true;
    let old_time = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    folder.aggregate = Some(Ok(files::DirectoryStats {
        size: 123,
        modified: Some(old_time),
        ..Default::default()
    }));
    pane.entries = vec![folder.clone()];
    let mut fresh_listing = folder.clone();
    fresh_listing.aggregate = None;
    for _ in 0..3 {
        pane.begin_navigation(root.clone());
        pane.replace_entries(vec![fresh_listing.clone()]);
        let retained = pane.entries[0]
            .aggregate
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(retained.size, 123);
        assert_eq!(retained.modified, Some(old_time));
        assert!(pane.pending_aggregates.contains(&folder.path));
    }
    let new_time = old_time + Duration::from_secs(100);
    assert!(pane.apply_aggregate(
        &folder.path,
        Ok(files::DirectoryStats {
            size: 456,
            modified: Some(new_time),
            ..Default::default()
        })
    ));
    let updated = pane.entries[0]
        .aggregate
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(updated.size, 456);
    assert_eq!(updated.modified, Some(new_time));
    assert!(pane.pending_aggregates.is_empty());
    pane.replace_entries(vec![fresh_listing]);
    assert!(pane.apply_aggregate(&folder.path, Err("Read denied".into())));
    assert_eq!(
        pane.entries[0]
            .aggregate
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err(),
        "Read denied"
    );
    assert!(pane.pending_aggregates.is_empty());
}

#[test]
fn directory_refresh_does_not_retain_removed_or_type_changed_snapshots() {
    let mut pane = Pane::new(PathBuf::from("fixture"));
    let mut folder = entry("folder");
    folder.directory = true;
    folder.aggregate = Some(Ok(files::DirectoryStats {
        size: 123,
        ..Default::default()
    }));
    pane.entries = vec![folder.clone()];
    pane.replace_entries(vec![entry("folder")]);
    assert!(pane.entries[0].aggregate.is_none());
    assert!(pane.pending_aggregates.is_empty());
    pane.entries = vec![folder.clone()];
    let mut link = folder.clone();
    link.link = true;
    link.aggregate = Some(Err("Link folders are not traversed.".into()));
    pane.replace_entries(vec![link]);
    assert!(pane.entries[0].aggregate.as_ref().unwrap().is_err());
    assert!(pane.pending_aggregates.is_empty());
    pane.entries = vec![folder.clone()];
    let mut new_folder = entry("new-folder");
    new_folder.directory = true;
    pane.replace_entries(vec![new_folder.clone()]);
    assert!(pane.entries[0].aggregate.is_none());
    assert_eq!(pane.pending_aggregates, BTreeSet::from([new_folder.path]));
    assert!(!pane.apply_aggregate(&folder.path, Ok(files::DirectoryStats::default())));
    pane.begin_navigation(PathBuf::from("other"));
    assert!(pane.entries.is_empty());
    assert!(pane.pending_aggregates.is_empty());
}

#[test]
fn ten_row_jumps_clamp_and_preserve_selection() {
    let mut pane = Pane::new(PathBuf::from("fixture"));
    pane.entries = (0..25)
        .map(|row| entry(&format!("file-{row:02}")))
        .collect();
    pane.rebuild();
    pane.selected.insert(PathBuf::from("file-03"));
    pane.jump(true);
    assert_eq!(pane.focus_position(), 10);
    pane.jump(true);
    assert_eq!(pane.focus_position(), 20);
    pane.jump(true);
    assert_eq!(pane.focus_position(), 25);
    pane.jump(false);
    assert_eq!(pane.focus_position(), 15);
    pane.jump(false);
    pane.jump(false);
    assert_eq!(pane.focus_position(), 0);
    assert_eq!(pane.targets(), vec![PathBuf::from("file-03")]);
    pane.filter = "missing".into();
    pane.rebuild();
    pane.jump(true);
    assert_eq!(pane.focus_position(), 0);
    assert!(pane.scroll_to_focus);
}

#[test]
fn parent_returns_are_per_pane_bounded_and_invalidated_only_by_new_navigation() {
    let mut left = Pane::new(PathBuf::from(r"C:\root\child\grandchild"));
    let right = Pane::new(PathBuf::from(r"C:\other"));
    left.filter = "alpha".into();
    left.focused = Some(left.path.join("alpha.txt"));
    left.scroll_offset = 42.0;
    let history = left.remember_parent_return();
    left.begin_navigation(PathBuf::from(r"C:\root\child"));
    left.parent_returns = history;
    let history = left.remember_parent_return();
    left.begin_navigation(PathBuf::from(r"C:\root"));
    left.parent_returns = history;
    assert_eq!(left.parent_returns.len(), 2);
    assert!(right.parent_returns.is_empty());
    left.begin_navigation(left.path.clone());
    assert_eq!(left.parent_returns.len(), 2);
    let child = left.parent_returns.pop().unwrap();
    assert_eq!(child.path, PathBuf::from(r"C:\root\child"));
    let grandchild = left.parent_returns.pop().unwrap();
    assert_eq!(grandchild.filter, "alpha");
    assert_eq!(grandchild.scroll_offset, 42.0);
    assert_eq!(grandchild.focused, Some(grandchild.path.join("alpha.txt")));
    for _ in 0..70 {
        left.parent_returns = left.remember_parent_return();
    }
    assert_eq!(left.parent_returns.len(), 64);
    left.begin_navigation(PathBuf::from(r"C:\unrelated"));
    assert!(left.parent_returns.is_empty());
}

#[test]
fn filename_filter_ignores_czech_diacritics_without_changing_names_or_sort_keys() {
    let mut pane = Pane::new(PathBuf::from("fixture"));
    pane.entries = vec![
        entry("ČEZ.txt"),
        entry("C\u{030c}EZ.pdf"),
        entry("other.txt"),
    ];
    pane.selected.insert(PathBuf::from("other.txt"));
    for query in ["cez", "CEZ", "čez", "ČEZ", "c\u{030c}ez"] {
        pane.filter = query.into();
        pane.rebuild();
        assert_eq!(pane.visible, vec![0, 1]);
        assert!(pane.selected.contains(&PathBuf::from("other.txt")));
        assert_eq!(pane.entries[0].name, "ČEZ.txt");
        assert_eq!(pane.entries[0].folded, "čez.txt");
    }
}

#[test]
fn filtering_preserves_path_selection_and_resets_range_anchor() {
    let mut pane = Pane::new(PathBuf::from("fixture"));
    pane.entries = vec![entry("Alpha"), entry("Beta"), entry("Gamma")];
    pane.rebuild();
    pane.focused = Some(PathBuf::from("Gamma"));
    pane.anchor = 2;
    pane.selected.insert(PathBuf::from("Beta"));
    pane.filter = "ALPHA".into();
    pane.rebuild();
    assert_eq!(pane.focused, Some(PathBuf::from("Alpha")));
    assert_eq!(pane.anchor, 1);
    assert_eq!(pane.targets(), vec![PathBuf::from("Beta")]);
    pane.filter = "missing".into();
    pane.rebuild();
    assert!(pane.focused.is_none());
    assert!(pane.visible.is_empty());
}

#[test]
fn parent_row_remains_first_and_never_becomes_an_operation_target() {
    let mut pane = Pane::new(PathBuf::from("fixture"));
    pane.rebuild();
    assert_eq!(pane.focus_position(), 0);
    assert_eq!(pane.row_path(0), None);
    assert!(pane.targets().is_empty());
    pane.entries = vec![entry("Zulu"), entry("Alpha")];
    pane.rebuild();
    files::sort_entries(&mut pane.entries, Sort::Size);
    pane.focused = pane.row_path(0);
    assert!(pane.targets().is_empty());
    pane.filter = "absent".into();
    pane.rebuild();
    assert_eq!(pane.focus_position(), 0);
    assert!(pane.targets().is_empty());
    pane.filter.clear();
    pane.rebuild();
    let range: Vec<_> = (0..=2)
        .filter_map(|position| pane.row_path(position))
        .collect();
    assert_eq!(range.len(), 2);
    assert!(!range.contains(&pane.path));
}

#[test]
fn sort_cycle_uses_exact_three_modes_and_preserves_file_state() {
    let mut pane = Pane::new(PathBuf::from("fixture"));
    let mut alpha = entry("Alpha");
    alpha.size = 30;
    alpha.modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(10));
    let mut bravo = entry("Bravo");
    bravo.size = 10;
    bravo.modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(30));
    let mut zulu = entry("Zulu");
    zulu.size = 20;
    zulu.modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(20));
    pane.entries = vec![alpha, bravo, zulu];
    pane.filter = "a".into();
    pane.focused = Some(PathBuf::from("Bravo"));
    pane.selected.insert(PathBuf::from("Alpha"));
    pane.sort = Sort::Size;
    for _ in 0..2 {
        for (sort, descending, names) in [
            (Sort::Name, false, vec!["Alpha", "Bravo", "Zulu"]),
            (Sort::Modified, true, vec!["Bravo", "Zulu", "Alpha"]),
            (Sort::Size, true, vec!["Alpha", "Zulu", "Bravo"]),
        ] {
            pane.cycle_sort();
            assert_eq!(pane.sort, sort);
            assert_eq!(pane.descending, descending);
            assert_eq!(
                pane.entries
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>(),
                names
            );
            assert_eq!(pane.focused, Some(PathBuf::from("Bravo")));
            assert_eq!(pane.selected, BTreeSet::from([PathBuf::from("Alpha")]));
            assert_eq!(pane.filter, "a");
            assert_eq!(pane.anchor, pane.focus_position());
            assert!(pane.scroll_to_focus);
        }
    }
    for (sort, expected, descending) in [
        (Sort::Name, Sort::Modified, true),
        (Sort::Modified, Sort::Size, true),
        (Sort::Size, Sort::Name, false),
    ] {
        pane.sort = sort;
        pane.descending = false;
        pane.cycle_sort();
        assert_eq!((pane.sort, pane.descending), (expected, descending));
    }
}

#[test]
fn sorting_retains_focused_item_identity() {
    let mut pane = Pane::new(PathBuf::from("fixture"));
    pane.entries = vec![entry("Zulu"), entry("Alpha")];
    pane.rebuild();
    pane.focused = Some(PathBuf::from("Zulu"));
    files::sort_entries(&mut pane.entries, Sort::Name);
    pane.rebuild();
    assert_eq!(pane.focus_position(), 2);
    assert_eq!(pane.targets(), vec![PathBuf::from("Zulu")]);
}
