use super::*;

#[test]
fn palette_slash_and_description_have_distinct_graphic_styles() {
    let ctx = egui::Context::default();
    ctx.run_ui(egui::RawInput::default(), |ui| {
        let label = palette_label(ui, Command::Rename, "Rename focused item", Color32::BLUE);
        assert_eq!(label.text, " /rename Rename focused item");
        assert_eq!(label.sections.len(), 2);
        assert_eq!(
            label.sections[0].format.font_id.family,
            egui::FontFamily::Monospace
        );
        assert_ne!(
            label.sections[0].format.color,
            label.sections[1].format.color
        );
        assert_ne!(label.sections[0].format.background, Color32::TRANSPARENT);
        assert!(label.sections[1].leading_space > 0.0);
    })
    .textures_delta
    .clear();
}

#[test]
fn slash_commands_are_unique_and_filter_by_name_or_slash() {
    let mut slashes = BTreeSet::new();
    for (command, name, _) in COMMANDS {
        assert!(slashes.insert(command.slash()));
        assert!(palette_matches(command.slash(), command, name));
        assert_eq!(
            palette_submission(command.slash(), None).unwrap(),
            (command, String::new()),
        );
    }
    assert!(palette_matches(
        "rename",
        Command::Rename,
        "Rename focused file"
    ));
    assert!(palette_matches(
        "/ren",
        Command::Rename,
        "Rename focused file"
    ));
    assert!(!palette_matches("/ren", Command::Refresh, "Refresh folder"));
    assert!(palette_matches(
        "/ai-search holiday photos",
        Command::AiSearch,
        "AI file search"
    ));
}

#[test]
fn palette_routes_prompts_and_rejects_unknown_commands_or_effect_arguments() {
    assert_eq!(
        palette_submission("/ai-search images from\nSrbsko", Some(Command::AiSearch)).unwrap(),
        (Command::AiSearch, "images from\nSrbsko".into()),
    );
    assert_eq!(
        palette_submission("images from Srbsko", None).unwrap(),
        (Command::AiSearch, "images from Srbsko".into()),
    );
    assert_eq!(
        palette_submission("/search alpha", Some(Command::Search)).unwrap(),
        (Command::Search, "alpha".into()),
    );
    assert_eq!(
        palette_submission("rename", Some(Command::Rename)).unwrap(),
        (Command::AiSearch, "rename".into()),
    );
    assert!(palette_submission("/missing prompt", None).is_err());
    assert!(palette_submission("/rename alpha.txt", Some(Command::Rename)).is_err());
    assert!(palette_submission("/delete\nanything", Some(Command::Recycle)).is_err());
    assert!(palette_submission("", None).is_err());
}

#[test]
fn palette_matching_is_offline_bounded_and_measured() {
    for trial in 0..3 {
        let started = Instant::now();
        let query = "natural-language".to_lowercase();
        let matches: Vec<_> = COMMANDS
            .iter()
            .filter(|(command, name, _)| palette_matches(&query, *command, name))
            .collect();
        assert_eq!(matches.len(), 1);
        assert!(matches!(matches[0].0, Command::AiPlan));
        println!(
            "CMD-01 PALETTE matching/typed dispatch selection trial {trial}: {} us",
            started.elapsed().as_micros()
        );
    }
    assert!(
        !COMMANDS
            .iter()
            .any(|(_, name, _)| name.contains("nonexistent command"))
    );
}
