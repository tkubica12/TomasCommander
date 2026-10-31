use super::*;

impl Ledger {
    pub(super) fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| {
            input.pointer.any_pressed()
                || input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))
        }) {
            self.inline_pending = None;
        }
        if self
            .name_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.inline)
        {
            if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
                self.name_dialog = None;
                self.file_focus_requested = true;
            }
            return;
        }
        if self.change_plan.is_some()
            || self.zip_dialog.is_some()
            || self.name_dialog.is_some()
            || self.open_with.is_some()
            || self.settings_open
            || self.operations_open
            || self.ai.open
            || self.file_agent.open
            || self.answers.open
            || self.help_open
            || self.places_popup.is_some()
            || self.drives_popup.is_some()
            || self.place_edit.is_some()
        {
            return;
        }
        let editing = ctx.memory(|memory| {
            if memory.has_focus(Id::new("reader-editor")) {
                return true;
            }
            if memory.has_focus(Id::new("archive-filter")) {
                return true;
            }
            if memory.has_focus(Id::new("reader-source-preview")) {
                return true;
            }
            (0..2).any(|pane| {
                memory.has_focus(Id::new(("path", pane)))
                    || memory.has_focus(Id::new(("filter", pane)))
            })
        });
        let copy = consume_file_copy(
            ctx,
            &mut self.copy_modifiers,
            !editing
                && ((self.reader.target != Some(self.active) && self.rail_index.is_none())
                    || (self.rail == Rail::Recent && self.rail_index.is_some()))
                && !self.palette
                && self.plan.is_none()
                && self.error.is_none()
                && self.search_dialog.is_none()
                && self.command_dialog.is_none(),
        );
        if self.plan.is_some()
            || self.error.is_some()
            || self.search_dialog.is_some()
            || self.command_dialog.is_some()
        {
            return;
        }
        if !self.palette
            && self.rail_index.is_none()
            && self.reader.target != Some(self.active)
            && self.archive.target != Some(self.active)
            && (!editing || ctx.memory(|memory| memory.has_focus(Id::new(("filter", self.active)))))
            && clear_filter_on_escape(ctx, &mut self.panes[self.active], self.active)
        {
            self.file_focus_requested = true;
            return;
        }
        if !self.palette && leave_filter_with_down(ctx, self.active) {
            let pane = &mut self.panes[self.active];
            pane.focused = pane.row_path(usize::from(!pane.visible.is_empty()));
            pane.anchor = pane.focus_position();
            pane.scroll_to_focus = true;
            self.file_focus_requested = true;
            return;
        }
        let chord = Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        if consume_prompt_key(ctx, Modifiers::CTRL, Key::Space) {
            self.palette = !self.palette;
            self.palette_focus = self.palette;
            self.palette_query.clear();
            self.palette_index = 0;
            self.palette_command_selection = false;
        }
        if self.palette || self.plan.is_some() || self.error.is_some() {
            return;
        }
        if consume_prompt_key(ctx, chord, Key::H) {
            ctx.memory_mut(|memory| {
                for pane in 0..2 {
                    memory.surrender_focus(Id::new(("path", pane)));
                    memory.surrender_focus(Id::new(("filter", pane)));
                }
                memory.move_focus(egui::FocusDirection::None);
            });
            self.dispatch(Command::Recent);
            return;
        }
        if self.reader.target.is_some()
            && self.rail_index.is_none()
            && (!editing || self.reader.target == Some(self.active))
            && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.reader.close();
            if self.reader.target.is_none() {
                self.active = self.reader.source;
                self.file_focus_requested = true;
            }
            return;
        }
        if self.reader.target == Some(self.active) && self.rail_index.is_none() {
            if consume_prompt_key(ctx, Modifiers::NONE, Key::Tab) {
                self.active = 1 - self.active;
                ctx.memory_mut(|memory| memory.surrender_focus(Id::new("reader-source-preview")));
                self.file_focus_requested = true;
                return;
            }
            if ctx.memory(|memory| memory.has_focus(Id::new("reader-source-preview")))
                && consume_prompt_key(ctx, Modifiers::CTRL, Key::Enter)
            {
                self.reader.close();
                self.active = self.reader.source;
                self.file_focus_requested = true;
                return;
            }
            for (key, command) in [(Key::S, Command::SaveSource), (Key::E, Command::EditSource)] {
                if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, key)) {
                    self.dispatch(command);
                    return;
                }
            }
        }
        if self.archive.target == Some(self.active)
            && self.rail_index.is_none()
            && self.archive.archive_filter_keys(ctx)
        {
            return;
        }
        if self.archive.target == Some(self.active)
            && self.rail_index.is_none()
            && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.archive.close();
            if self.reader.is_virtual() {
                self.reader.close();
            }
            self.file_focus_requested = true;
            return;
        }
        // Tab and history chords also work from the active pane's filter field.
        let filter = Id::new(("filter", self.active));
        if self.reader.target != Some(self.active)
            && (!editing || ctx.memory(|memory| memory.has_focus(filter)))
            && let Some(command) =
                consume_tab_command(ctx, self.archive.target != Some(self.active))
        {
            ctx.memory_mut(|memory| {
                memory.surrender_focus(filter);
                memory.move_focus(egui::FocusDirection::None);
            });
            self.rail_index = None;
            self.dispatch(command);
            self.file_focus_requested = true;
            return;
        }
        if editing {
            return;
        }
        if consume_help(ctx) {
            self.dispatch(Command::Help);
            return;
        }
        if self.rail_keys(ctx, copy) {
            return;
        }
        if self.reader.target == Some(self.active) {
            self.reader_pane_shortcuts(ctx);
            return;
        }
        if self.archive.target == Some(self.active) {
            self.archive_pane_shortcuts(ctx, copy, chord);
            return;
        }
        self.file_pane_shortcuts(ctx, copy, chord);
    }

    fn reader_pane_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.reader.is_virtual()
            && ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::Enter))
        {
            self.reader.close();
            if self.reader.target.is_none() {
                self.active = self.reader.source;
                self.file_focus_requested = true;
            }
            return;
        }
        for (key, delta) in [(Key::PageUp, -1), (Key::PageDown, 1)] {
            if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, key)) {
                if let Err(error) = self.reader.step_pdf(delta) {
                    self.fail(error);
                }
                return;
            }
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab)) {
            self.active = 1 - self.active;
            self.file_focus_requested = true;
        }
        if self.reader.is_archive() {
            if self.reader.archive_keys(ctx) {
                self.active = self.reader.source;
                self.reader.close();
                self.file_focus_requested = true;
            }
            return;
        }
        for (key, command) in [
            (Key::ArrowLeft, Command::RotateLeft),
            (Key::ArrowRight, Command::RotateRight),
        ] {
            if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, key)) {
                self.dispatch(command);
                return;
            }
        }
        for (key, delta) in [
            (Key::ArrowUp, -30.0),
            (Key::ArrowDown, 30.0),
            (Key::PageUp, -300.0),
            (Key::PageDown, 300.0),
        ] {
            if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, key)) {
                ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                self.reader.scroll(delta);
            }
        }
    }

    fn archive_pane_shortcuts(&mut self, ctx: &egui::Context, copy: bool, chord: Modifiers) {
        if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::F)) {
            ctx.memory_mut(|memory| memory.request_focus(Id::new("archive-filter")));
            return;
        }
        if copy || ctx.input_mut(|input| input.consume_key(chord, Key::C)) {
            self.dispatch(Command::Copy);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(chord, Key::M)) {
            self.dispatch(Command::Move);
            return;
        }
        if consume_prompt_key(ctx, chord, Key::P) {
            self.dispatch(Command::CreateZip);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(chord, Key::D)) {
            self.fail("ZIP member deletion is not a Recycle Bin operation; use an approved Move to a folder.");
            return;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab)) {
            self.active = 1 - self.active;
            return;
        }
        if self.archive.archive_keys(ctx) {
            self.archive.close();
            if self.reader.is_virtual() {
                self.reader.close();
            }
            self.file_focus_requested = true;
        }
    }

    fn file_pane_shortcuts(&mut self, ctx: &egui::Context, copy: bool, chord: Modifiers) {
        if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::Enter)) {
            self.dispatch(Command::Preview);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::SHIFT, Key::Enter)) {
            self.dispatch(Command::OpenWith);
            return;
        }
        if let Some(command) = consume_file_action(ctx) {
            self.dispatch(command);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::ALT, Key::ArrowUp)) {
            self.dispatch(Command::Places);
            return;
        }
        if copy {
            self.dispatch(Command::Copy);
            return;
        }
        for (key, command) in [
            (Key::ArrowLeft, Command::Parent),
            (Key::ArrowRight, Command::ReturnInside),
        ] {
            if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, key)) {
                ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                self.dispatch(command);
                return;
            }
        }
        if let Some(down) = consume_file_jump(ctx) {
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            self.dispatch(if down {
                Command::JumpDown
            } else {
                Command::JumpUp
            });
            return;
        }
        for (key, command) in FILE_PANE_CHORDS {
            if ctx.input_mut(|input| input.consume_key(chord, key)) {
                self.dispatch(command);
                return;
            }
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::F)) {
            self.dispatch(Command::Filter);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(chord, Key::ArrowLeft)) {
            self.left_width = (self.left_width - 35.0).max(240.0);
            self.resize_requested = true;
        }
        if ctx.input_mut(|input| input.consume_key(chord, Key::ArrowRight)) {
            self.left_width += 35.0;
            self.resize_requested = true;
        }
        if let Some(text) = take_filter_text(ctx) {
            let pane = &mut self.panes[self.active];
            pane.filter.push_str(&text);
            pane.rebuild();
            let id = Id::new(("filter", self.active));
            let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(pane.filter.chars().count()),
                )));
            state.store(ctx, id);
            self.inline_pending = None;
            self.file_focus_requested = false;
            self.editor_focus = Some(Id::new(("filter", self.active)));
            return;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab)) {
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            self.active = 1 - self.active;
            self.file_focus_requested = true;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter)) {
            self.open_focused();
            return;
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Backspace)) {
            self.parent(self.active);
            return;
        }
        let pane = &mut self.panes[self.active];
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            pane.selected.clear();
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::A)) {
            pane.selected = pane
                .visible
                .iter()
                .map(|index| pane.entries[*index].path.clone())
                .collect();
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Space))
            && let Some(path) = &pane.focused
            && !pane.selected.remove(path)
        {
            pane.selected.insert(path.clone());
        }
        for key in [Key::ArrowUp, Key::ArrowDown, Key::Home, Key::End] {
            let Some(modifiers) = consume_navigation_key(ctx, key) else {
                continue;
            };
            // egui resolves focus traversal from raw input before consume_key.
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            let previous = pane.focus_position();
            let position = match key {
                Key::ArrowUp => previous.saturating_sub(1),
                Key::ArrowDown => (previous + 1).min(pane.visible.len()),
                Key::Home => 0,
                _ => pane.visible.len(),
            };
            pane.focused = pane.row_path(position);
            if modifiers.shift {
                pane.selected = (pane.anchor.min(position)..=pane.anchor.max(position))
                    .filter_map(|position| pane.row_path(position))
                    .collect();
            } else {
                pane.anchor = position;
            }
            pane.scroll_to_focus = true;
            self.file_focus_requested = true;
        }
    }
}

pub(crate) fn consume_prompt_key(ctx: &egui::Context, chord: Modifiers, key: Key) -> bool {
    ctx.input_mut(|input| {
        let mut consumed = false;
        input.events.retain(|event| {
            if let egui::Event::Key {
                key: actual,
                pressed: true,
                modifiers,
                ..
            } = event
                && *actual == key
                && modifiers.matches_logically(chord)
                && modifiers.shift == chord.shift
                && modifiers.alt == chord.alt
            {
                consumed = true;
                false
            } else {
                true
            }
        });
        consumed
    })
}

pub(super) fn consume_file_action(ctx: &egui::Context) -> Option<Command> {
    let chord = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::NONE
    };
    FILE_ACTION_CHORDS
        .into_iter()
        .find_map(|(key, command)| consume_prompt_key(ctx, chord, key).then_some(command))
}

/// Ctrl+Shift actions matched exactly (no Alt), shared by keyboard handling and its tests.
pub(super) const FILE_ACTION_CHORDS: [(Key, Command); 5] = [
    (Key::E, Command::Rename),
    (Key::N, Command::NewFolder),
    (Key::R, Command::Refresh),
    (Key::P, Command::CreateZip),
    (Key::U, Command::Undo),
];

/// Other Ctrl+Shift file-pane chords.
pub(super) const FILE_PANE_CHORDS: [(Key, Command); 5] = [
    (Key::C, Command::Copy),
    (Key::M, Command::Move),
    (Key::S, Command::Sort),
    (Key::F, Command::Search),
    (Key::D, Command::Recycle),
];

/// Tab chords, matched exactly (no extra Shift or Alt) so plain Tab keeps switching panes.
pub(super) const TAB_CHORDS: [(Modifiers, Key, Command); 4] = [
    (
        Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::NONE
        },
        Key::Tab,
        Command::PreviousTab,
    ),
    (Modifiers::CTRL, Key::Tab, Command::NextTab),
    (Modifiers::CTRL, Key::T, Command::NewTab),
    (Modifiers::CTRL, Key::W, Command::CloseTab),
];

/// Folder history chords.
pub(super) const HISTORY_CHORDS: [(Modifiers, Key, Command); 2] = [
    (Modifiers::ALT, Key::ArrowLeft, Command::Back),
    (Modifiers::ALT, Key::ArrowRight, Command::Forward),
];

/// Consumes one tab chord, or a history chord when `history` is allowed (file panes).
pub(super) fn consume_tab_command(ctx: &egui::Context, history: bool) -> Option<Command> {
    TAB_CHORDS
        .into_iter()
        .chain(HISTORY_CHORDS.into_iter().filter(|_| history))
        .find_map(|(modifiers, key, command)| {
            consume_prompt_key(ctx, modifiers, key).then_some(command)
        })
}

/// Mouse side buttons over `rect`: `Some(false)` for Back, `Some(true)` for Forward.
pub(super) fn side_button_history(ctx: &egui::Context, rect: egui::Rect) -> Option<bool> {
    ctx.input(|input| {
        if !input
            .pointer
            .interact_pos()
            .is_some_and(|position| rect.contains(position))
        {
            None
        } else if input.pointer.button_pressed(egui::PointerButton::Extra1) {
            Some(false)
        } else if input.pointer.button_pressed(egui::PointerButton::Extra2) {
            Some(true)
        } else {
            None
        }
    })
}

// egui 0.36 text editors expose ValuePattern but do not process SetValue.
pub(crate) fn accessible_text_value(ctx: &egui::Context, id: Id, text: &mut String) -> bool {
    let mut changed = false;
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            if let egui::Event::AccessKitActionRequest(request) = event
                && request.target_node == id.accesskit_id()
                && request.action == egui::accesskit::Action::SetValue
                && let Some(egui::accesskit::ActionData::Value(value)) = &request.data
            {
                text.clear();
                text.push_str(value);
                changed = true;
                return false;
            }
            true
        })
    });
    if changed && let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        state.cursor.set_char_range(None);
        state.store(ctx, id);
    }
    changed
}

pub(super) fn consume_file_jump(ctx: &egui::Context) -> Option<bool> {
    for (key, modifiers, down) in [
        (Key::PageUp, Modifiers::NONE, false),
        (Key::PageDown, Modifiers::NONE, true),
        (Key::ArrowUp, Modifiers::CTRL, false),
        (Key::ArrowDown, Modifiers::CTRL, true),
    ] {
        if consume_prompt_key(ctx, modifiers, key) {
            return Some(down);
        }
    }
    None
}

pub(super) fn consume_file_copy(
    ctx: &egui::Context,
    modifiers: &mut Modifiers,
    enabled: bool,
) -> bool {
    // egui-winit converts Ctrl+C to Copy even when Shift is held.
    ctx.input_mut(|input| {
        if !input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::ModifiersChanged(_)))
        {
            *modifiers = input.modifiers;
        }
        let mut copy = false;
        input.events.retain(|event| {
            if let egui::Event::ModifiersChanged(changed) = event {
                *modifiers = *changed;
            }
            if matches!(event, egui::Event::Copy)
                && enabled
                && modifiers.ctrl
                && modifiers.shift
                && !modifiers.alt
                && !modifiers.mac_cmd
            {
                copy = true;
                return false;
            }
            true
        });
        *modifiers = input.modifiers;
        copy
    })
}

pub(super) fn consume_help(ctx: &egui::Context) -> bool {
    ctx.input_mut(|input| {
        let requested = input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Text(text) if text == "?"));
        input
            .events
            .retain(|event| !matches!(event, egui::Event::Text(text) if text == "?"));
        requested
    })
}

pub(super) fn consume_navigation_key(ctx: &egui::Context, key: Key) -> Option<Modifiers> {
    ctx.input_mut(|input| {
        let modifiers = input.events.iter().find_map(|event| match event {
            egui::Event::Key {
                key: event_key,
                pressed: true,
                modifiers,
                ..
            } if *event_key == key
                && (*modifiers == Modifiers::NONE || *modifiers == Modifiers::SHIFT) =>
            {
                Some(*modifiers)
            }
            _ => None,
        })?;
        input.consume_key(modifiers, key).then_some(modifiers)
    })
}

pub(super) fn leave_filter_with_down(ctx: &egui::Context, pane: usize) -> bool {
    let id = Id::new(("filter", pane));
    if !ctx.memory(|memory| memory.has_focus(id))
        || !ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown))
    {
        return false;
    }
    ctx.memory_mut(|memory| {
        memory.surrender_focus(id);
        memory.move_focus(egui::FocusDirection::None);
    });
    true
}

pub(super) fn clear_filter_on_escape(ctx: &egui::Context, pane: &mut Pane, index: usize) -> bool {
    if pane.filter.is_empty()
        || !ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
    {
        return false;
    }
    pane.filter.clear();
    pane.rebuild();
    pane.scroll_to_focus = true;
    ctx.memory_mut(|memory| {
        memory.surrender_focus(Id::new(("filter", index)));
        memory.move_focus(egui::FocusDirection::None);
    });
    true
}

pub(crate) fn take_filter_text(ctx: &egui::Context) -> Option<String> {
    ctx.input_mut(|input| {
        if input.modifiers.mac_cmd
            || (input.modifiers.ctrl && !input.modifiers.alt)
            || (input.modifiers.alt && !input.modifiers.ctrl)
        {
            return None;
        }
        if !input.events.iter().any(|event| {
            matches!(event, egui::Event::Text(value) if value.chars().any(|character| !character.is_whitespace() && !character.is_control()))
        }) {
            return None;
        }
        let mut text = String::new();
        input.events.retain(|event| {
            if let egui::Event::Text(value) = event
                && value.chars().any(|character| !character.is_control())
            {
                text.extend(value.chars().filter(|character| !character.is_control()));
                false
            } else {
                true
            }
        });
        (!text.is_empty()).then_some(text)
    })
}
