use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Clone,
    Rename,
    NewFolder,
    OpenWith,
    Settings,
    ClearSelection,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    CreateZip,
    EditSource,
    SaveSource,
    RotateLeft,
    RotateRight,
    Preview,
    Parent,
    ReturnInside,
    JumpUp,
    JumpDown,
    Places,
    Help,
    Copy,
    Move,
    Recycle,
    Undo,
    Refresh,
    Favorites,
    Recent,
    Pin,
    Sort,
    Filter,
    Search,
    AiSearch,
    AiPlan,
    Balance,
    Code,
    CopilotCli,
    CopilotApp,
    Activity,
    Theme,
    Accent,
    Foundry,
    Answers,
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    Back,
    Forward,
    Usage,
}

impl Command {
    pub(super) fn slash(self) -> &'static str {
        match self {
            Self::Clone => "/clone",
            Self::Rename => "/rename",
            Self::NewFolder => "/new-folder",
            Self::OpenWith => "/open-with",
            Self::Settings => "/settings",
            Self::ClearSelection => "/clear-selection",
            Self::ZoomIn => "/zoom-in",
            Self::ZoomOut => "/zoom-out",
            Self::ZoomReset => "/zoom-reset",
            Self::CreateZip => "/zip",
            Self::EditSource => "/edit",
            Self::SaveSource => "/save",
            Self::RotateLeft => "/rotate-left",
            Self::RotateRight => "/rotate-right",
            Self::Preview => "/preview",
            Self::Parent => "/parent",
            Self::ReturnInside => "/return",
            Self::JumpUp => "/jump-up",
            Self::JumpDown => "/jump-down",
            Self::Places => "/places",
            Self::Help => "/help",
            Self::Copy => "/copy",
            Self::Move => "/move",
            Self::Recycle => "/delete",
            Self::Undo => "/undo",
            Self::Refresh => "/refresh",
            Self::Favorites => "/favorites",
            Self::Recent => "/recent",
            Self::Pin => "/pin",
            Self::Sort => "/sort",
            Self::Filter => "/filter",
            Self::Search => "/search",
            Self::AiSearch => "/ai-search",
            Self::AiPlan => "/file-command",
            Self::Balance => "/balance",
            Self::Code => "/code",
            Self::CopilotCli => "/copilot-cli",
            Self::CopilotApp => "/copilot-app",
            Self::Activity => "/operations",
            Self::Theme => "/theme",
            Self::Accent => "/accent",
            Self::Foundry => "/foundry",
            Self::Answers => "/answers",
            Self::NewTab => "/new-tab",
            Self::CloseTab => "/close-tab",
            Self::NextTab => "/next-tab",
            Self::PreviousTab => "/previous-tab",
            Self::Back => "/back",
            Self::Forward => "/forward",
            Self::Usage => "/usage",
        }
    }
}

pub(super) fn palette_matches(query: &str, command: Command, name: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.starts_with('/') {
        let token = query.split_whitespace().next().unwrap_or("/");
        command.slash().starts_with(token)
    } else {
        name.to_lowercase().contains(&query) || command.slash().contains(&query)
    }
}

pub(super) fn palette_label(
    ui: &egui::Ui,
    command: Command,
    name: &str,
    accent: Color32,
) -> egui::text::LayoutJob {
    let mut label = egui::text::LayoutJob::default();
    label.append(
        &format!(" {} ", command.slash()),
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::monospace(14.0),
            color: accent,
            background: tint(ui.visuals().extreme_bg_color, accent, 24),
            ..Default::default()
        },
    );
    label.append(
        name,
        14.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(12.0),
            color: ui.visuals().weak_text_color(),
            ..Default::default()
        },
    );
    label
}

pub(super) fn palette_submission(
    query: &str,
    selected: Option<Command>,
) -> Result<(Command, String), String> {
    let query = query.trim();
    if query.starts_with('/') {
        let token = query.split_whitespace().next().unwrap_or("/");
        let command = COMMANDS
            .iter()
            .find(|(command, _, _)| command.slash().eq_ignore_ascii_case(token))
            .map(|(command, _, _)| *command)
            .or_else(|| {
                (!query.contains(char::is_whitespace))
                    .then_some(selected)
                    .flatten()
            })
            .ok_or_else(|| "Unknown slash command.".to_owned())?;
        let argument = query.get(token.len()..).unwrap_or("").trim();
        if !argument.is_empty() && !matches!(command, Command::Search | Command::AiSearch) {
            return Err(format!("{} takes no inline arguments.", command.slash()));
        }
        return Ok((command, argument.to_owned()));
    }
    if !query.is_empty() {
        Ok((Command::AiSearch, query.to_owned()))
    } else {
        Err("Enter a command or search prompt.".into())
    }
}

pub(super) const COMMANDS: [(Command, &str, &str); 50] = [
    (
        Command::Clone,
        "Clone focused file in the same folder",
        "Ctrl Shift K",
    ),
    (
        Command::Rename,
        "Rename focused file / folder",
        "Ctrl Shift E",
    ),
    (
        Command::NewFolder,
        "New folder in active pane",
        "Ctrl Shift N",
    ),
    (
        Command::OpenWith,
        "Open with / development workspace",
        "Shift Enter",
    ),
    (
        Command::Settings,
        "Settings / appearance and connections",
        "",
    ),
    (
        Command::ClearSelection,
        "Clear selection in active pane",
        "",
    ),
    (
        Command::ZoomIn,
        "Zoom in / entire UI",
        "Ctrl + / Ctrl = / Ctrl wheel",
    ),
    (
        Command::ZoomOut,
        "Zoom out / entire UI",
        "Ctrl - / Ctrl wheel",
    ),
    (Command::ZoomReset, "Reset UI zoom to 100%", "Ctrl 0"),
    (
        Command::Preview,
        "Internal file preview / reading pane",
        "Ctrl Enter",
    ),
    (Command::Parent, "Parent folder", "Ctrl Left / Backspace"),
    (
        Command::ReturnInside,
        "Undo parent navigation",
        "Ctrl Right",
    ),
    (
        Command::Back,
        "Back / previous folder in this tab",
        "Alt Left / Mouse back",
    ),
    (
        Command::Forward,
        "Forward / next folder in this tab",
        "Alt Right / Mouse forward",
    ),
    (Command::NewTab, "New tab / current folder", "Ctrl T"),
    (Command::CloseTab, "Close tab", "Ctrl W"),
    (Command::NextTab, "Next tab", "Ctrl Tab"),
    (Command::PreviousTab, "Previous tab", "Ctrl Shift Tab"),
    (Command::JumpUp, "Jump up ten rows", "Page Up / Ctrl Up"),
    (
        Command::JumpDown,
        "Jump down ten rows",
        "Page Down / Ctrl Down",
    ),
    (Command::Places, "Choose saved place", "Alt Up"),
    (Command::Help, "Keyboard shortcuts / Help", "?"),
    (Command::Answers, "Connected answers", ""),
    (Command::AiPlan, "Natural-language file command / CZ EN", ""),
    (Command::AiSearch, "AI file search", ""),
    (Command::Foundry, "Foundry connection", ""),
    (Command::Copy, "Copy to opposite pane", "Ctrl Shift C"),
    (Command::Move, "Move to opposite pane", "Ctrl Shift M"),
    (Command::Recycle, "Delete selected items", "Ctrl Shift D"),
    (
        Command::Undo,
        "Undo an operation / reviewed inverse plan",
        "Ctrl Shift U",
    ),
    (Command::Refresh, "Refresh folder", "Ctrl Shift R"),
    (Command::Favorites, "Show pinned locations", ""),
    (Command::Recent, "Toggle workspace / Recent", "Ctrl Shift H"),
    (Command::Pin, "Pin current folder", ""),
    (
        Command::Sort,
        "Cycle sorting: A-Z / newest / largest",
        "Ctrl Shift S",
    ),
    (Command::Filter, "Filter active folder", "Ctrl F"),
    (
        Command::Search,
        "Search filenames recursively",
        "Ctrl Shift F",
    ),
    (Command::Balance, "Balance file panes", ""),
    (Command::Code, "Run VS Code", ""),
    (Command::CopilotCli, "Run GitHub Copilot CLI", ""),
    (Command::CopilotApp, "Run GitHub Copilot App", ""),
    (
        Command::Activity,
        "Show activity and exact operation results",
        "",
    ),
    (Command::Theme, "Toggle light and dark theme", ""),
    (
        Command::Usage,
        "Usage insights / local usage log report",
        "",
    ),
    (Command::Accent, "Cycle accent color", ""),
    (
        Command::CreateZip,
        "Create ZIP / extract ZIP by context",
        "Ctrl Shift A",
    ),
    (
        Command::EditSource,
        "Edit original text / Markdown source",
        "Reader Ctrl E",
    ),
    (
        Command::SaveSource,
        "Save edited source / exact approval",
        "Reader Ctrl S",
    ),
    (
        Command::RotateLeft,
        "Rotate reading-pane image left",
        "Reader Ctrl Left",
    ),
    (
        Command::RotateRight,
        "Rotate reading-pane image right",
        "Reader Ctrl Right",
    ),
];

impl Ledger {
    pub(super) fn palette_ui(&mut self, ctx: &egui::Context) -> bool {
        if self.palette {
            let mut chosen = None;
            let mut submit = false;
            let mut dismiss = false;
            let modal = egui::Modal::new(Id::new("palette")).show(ctx, |ui| {
                ui.set_width(680.0);
                ui.heading("Ask");
                let query_id = Id::new("palette-query");
                let keyboard = ui.memory(|memory| memory.has_focus(query_id));
                let enter = keyboard && consume_prompt_key(ctx, Modifiers::NONE, Key::Enter);
                if keyboard {
                    if consume_prompt_key(ctx, Modifiers::NONE, Key::ArrowDown) {
                        self.palette_index += 1;
                        self.palette_command_selection = true;
                    }
                    if consume_prompt_key(ctx, Modifiers::NONE, Key::ArrowUp) {
                        self.palette_index = self.palette_index.saturating_sub(1);
                        self.palette_command_selection = true;
                    }
                }
                if accessible_text_value(
                    ui.ctx(),
                    Id::new("palette-query"),
                    &mut self.palette_query,
                ) {
                    self.palette_index = 0;
                    self.palette_command_selection = false;
                }
                let response = ui.add(
                    egui::TextEdit::multiline(&mut self.palette_query)
                        .id(query_id)
                        .char_limit(2048)
                        .desired_rows(4)
                        .hint_text("Find a command, /rename, or describe a file search...")
                        .desired_width(f32::INFINITY),
                );
                if self.palette_focus {
                    response.request_focus();
                    self.palette_focus = false;
                }
                if response.changed() {
                    self.palette_index = 0;
                    self.palette_command_selection = false;
                }
                ui.ctx().accesskit_node_builder(response.id, |node| {
                    node.set_label("Ask prompt / AI and slash commands");
                    node.add_action(egui::accesskit::Action::SetValue)
                });
                let matches: Vec<_> = COMMANDS
                    .iter()
                    .filter(|(command, name, _)| {
                        palette_matches(&self.palette_query, *command, name)
                    })
                    .collect();
                self.palette_index = self.palette_index.min(matches.len().saturating_sub(1));
                self.usage.ask_candidates = matches.len();
                egui::ScrollArea::vertical()
                    .id_salt("palette-commands")
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (index, (command, name, hint)) in matches.iter().enumerate() {
                            let response = ui.add(
                                egui::Button::new(palette_label(ui, *command, name, self.accent()))
                                    .shortcut_text(RichText::new(*hint).monospace().size(11.0))
                                    .min_size(Vec2::new(ui.available_width(), 32.0))
                                    .selected(index == self.palette_index),
                            );
                            ui.ctx().accesskit_node_builder(response.id, |node| {
                                node.set_label(format!("{}  {name}    {hint}", command.slash()));
                            });
                            if index == self.palette_index {
                                response.scroll_to_me(None);
                            }
                            if response.clicked() {
                                chosen = Some(*command);
                            }
                        }
                    });
                if enter
                    && (self.palette_query.trim().starts_with('/')
                        || self.palette_command_selection
                        || self.palette_query.trim().is_empty())
                    && let Some((command, _, _)) = matches.get(self.palette_index)
                {
                    chosen = Some(*command);
                }
                submit = enter && chosen.is_none();
                if !self.palette_query.trim().starts_with('/') {
                    submit |= ui
                        .add_enabled(
                            !self.palette_query.trim().is_empty() && !self.file_agent.busy(),
                            egui::Button::new("Search with AI"),
                        )
                        .clicked();
                }
                if self.file_agent.busy() {
                    ui.label("An AI task is running. Open Notifications for progress; slash commands remain available.");
                }
                dismiss = ui.button("Close").on_hover_text("Escape").clicked();
            });
            dismiss |= modal.should_close();
            if dismiss {
                self.palette = false;
                self.file_focus_requested = true;
            }
            if !dismiss && (chosen.is_some() || submit) {
                let via = if self.palette_query.trim().starts_with('/') {
                    Via::Slash
                } else {
                    Via::Palette
                };
                let submission = if let Some(command) = chosen
                    && !submit
                    && !self.palette_query.trim().starts_with('/')
                {
                    Ok((command, String::new()))
                } else {
                    palette_submission(&self.palette_query, chosen)
                };
                match submission {
                    Ok((command, prompt)) => {
                        if command == Command::AiSearch && !prompt.is_empty() {
                            #[cfg(feature = "native-search-tests")]
                            let legacy_test = self.ai.test_scenario.is_some();
                            #[cfg(not(feature = "native-search-tests"))]
                            let legacy_test = false;
                            if !legacy_test {
                                self.usage_ask("ai");
                                self.palette = false;
                                self.file_agent.prepare(
                                    prompt,
                                    self.panes[self.active].path.clone(),
                                    self.active,
                                    self.scope.clone(),
                                );
                                self.file_agent.start(ctx, false);
                                return true;
                            }
                        }
                        self.usage_ask_command(command);
                        if matches!(command, Command::Search | Command::AiSearch)
                            && !prompt.is_empty()
                        {
                            self.dispatch(via, Command::Search);
                            if let Some(dialog) = &mut self.search_dialog {
                                dialog.ai_assisted = command == Command::AiSearch;
                                dialog.query = prompt;
                                if dialog.ai_assisted {
                                    dialog.interpreting = self.ai.start_search(ctx, &dialog.query);
                                    dialog.busy = dialog.interpreting;
                                    if !dialog.busy {
                                        dialog.error = Some(self.ai.status().to_owned());
                                    }
                                }
                            }
                        } else {
                            self.dispatch(via, command);
                        }
                    }
                    Err(error) => {
                        self.usage_ask(if error.starts_with("Unknown") {
                            "no_match"
                        } else {
                            "invalid"
                        });
                        self.palette = false;
                        self.fail(error);
                    }
                }
            }
        }
        false
    }
}
