use rtrack_core::theory::{Scale, ScaleSetting};
use rtrack_core::tracker::NoteValue;

use crate::app::RtrackApp;

impl RtrackApp {
    pub fn draw_dialogs(&mut self, ctx: &egui::Context) {
        self.draw_song_settings(ctx);
        self.draw_quit_confirm(ctx);
        self.draw_track_config(ctx);
        self.draw_help(ctx);
        self.draw_midi_ports(ctx);
        self.draw_effect_prompt(ctx);
        self.draw_phrases(ctx);
        self.draw_tables(ctx);
    }

    /// The table editor: one table at a time, by number.
    fn draw_tables(&mut self, ctx: &egui::Context) {
        use rtrack_core::tracker::{MAX_TABLE_ROWS, MAX_TABLE_TICKS};
        if !self.show_tables {
            return;
        }
        let number = self.table_number;
        let table = self.core.song.tables.get(&number).cloned();
        let mut open = true;
        let mut action: Option<TableAction> = None;
        egui::Window::new("Tables")
            .open(&mut open)
            .resizable(true)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let mut n = number as i32;
                    if ui
                        .add(
                            egui::DragValue::new(&mut n)
                                .range(0..=255)
                                .custom_formatter(|v, _| format!("{:02X}", v as i64))
                                .custom_parser(|t| {
                                    i64::from_str_radix(t.trim(), 16).ok().map(|v| v as f64)
                                })
                                .prefix("Table "),
                        )
                        .on_hover_text("Instrument N plays table N; TBLxx plays table xx")
                        .changed()
                    {
                        action = Some(TableAction::Select(n as u8));
                    }
                    match &table {
                        Some(_) => {
                            if ui.button("Delete").clicked() {
                                action = Some(TableAction::Delete);
                            }
                        }
                        None => {
                            if ui.button("Create").clicked() {
                                action = Some(TableAction::Rows(1));
                            }
                        }
                    }
                });
                let Some(table) = table else {
                    ui.label("No table with this number.");
                    return;
                };
                let mut name = table.name.clone();
                if ui.text_edit_singleline(&mut name).changed() {
                    action = Some(TableAction::Name(name));
                }
                ui.horizontal(|ui| {
                    let mut ticks = table.ticks_per_row;
                    if ui
                        .add(
                            egui::DragValue::new(&mut ticks)
                                .range(1..=MAX_TABLE_TICKS)
                                .suffix(" ticks/row"),
                        )
                        .changed()
                    {
                        action = Some(TableAction::Ticks(ticks));
                    }
                    let mut looping = table.loop_from.is_some();
                    if ui.checkbox(&mut looping, "Loop from row").changed() {
                        action = Some(TableAction::Loop(looping.then_some(0)));
                    }
                    if let Some(from) = table.loop_from {
                        let mut from = from as usize;
                        if ui
                            .add(egui::DragValue::new(&mut from).range(0..=table.rows.len() - 1))
                            .changed()
                        {
                            action = Some(TableAction::Loop(Some(from as u8)));
                        }
                    }
                });
                ui.separator();
                egui::Grid::new("table_grid").num_columns(4).show(ui, |ui| {
                    ui.label("row");
                    ui.label("pitch");
                    ui.label("volume");
                    ui.label("effect");
                    ui.end_row();
                    for (i, row) in table.rows.iter().enumerate() {
                        ui.label(format!("{i:X}"));
                        let mut tr = row.transpose as i32;
                        if ui
                            .add(egui::DragValue::new(&mut tr).range(-48..=48))
                            .on_hover_text("Semitones above the note")
                            .changed()
                        {
                            action = Some(TableAction::Transpose(i, tr as i8));
                        }
                        ui.horizontal(|ui| {
                            let mut set = row.volume.is_some();
                            let mut v = row.volume.unwrap_or(0x40) as i32;
                            let toggled = ui.checkbox(&mut set, "").changed();
                            let dragged = set
                                && ui
                                    .add(egui::DragValue::new(&mut v).range(0..=127))
                                    .changed();
                            if toggled || dragged {
                                action = Some(TableAction::Volume(i, set.then_some(v as u8)));
                            }
                        });
                        let editing = matches!(&self.table_fx_edit, Some((r, _)) if *r == i);
                        let mut text = match &self.table_fx_edit {
                            Some((r, t)) if *r == i => t.clone(),
                            _ => row.fx.map(|fx| fx.to_string()).unwrap_or_default(),
                        };
                        let field =
                            ui.add(egui::TextEdit::singleline(&mut text).desired_width(60.0));
                        if field.changed() {
                            self.table_fx_edit = Some((i, text.clone()));
                        }
                        if editing && field.lost_focus() {
                            action = Some(TableAction::Effect(i, text));
                        }
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            table.rows.len() < MAX_TABLE_ROWS,
                            egui::Button::new("Add row"),
                        )
                        .clicked()
                    {
                        action = Some(TableAction::Rows(table.rows.len() + 1));
                    }
                    if ui
                        .add_enabled(table.rows.len() > 1, egui::Button::new("Remove row"))
                        .clicked()
                    {
                        action = Some(TableAction::Rows(table.rows.len() - 1));
                    }
                });
            });
        self.show_tables = open;
        if let Some(action) = action {
            self.apply_table_action(action);
        }
    }

    /// Apply one change from the table window. Each field folds its own
    /// run of changes into one undo step.
    pub(crate) fn apply_table_action(&mut self, action: TableAction) {
        let n = self.table_number;
        let before = self.begin_structural_edit();
        let control = match action {
            TableAction::Select(number) => {
                self.table_number = number;
                self.table_fx_edit = None;
                return;
            }
            TableAction::Delete => {
                self.core.song.tables.remove(&n);
                return self.end_structural_edit(before);
            }
            TableAction::Rows(rows) => {
                self.core.song.table_mut(n).resize(rows);
                return self.end_structural_edit(before);
            }
            TableAction::Name(name) => {
                self.core.song.table_mut(n).name = name;
                "table name"
            }
            TableAction::Ticks(ticks) => {
                self.core.song.table_mut(n).ticks_per_row = ticks;
                "table ticks"
            }
            TableAction::Loop(from) => {
                self.core.song.table_mut(n).loop_from = from;
                "table loop"
            }
            TableAction::Transpose(row, tr) => {
                self.core.song.table_mut(n).rows[row].transpose = tr;
                "table pitch"
            }
            TableAction::Volume(row, v) => {
                self.core.song.table_mut(n).rows[row].volume = v;
                "table volume"
            }
            TableAction::Effect(row, text) => {
                self.table_fx_edit = None;
                if let Err(e) = self.core.song.table_mut(n).set_row_effect(row, &text) {
                    self.status_message = Some(e);
                    return;
                }
                return self.end_structural_edit(before);
            }
        };
        self.record_settings_edit(control, before);
    }

    /// Phrase links for the cursor's track, and the phrase list.
    fn draw_phrases(&mut self, ctx: &egui::Context) {
        use rtrack_core::tracker::PhraseTiming;
        if !self.show_phrases {
            return;
        }
        let pos = self.edit_order;
        let ch = self.cursor_channel;
        let link = self.core.song.pattern_at(pos).and_then(|p| p.link(ch));
        let mut open = true;
        // One change per frame at most, applied after the window is drawn.
        let mut action: Option<PhraseAction> = None;
        egui::Window::new("Phrases")
            .open(&mut open)
            .resizable(true)
            .default_width(360.0)
            .show(ctx, |ui| {
                match link {
                    Some(l) => {
                        ui.label(format!("Track {} plays phrase {:02X}", ch + 1, l.phrase));
                        let mut steps = l.transpose as i32;
                        let max = rtrack_core::tracker::MAX_LINK_TRANSPOSE as i32;
                        if ui
                            .add(egui::DragValue::new(&mut steps).range(-max..=max).prefix("Transpose: "))
                            .on_hover_text(
                                "Applied at playback; the grid shows the phrase as stored. Scale degrees when the song has a scale, semitones otherwise.",
                            )
                            .changed()
                        {
                            action = Some(PhraseAction::Transpose(steps as i8));
                        }
                        ui.horizontal(|ui| {
                            let mut running = l.timing == PhraseTiming::Running;
                            if ui
                                .checkbox(&mut running, "Runs across patterns")
                                .on_hover_text(
                                    "Off: restarts with each pattern. On: keeps its phase through patterns, repeats and jumps.",
                                )
                                .changed()
                            {
                                action = Some(PhraseAction::Timing(running));
                            }
                            if ui
                                .button("Unlink")
                                .on_hover_text("The phrase rows become the track's own cells")
                                .clicked()
                            {
                                action = Some(PhraseAction::Unlink);
                            }
                        });
                    }
                    None => {
                        ui.label(format!("Track {} plays its own cells", ch + 1));
                        if ui.button("New phrase from track").clicked() {
                            action = Some(PhraseAction::LinkNew);
                        }
                    }
                }
                ui.separator();
                if self.core.song.phrases.is_empty() {
                    ui.label("No phrases yet.");
                }
                let shown = self.phrase_view.map(|v| v.phrase);
                egui::Grid::new("phrase_grid").num_columns(6).show(ui, |ui| {
                    for i in 0..self.core.song.phrases.len() {
                        // The number the track headers show, marked when
                        // it is the cursor track's phrase or the one on view.
                        let current = link.map(|l| l.phrase) == Some(i) || shown == Some(i);
                        ui.label(egui::RichText::new(format!("{i:02X}")).monospace().strong())
                            .on_hover_text("Shown in track headers as [Ann] or [Rnn]");
                        let mut name = self.core.song.phrases[i].name.clone();
                        let field = egui::TextEdit::singleline(&mut name).desired_width(140.0);
                        if ui.add(field).changed() {
                            action = Some(PhraseAction::Rename(i, name));
                        }
                        let mut len = self.core.song.phrases[i].rows.len();
                        if ui
                            .add(egui::DragValue::new(&mut len).range(1..=256).suffix(" rows"))
                            .changed()
                        {
                            action = Some(PhraseAction::Resize(i, len));
                        }
                        let uses = self.core.song.phrase_uses(i);
                        ui.label(if current {
                            egui::RichText::new(format!("{uses} uses")).strong()
                        } else {
                            egui::RichText::new(format!("{uses} uses"))
                        });
                        if ui
                            .button("Edit")
                            .on_hover_text("Show the phrase in the grid (F5)")
                            .clicked()
                        {
                            action = Some(PhraseAction::Show(i));
                        }
                        if ui.button("Link track").clicked() {
                            action = Some(PhraseAction::Link(i));
                        }
                        ui.end_row();
                    }
                });
            });
        self.show_phrases = open;
        if let Some(action) = action {
            self.apply_phrase_action(action);
        }
    }

    /// Apply one change from the phrase window, as one undo step. Typing a
    /// name and dragging a length fold into one step each.
    pub(crate) fn apply_phrase_action(&mut self, action: PhraseAction) {
        use rtrack_core::tracker::PhraseTiming;
        // Moving the view edits nothing.
        match action {
            PhraseAction::Show(i) => return self.open_phrase_view(Some(i)),
            PhraseAction::Step(forward) => return self.step_phrase_view(forward),
            PhraseAction::Add { duplicate } => return self.add_phrase_in_view(duplicate),
            PhraseAction::Close => return self.close_phrase_view(),
            _ => {}
        }
        let pos = self.edit_order;
        let ch = self.cursor_channel;
        let before = self.begin_structural_edit();
        let timing = self
            .core
            .song
            .pattern_at(pos)
            .and_then(|p| p.link(ch))
            .map_or(PhraseTiming::Aligned, |l| l.timing);
        match action {
            PhraseAction::LinkNew => {
                self.core.song.link_new_phrase(pos, ch, timing);
            }
            PhraseAction::Link(i) => {
                self.core.song.link_column(pos, ch, i, timing);
            }
            PhraseAction::Unlink => {
                self.core.song.unlink_column(pos, ch);
            }
            PhraseAction::Timing(running) => {
                let timing = if running {
                    PhraseTiming::Running
                } else {
                    PhraseTiming::Aligned
                };
                self.core.song.set_link_timing(pos, ch, timing);
            }
            PhraseAction::Rename(i, name) => {
                self.core.song.phrases[i].name = name;
                return self.record_settings_edit("phrase name", before);
            }
            PhraseAction::Transpose(steps) => {
                self.core.song.set_link_transpose(pos, ch, steps);
                return self.record_settings_edit("phrase transpose", before);
            }
            PhraseAction::Resize(i, len) => {
                self.core.song.resize_phrase(i, len);
                self.clamp_cursor_to_song();
                return self.record_settings_edit("phrase length", before);
            }
            PhraseAction::Show(_)
            | PhraseAction::Step(_)
            | PhraseAction::Add { .. }
            | PhraseAction::Close => unreachable!("handled above"),
        }
        self.end_structural_edit(before);
    }

    /// The bar above the grid while it shows a phrase: which phrase, its
    /// name and length, and moving between phrases.
    pub(crate) fn draw_phrase_bar(&mut self, ui: &mut egui::Ui) {
        let Some(view) = self.phrase_view else {
            return;
        };
        let Some(phrase) = self.core.song.phrases.get(view.phrase) else {
            return;
        };
        let count = self.core.song.phrases.len();
        let mut action: Option<PhraseAction> = None;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("Phrase {:02X}/{:02X}", view.phrase, count - 1))
                    .monospace()
                    .strong(),
            );
            if ui
                .add_enabled(view.phrase > 0, egui::Button::new("<"))
                .on_hover_text("Previous phrase (Ctrl+Left)")
                .clicked()
            {
                action = Some(PhraseAction::Step(false));
            }
            if ui
                .add_enabled(view.phrase + 1 < count, egui::Button::new(">"))
                .on_hover_text("Next phrase (Ctrl+Right)")
                .clicked()
            {
                action = Some(PhraseAction::Step(true));
            }
            let mut name = phrase.name.clone();
            let field = egui::TextEdit::singleline(&mut name).desired_width(140.0);
            if ui.add(field).changed() {
                action = Some(PhraseAction::Rename(view.phrase, name));
            }
            let mut len = phrase.rows.len();
            if ui
                .add(
                    egui::DragValue::new(&mut len)
                        .range(1..=256)
                        .suffix(" rows"),
                )
                .changed()
            {
                action = Some(PhraseAction::Resize(view.phrase, len));
            }
            ui.label(format!(
                "{} uses, plays through Ch{}",
                self.core.song.phrase_uses(view.phrase),
                self.cursor_channel + 1
            ));
            if ui.button("New").on_hover_text("Ctrl+N").clicked() {
                action = Some(PhraseAction::Add { duplicate: false });
            }
            if ui.button("Duplicate").on_hover_text("Ctrl+D").clicked() {
                action = Some(PhraseAction::Add { duplicate: true });
            }
            if ui.button("Back to pattern").on_hover_text("F5").clicked() {
                action = Some(PhraseAction::Close);
            }
        });
        if let Some(action) = action {
            self.apply_phrase_action(action);
        }
    }

    /// One-line editor for an effect's text form, e.g. PVB44 or C0740.
    fn draw_effect_prompt(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.effect_prompt.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Effect")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let field = ui.text_edit_singleline(&mut prompt.text);
                field.request_focus();
                ui.label("Name and hex parameter, e.g. PVB44 or C0740. Empty clears.");
                apply = ui.input(|i| i.key_pressed(egui::Key::Enter));
                cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        if apply {
            self.apply_effect_prompt();
        } else if cancel {
            self.effect_prompt = None;
        }
    }

    fn draw_song_settings(&mut self, ctx: &egui::Context) {
        if !self.show_song_settings {
            return;
        }

        let mut open = true;
        egui::Window::new("Song Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(300.0)
            .show(ctx, |ui| {
                egui::Grid::new("song_settings_grid")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        // Title
                        ui.label("Title:");
                        let mut title = self.core.song.title.clone();
                        ui.text_edit_singleline(&mut title);
                        if title != self.core.song.title {
                            let before = self.core.song.clone();
                            self.core.song.title = title;
                            self.record_settings_edit("song.title", before);
                        }
                        ui.end_row();

                        // BPM
                        ui.label("BPM:");
                        let mut bpm = self.core.song.bpm;
                        ui.add(egui::DragValue::new(&mut bpm).range(20..=999));
                        if bpm != self.core.song.bpm {
                            let before = self.core.song.clone();
                            self.core.song.bpm = bpm;
                            self.core.link.set_tempo(bpm as f64);
                            self.record_settings_edit("song.bpm", before);
                        }
                        ui.end_row();

                        // Speed
                        ui.label("Speed:");
                        let mut speed = self.core.song.speed;
                        ui.add(egui::DragValue::new(&mut speed).range(1..=31));
                        if speed != self.core.song.speed {
                            let before = self.core.song.clone();
                            self.core.song.speed = speed;
                            self.record_settings_edit("song.speed", before);
                        }
                        ui.end_row();

                        // Highlight Beat
                        ui.label("Beat highlight:");
                        let mut highlight_beat = self.core.song.highlight_beat;
                        ui.add(egui::DragValue::new(&mut highlight_beat).range(1..=64));
                        if highlight_beat != self.core.song.highlight_beat {
                            let before = self.core.song.clone();
                            self.core.song.highlight_beat = highlight_beat;
                            self.record_settings_edit("song.highlight_beat", before);
                        }
                        ui.end_row();

                        // Highlight Bar
                        ui.label("Bar highlight:");
                        let mut highlight_bar = self.core.song.highlight_bar;
                        ui.add(egui::DragValue::new(&mut highlight_bar).range(1..=256));
                        if highlight_bar != self.core.song.highlight_bar {
                            let before = self.core.song.clone();
                            self.core.song.highlight_bar = highlight_bar;
                            self.record_settings_edit("song.highlight_bar", before);
                        }
                        ui.end_row();

                        // Groove: relative row lengths; one number is swing
                        ui.label("Groove:");
                        let mut text = self
                            .groove_edit
                            .clone()
                            .unwrap_or_else(|| self.core.song.groove_text());
                        let field = ui.add(egui::TextEdit::singleline(&mut text).desired_width(120.0))
                            .on_hover_text("Row lengths, repeating: 67 33 swings; 55 45 52 48 pushes and pulls; off is straight");
                        if field.changed() {
                            self.groove_edit = Some(text.clone());
                        }
                        if field.lost_focus() && self.groove_edit.is_some() {
                            self.groove_edit = None;
                            match rtrack_core::tracker::Song::parse_groove(&text) {
                                Ok(groove) if groove != self.core.song.groove => {
                                    let before = self.core.song.clone();
                                    self.core.song.groove = groove;
                                    self.record_settings_edit("song.groove", before);
                                }
                                Ok(_) => {}
                                Err(e) => self.status_message = Some(e),
                            }
                        }
                        ui.end_row();

                        // Scale for keyboard note entry
                        ui.label("Scale:");
                        ui.horizontal(|ui| {
                            let current = self.core.song.scale;
                            let mut chosen = current;

                            let root = current.map(|s| s.root).unwrap_or(NoteValue::C);
                            let scale = current.map(|s| s.scale).unwrap_or(Scale::Major);

                            egui::ComboBox::from_id_salt("scale_root")
                                .width(50.0)
                                .selected_text(root.name())
                                .show_ui(ui, |ui| {
                                    for i in 0..12u8 {
                                        let v = NoteValue::from_index(i).unwrap();
                                        if ui.selectable_label(v == root, v.name()).clicked() {
                                            chosen = Some(ScaleSetting::new(v, scale));
                                        }
                                    }
                                });

                            egui::ComboBox::from_id_salt("scale_name")
                                .width(150.0)
                                .selected_text(match current {
                                    Some(s) => s.scale.name(),
                                    None => "off",
                                })
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(current.is_none(), "off").clicked() {
                                        chosen = None;
                                    }
                                    for s in Scale::ALL {
                                        let on = current.map(|c| c.scale) == Some(s);
                                        if ui.selectable_label(on, s.name()).clicked() {
                                            chosen = Some(ScaleSetting::new(root, s));
                                        }
                                    }
                                });

                            if chosen != current {
                                let before = self.core.song.clone();
                                self.core.song.scale = chosen;
                                self.record_settings_edit("song.scale", before);
                            }
                        });
                        ui.end_row();

                        // Rows per pattern (current pattern)
                        ui.label("Rows:");
                        let order_pos = if self.core.playing {
                            self.core.playback_position().0
                        } else {
                            self.edit_order
                        };
                        let mut rows = self
                            .core
                            .song
                            .pattern_at(order_pos)
                            .map(|p| p.rows)
                            .unwrap_or(self.core.song.rows_per_pattern);
                        let prev_rows = rows;
                        ui.add(egui::DragValue::new(&mut rows).range(1..=256));
                        if rows != prev_rows {
                            let before = self.core.song.clone();
                            if let Some(pattern) = self.core.song.pattern_at_mut(order_pos) {
                                pattern.resize_rows(rows);
                                self.record_settings_edit("song.rows", before);
                            }
                        }
                        ui.end_row();

                        ui.separator();
                        ui.separator();
                        ui.end_row();

                        // Channels
                        ui.label("Channels:");
                        let mut ch_count = self.core.song.channels;
                        let prev_ch_count = ch_count;
                        ui.add(
                            egui::DragValue::new(&mut ch_count)
                                .range(1..=rtrack_core::constants::MAX_CHANNELS),
                        );
                        if ch_count != prev_ch_count {
                            let before = self.core.song.clone();
                            self.core.song.channels = ch_count;
                            for pat in &mut self.core.song.patterns {
                                for row in &mut pat.data {
                                    row.resize(ch_count, rtrack_core::tracker::Cell::default());
                                }
                                pat.channels = ch_count;
                            }
                            while self.core.channels.len() < ch_count {
                                let idx = self.core.channels.len();
                                self.core
                                    .channels
                                    .push(rtrack_core::ChannelConfig::new(idx as u8));
                            }
                            self.core.channels.truncate(ch_count);
                            if self.cursor_channel >= ch_count {
                                self.cursor_channel = ch_count.saturating_sub(1);
                            }
                            self.record_settings_edit("song.channels", before);
                        }
                        ui.end_row();

                        // Pattern count (read-only)
                        ui.label("Patterns:");
                        ui.label(format!("{}", self.core.song.patterns.len()));
                        ui.end_row();

                        // Order length (read-only)
                        ui.label("Order length:");
                        ui.label(format!("{}", self.core.song.order.len()));
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        self.show_song_settings = false;
                        // Two visits to the same control with the dialog
                        // closed in between are two things the user did.
                        self.break_edit_coalescing();
                    }
                });
            });

        if !open {
            self.show_song_settings = false;
            self.break_edit_coalescing();
        }
    }

    fn draw_quit_confirm(&mut self, ctx: &egui::Context) {
        if !self.show_quit_confirm {
            return;
        }
        egui::Window::new("Unsaved Changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("You have unsaved changes. What would you like to do?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save & Quit").clicked() {
                        self.do_save();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Quit without saving").clicked() {
                        self.core.dirty = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_quit_confirm = false;
                    }
                });
            });
    }

    fn draw_track_config(&mut self, ctx: &egui::Context) {
        let ch_idx = match self.show_track_config {
            Some(idx) => idx,
            None => return,
        };

        let mut open = true;
        egui::Window::new(format!("Track {} Config", ch_idx + 1))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(350.0)
            .show(ctx, |ui| {
                if ch_idx >= self.core.channels.len() {
                    ui.label("Invalid channel");
                    return;
                }

                egui::Grid::new("track_config_grid")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        // Name
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.core.channels[ch_idx].name);
                        ui.end_row();

                        // Muted
                        ui.label("Muted:");
                        ui.checkbox(&mut self.core.channels[ch_idx].muted, "");
                        ui.end_row();

                        // MIDI Channel
                        ui.label("MIDI Ch:");
                        let mut midi_ch = self.core.channels[ch_idx].midi_channel as i32;
                        if ui
                            .add(egui::DragValue::new(&mut midi_ch).range(0..=15))
                            .changed()
                        {
                            self.core.channels[ch_idx].midi_channel = midi_ch as u8;
                        }
                        ui.end_row();

                        // Default instrument, for notes entered or played without one.
                        // -1 is none.
                        ui.label("Instrument:");
                        let mut inst = self.core.channels[ch_idx]
                            .default_instrument
                            .map_or(-1, |i| i as i32);
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::DragValue::new(&mut inst)
                                        .range(-1..=255)
                                        .custom_formatter(|v, _| match v as i32 {
                                            -1 => "none".to_string(),
                                            i => format!("{i:02X}"),
                                        }),
                                )
                                .changed()
                            {
                                self.core.channels[ch_idx].default_instrument =
                                    (inst >= 0).then_some(inst as u8);
                            }
                            ui.label(self.core.track_label(ch_idx));
                        });
                        ui.end_row();

                        // Volume
                        ui.label("Volume:");
                        ui.add(egui::Slider::new(
                            &mut self.core.channels[ch_idx].volume,
                            0.0..=1.0,
                        ));
                        ui.end_row();

                        // Pan
                        ui.label("Pan:");
                        ui.add(egui::Slider::new(
                            &mut self.core.channels[ch_idx].pan,
                            -1.0..=1.0,
                        ));
                        ui.end_row();
                    });

                // Per-channel effects section
                {
                    ui.separator();
                    ui.heading("Effects");
                    ui.add_space(4.0);

                    let fx = &mut self.core.channels[ch_idx].effects_params;

                    // Filter
                    ui.group(|ui| {
                        ui.checkbox(&mut fx.filter_enabled, "Filter");
                        if fx.filter_enabled {
                            ui.indent("filter_params", |ui| {
                                egui::Grid::new("filter_grid")
                                    .num_columns(2)
                                    .spacing([10.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Cutoff:");
                                        ui.add(
                                            egui::Slider::new(
                                                &mut fx.filter_cutoff,
                                                20.0..=20000.0,
                                            )
                                            .logarithmic(true)
                                            .suffix(" Hz"),
                                        );
                                        ui.end_row();
                                        ui.label("Resonance:");
                                        ui.add(egui::Slider::new(
                                            &mut fx.filter_resonance,
                                            0.0..=1.0,
                                        ));
                                        ui.end_row();
                                    });
                            });
                        }
                    });
                    ui.add_space(2.0);

                    // Distortion
                    ui.group(|ui| {
                        ui.checkbox(&mut fx.distortion_enabled, "Distortion");
                        if fx.distortion_enabled {
                            ui.indent("dist_params", |ui| {
                                egui::Grid::new("dist_grid")
                                    .num_columns(2)
                                    .spacing([10.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Drive:");
                                        ui.add(egui::Slider::new(
                                            &mut fx.distortion_drive,
                                            1.0..=20.0,
                                        ));
                                        ui.end_row();
                                    });
                            });
                        }
                    });
                    ui.add_space(2.0);

                    // Chorus
                    ui.group(|ui| {
                        ui.checkbox(&mut fx.chorus_enabled, "Chorus");
                        if fx.chorus_enabled {
                            ui.indent("chorus_params", |ui| {
                                egui::Grid::new("chorus_grid")
                                    .num_columns(2)
                                    .spacing([10.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Rate:");
                                        ui.add(
                                            egui::Slider::new(&mut fx.chorus_rate, 0.1..=10.0)
                                                .suffix(" Hz"),
                                        );
                                        ui.end_row();
                                        ui.label("Depth:");
                                        ui.add(egui::Slider::new(&mut fx.chorus_depth, 0.5..=20.0));
                                        ui.end_row();
                                        ui.label("Mix:");
                                        ui.add(egui::Slider::new(&mut fx.chorus_mix, 0.0..=1.0));
                                        ui.end_row();
                                    });
                            });
                        }
                    });
                    ui.add_space(2.0);

                    // Delay
                    ui.group(|ui| {
                        ui.checkbox(&mut fx.delay_enabled, "Delay");
                        if fx.delay_enabled {
                            ui.indent("delay_params", |ui| {
                                egui::Grid::new("delay_grid")
                                    .num_columns(2)
                                    .spacing([10.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Time:");
                                        ui.add(
                                            egui::Slider::new(&mut fx.delay_time, 1.0..=2000.0)
                                                .suffix(" ms"),
                                        );
                                        ui.end_row();
                                        ui.label("Feedback:");
                                        ui.add(egui::Slider::new(
                                            &mut fx.delay_feedback,
                                            0.0..=0.95,
                                        ));
                                        ui.end_row();
                                        ui.label("Mix:");
                                        ui.add(egui::Slider::new(&mut fx.delay_mix, 0.0..=1.0));
                                        ui.end_row();
                                    });
                            });
                        }
                    });
                    ui.add_space(2.0);

                    // Reverb
                    ui.group(|ui| {
                        ui.checkbox(&mut fx.reverb_enabled, "Reverb");
                        if fx.reverb_enabled {
                            ui.indent("reverb_params", |ui| {
                                egui::Grid::new("reverb_grid")
                                    .num_columns(2)
                                    .spacing([10.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Size:");
                                        ui.add(egui::Slider::new(&mut fx.reverb_size, 0.0..=1.0));
                                        ui.end_row();
                                        ui.label("Damp:");
                                        ui.add(egui::Slider::new(&mut fx.reverb_damp, 0.0..=1.0));
                                        ui.end_row();
                                        ui.label("Mix:");
                                        ui.add(egui::Slider::new(&mut fx.reverb_mix, 0.0..=1.0));
                                        ui.end_row();
                                    });
                            });
                        }
                    });

                    // MIDI Learn section
                    ui.separator();
                    ui.heading("MIDI Learn");

                    // Show learning status
                    if let Some((learn_ch, learn_param)) = self.core.midi_learn_pending {
                        if learn_ch == ch_idx {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Waiting for CC... ({})",
                                    learn_param.name()
                                ))
                                .color(egui::Color32::from_rgb(255, 200, 60)),
                            );
                            if ui.button("Cancel").clicked() {
                                self.core.midi_learn_pending = None;
                            }
                        }
                    }

                    // Learn buttons for each parameter
                    use rtrack_core::LearnableParam;
                    let params = [
                        LearnableParam::FilterCutoff,
                        LearnableParam::FilterResonance,
                        LearnableParam::DistortionDrive,
                        LearnableParam::ChorusRate,
                        LearnableParam::ChorusDepth,
                        LearnableParam::ChorusMix,
                        LearnableParam::DelayTime,
                        LearnableParam::DelayFeedback,
                        LearnableParam::DelayMix,
                        LearnableParam::ReverbSize,
                        LearnableParam::ReverbDamp,
                        LearnableParam::ReverbMix,
                    ];

                    egui::Grid::new("midi_learn_grid")
                        .num_columns(3)
                        .spacing([8.0, 3.0])
                        .show(ui, |ui| {
                            for param in &params {
                                let mapping = self
                                    .core
                                    .midi_cc_mappings
                                    .iter()
                                    .find(|m| m.channel == ch_idx && m.param == *param);

                                ui.label(param.name());
                                if let Some(m) = mapping {
                                    ui.label(format!("CC{}", m.cc));
                                    if ui.small_button("Unlearn").clicked() {
                                        let p = *param;
                                        self.core
                                            .midi_cc_mappings
                                            .retain(|m| !(m.channel == ch_idx && m.param == p));
                                        self.status_message = Some(format!(
                                            "Unlearned {} (ch {})",
                                            p.name(),
                                            ch_idx + 1
                                        ));
                                    }
                                } else {
                                    ui.label("---");
                                    if ui.small_button("Learn").clicked() {
                                        self.core.midi_learn_pending = Some((ch_idx, *param));
                                    }
                                }
                                ui.end_row();
                            }
                        });
                }

                ui.add_space(8.0);
                if ui.button("Close").clicked() {
                    self.show_track_config = None;
                }
            });

        if !open {
            self.show_track_config = None;
        }
    }

    fn draw_help(&mut self, ctx: &egui::Context) {
        if !self.show_help {
            return;
        }

        let mut open = true;
        egui::Window::new("Help")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .default_height(500.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let dim = egui::Color32::from_rgb(140, 140, 160);

                        ui.heading("General");
                        help_grid(ui, "help_general", &[
                            ("Space", "Play / Stop"),
                            ("Ctrl+Space", "Play from start"),
                            ("i", "Enter Insert mode"),
                            ("Escape", "Return to Normal mode / Close dialog"),
                            ("F1", "Toggle this help"),
                            ("F7", "Instrument editor"),
                            ("Ctrl+P", "Pattern matrix"),
                            ("Ctrl+S", "Save"),
                            ("Ctrl+Z", "Undo"),
                            ("Ctrl+Shift+Z", "Redo"),
                        ], dim);

                        ui.add_space(8.0);
                        ui.heading("Navigation");
                        help_grid(ui, "help_nav", &[
                            ("Up / Down", "Move cursor"),
                            ("Left / Right", "Move sub-column"),
                            ("Tab / Shift+Tab", "Next / previous channel"),
                            ("Page Up / Down", "Move 16 rows"),
                            ("Home / End", "First / last row"),
                            ("Ctrl+Left / Right", "Previous / next order position"),
                        ], dim);

                        ui.add_space(8.0);
                        ui.heading("Editing (Insert Mode)");
                        help_grid(ui, "help_edit", &[
                            ("z s x d c v g b h n j m", "Notes C C# D D# E F F# G G# A A# B (lower octave)"),
                            ("q 2 w 3 e r 5 t 6 y 7 u", "Notes C C# D D# E F F# G G# A A# B (upper octave)"),
                            ("= (equals)", "Note off"),
                            ("0-9 a-f", "Hex digit (instrument/volume/effect columns)"),
                            ("Delete", "Clear cell"),
                            ("Insert", "Insert row"),
                            ("Backspace", "Delete row"),
                            ("+  / -", "Octave up / down"),
                        ], dim);

                        ui.add_space(8.0);
                        ui.heading("Block Operations");
                        help_grid(ui, "help_block", &[
                            ("Ctrl+B", "Start/toggle block selection"),
                            ("Ctrl+C", "Copy (cell or block)"),
                            ("Ctrl+X", "Cut (cell or block)"),
                            ("Ctrl+V", "Paste (cell or block)"),
                            ("Ctrl+I", "Interpolate (volume/effect values in block)"),
                            ("Shift+Up / Down", "Transpose note up/down (cell or block)"),
                        ], dim);

                        ui.add_space(8.0);
                        ui.heading("Pattern / Song");
                        help_grid(ui, "help_pattern", &[
                            ("Ctrl+N", "New pattern (insert after current)"),
                            ("Ctrl+D", "Clone pattern (insert after current)"),
                            ("Ctrl+F", "Toggle follow mode"),
                            ("Ctrl+R", "Toggle recording"),
                            ("Enter", "Open track config (Normal mode)"),
                        ], dim);

                        ui.add_space(8.0);
                        ui.heading("Phrase Editor");
                        help_grid(ui, "help_phrase", &[
                            ("F5", "Show the cursor track's phrase in the grid / back"),
                            ("Ctrl+Left / Right", "Previous / next phrase"),
                            ("Ctrl+N / Ctrl+D", "New / duplicate phrase"),
                            ("Insert / Backspace", "Insert / delete a phrase row"),
                        ], dim);

                        ui.add_space(8.0);
                        ui.heading("Export");
                        help_grid(ui, "help_export", &[
                            ("Ctrl+E", "Export MIDI"),
                            ("Ctrl+W", "Export WAV"),
                            ("Ctrl+L", "Export FLAC"),
                            ("Ctrl+M", "Toggle MIDI clock output"),
                        ], dim);

                        ui.add_space(8.0);
                        ui.heading("Sub-columns");
                        ui.label(
                            egui::RichText::new(
                                "Each channel has 4 sub-columns: Note | Instrument | Volume | Effect\n\
                                 Use Left/Right arrows to move between them."
                            ).color(dim),
                        );
                    });

                ui.add_space(4.0);
                if ui.button("Close").clicked() {
                    self.show_help = false;
                }
            });

        if !open {
            self.show_help = false;
        }
    }

    fn draw_midi_ports(&mut self, ctx: &egui::Context) {
        if !self.show_midi_ports {
            return;
        }

        let mut open = true;
        egui::Window::new("MIDI Ports")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(400.0)
            .show(ctx, |ui| {
                // Output ports
                ui.heading("Output");
                if self.midi_port_list.is_empty() {
                    ui.label("No MIDI output ports found.");
                } else {
                    let current = self.core.midi_port_display_name().to_string();
                    for port_name in self.midi_port_list.iter() {
                        let selected = *port_name == current;
                        let label = if selected {
                            format!("> {}", port_name)
                        } else {
                            port_name.clone()
                        };
                        if ui.selectable_label(selected, &label).clicked() {
                            match self.core.select_midi_port(port_name) {
                                Ok(()) => {
                                    self.status_message =
                                        Some(format!("Output: connected to {}", port_name));
                                }
                                Err(e) => {
                                    self.status_message =
                                        Some(format!("MIDI connect error: {}", e));
                                }
                            }
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if ui.small_button("Virtual Out").clicked() {
                        match self
                            .core
                            .select_midi_port(rtrack_core::midi::VIRTUAL_PORT_NAME)
                        {
                            Ok(()) => {
                                self.status_message =
                                    Some("Created virtual MIDI output port".to_string());
                            }
                            Err(e) => {
                                self.status_message = Some(format!("Virtual port error: {}", e));
                            }
                        }
                    }
                });

                ui.add_space(8.0);
                ui.separator();

                // Input ports
                ui.heading("Input");
                let input_port_name = self.core.midi_input.port_name.clone().unwrap_or_default();
                if self.midi_input_port_list.is_empty() {
                    ui.label("No MIDI input ports found.");
                } else {
                    for (i, port_name) in self.midi_input_port_list.iter().enumerate() {
                        let selected = *port_name == input_port_name;
                        let label = if selected {
                            format!("> {}", port_name)
                        } else {
                            port_name.clone()
                        };
                        if ui.selectable_label(selected, &label).clicked() {
                            match self.core.midi_input.connect(i) {
                                Ok(()) => {
                                    self.status_message =
                                        Some(format!("Input: connected to {}", port_name));
                                }
                                Err(e) => {
                                    self.status_message = Some(format!("MIDI input error: {}", e));
                                }
                            }
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if ui.small_button("Virtual In").clicked() {
                        match self.core.midi_input.create_virtual_port() {
                            Ok(()) => {
                                self.status_message =
                                    Some("Created virtual MIDI input port".to_string());
                            }
                            Err(e) => {
                                self.status_message = Some(format!("Virtual input error: {}", e));
                            }
                        }
                    }
                });

                ui.add_space(8.0);
                ui.separator();

                // Clock mode
                ui.heading("Clock");
                {
                    use rtrack_core::ClockMode;
                    let is_external = self.core.clock_mode == ClockMode::ExternalMidi;
                    ui.horizontal(|ui| {
                        ui.label("Source:");
                        if ui.selectable_label(!is_external, "Internal").clicked() && is_external {
                            self.core.toggle_clock_mode();
                            self.status_message = Some("Clock: Internal".to_string());
                        }
                        if ui.selectable_label(is_external, "External MIDI").clicked()
                            && !is_external
                        {
                            self.core.toggle_clock_mode();
                            self.status_message = Some("Clock: External MIDI".to_string());
                        }
                    });

                    // MIDI clock output toggle
                    let mut clock_out = self.core.midi.clock_enabled;
                    if ui.checkbox(&mut clock_out, "Send MIDI clock").changed() {
                        let msg = format!(
                            "MIDI clock {}",
                            if self.core.toggle_midi_clock() {
                                "on"
                            } else {
                                "off"
                            }
                        );
                        self.status_message = Some(msg);
                    }
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Refresh").clicked() {
                        self.midi_port_list =
                            rtrack_core::midi::MidiEngine::list_ports().unwrap_or_default();
                        self.midi_input_port_list =
                            rtrack_core::midi::MidiInputEngine::list_ports().unwrap_or_default();
                    }
                    if ui.button("Close").clicked() {
                        self.show_midi_ports = false;
                    }
                });
            });

        if !open {
            self.show_midi_ports = false;
        }
    }

    pub fn draw_load_sf2(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("SoundFont", &["sf2", "SF2"])
            .pick_file()
        {
            match rtrack_core::audio::AudioEngine::new(Some(&path)) {
                Ok(engine) => {
                    self.core.audio = Some(engine);
                    self.status_message = Some(format!(
                        "Loaded SF2: {}",
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown")
                    ));
                }
                Err(e) => {
                    self.status_message = Some(format!("Failed to load SF2: {}", e));
                }
            }
        }
    }
}

fn help_grid(ui: &mut egui::Ui, id: &str, items: &[(&str, &str)], dim: egui::Color32) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([16.0, 3.0])
        .show(ui, |ui| {
            for (key, desc) in items {
                ui.label(egui::RichText::new(*key).monospace().strong());
                ui.label(egui::RichText::new(*desc).color(dim));
                ui.end_row();
            }
        });
}

/// A change made in the phrase window.
pub(crate) enum PhraseAction {
    LinkNew,
    Link(usize),
    Unlink,
    Timing(bool),
    Transpose(i8),
    Rename(usize, String),
    Resize(usize, usize),
    /// Show phrase n in the grid.
    Show(usize),
    /// Show the next (true) or previous phrase.
    Step(bool),
    /// Add an empty phrase, or a copy of the one shown, and show it.
    Add {
        duplicate: bool,
    },
    /// Return the grid to the pattern.
    Close,
}

/// A change made in the table window.
pub(crate) enum TableAction {
    Select(u8),
    Delete,
    Rows(usize),
    Name(String),
    Ticks(u8),
    Loop(Option<u8>),
    Transpose(usize, i8),
    Volume(usize, Option<u8>),
    Effect(usize, String),
}
