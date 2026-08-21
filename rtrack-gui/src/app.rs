use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use rtrack_core::tracker::Cell;
use rtrack_core::TrackerCore;

use crate::grid::{self, GridAction, GridParams};
use crate::history::EditHistory;
use crate::state::{GridColors, Mode, SubColumn, Theme};
use crate::visualization::VisualizationState;

pub struct RtrackApp {
    pub core: TrackerCore,

    // Cursor
    pub cursor_row: usize,
    pub cursor_channel: usize,
    pub cursor_sub: SubColumn,
    pub current_octave: u8,
    pub edit_step: usize,
    pub edit_order: usize,
    pub follow_playback: bool,
    pub first_visible_channel: usize,

    // Mode
    pub mode: Mode,

    // Status
    pub status_message: Option<String>,

    // Undo/Redo
    pub history: EditHistory,

    // Clipboard
    pub clipboard: Option<Cell>,
    pub block_clipboard: Option<Vec<Vec<Cell>>>,

    // Block selection
    pub block_start: Option<(usize, usize)>,
    pub block_end: Option<(usize, usize)>,

    // Dialogs
    pub show_song_settings: bool,
    pub show_quit_confirm: bool,
    pub show_track_config: Option<usize>,
    pub show_help: bool,
    pub show_midi_ports: bool,
    pub midi_port_list: Vec<String>,
    pub midi_input_port_list: Vec<String>,

    // Instrument dialogs
    pub show_instrument_list: bool,
    pub selected_instrument: Option<usize>,
    pub slice_count: usize,
    pub slice_sensitivity: f32,

    // Pattern matrix
    pub show_pattern_matrix: bool,
    pub matrix_cursor: usize,

    // Recent files
    pub recent_files: Vec<PathBuf>,

    // Theme
    pub theme: Theme,
    pub grid_colors: GridColors,

    // Auto-save
    pub last_autosave: Instant,

    // Visualization
    pub vis: VisualizationState,
    pub show_visualization: bool,
}

impl RtrackApp {
    /// Build the app around an already-constructed core. Split out from
    /// `new` so that tests can drive the editor with a headless core, with
    /// no window, audio device or MIDI port involved.
    fn with_core(core: TrackerCore) -> Self {
        Self {
            core,
            cursor_row: 0,
            cursor_channel: 0,
            cursor_sub: SubColumn::Note,
            current_octave: 4,
            edit_step: 1,
            edit_order: 0,
            follow_playback: true,
            first_visible_channel: 0,
            mode: Mode::Normal,
            status_message: None,
            history: EditHistory::new(100),
            clipboard: None,
            block_clipboard: None,
            block_start: None,
            block_end: None,
            show_song_settings: false,
            show_quit_confirm: false,
            show_track_config: None,
            show_help: false,
            show_midi_ports: false,
            midi_port_list: Vec::new(),
            midi_input_port_list: Vec::new(),
            show_instrument_list: false,
            selected_instrument: None,
            slice_count: 8,
            slice_sensitivity: 0.5,
            show_pattern_matrix: false,
            matrix_cursor: 0,
            recent_files: rtrack_core::config::load_recent_files(),
            theme: Theme::Dark,
            grid_colors: GridColors::dark(),
            last_autosave: Instant::now(),
            vis: VisualizationState::new(),
            show_visualization: true,
        }
    }

    /// Render what a load reported into a one-line status message.
    pub(crate) fn describe_load(report: &rtrack_core::core::LoadReport) -> String {
        if report.is_clean() {
            return format!("Loaded: {}", report.path.display());
        }
        let mut notes = Vec::new();
        if report.from_newer_version {
            notes.push("written by a newer rtrack; some settings may be missing".to_string());
        }
        if !report.repairs.is_empty() {
            notes.push(format!("repaired: {}", report.repairs.join("; ")));
        }
        if !report.missing_samples.is_empty() {
            let names: Vec<&str> = report
                .missing_samples
                .iter()
                .map(|(name, _)| name.as_str())
                .collect();
            notes.push(format!("missing samples: {}", names.join(", ")));
        }
        format!("Loaded ({}): {}", notes.join(" | "), report.path.display())
    }

    /// A test instance with no hardware attached.
    #[cfg(test)]
    pub fn headless(channels: usize, rows: usize) -> Self {
        Self::with_core(
            rtrack_core::core::TrackerCoreBuilder::new()
                .song_size(channels, rows)
                .headless()
                .build(),
        )
    }

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (config, mut startup_notes) = rtrack_core::config::load_config_verbose();
        let mut core = TrackerCore::with_song_size(8, 64);

        // Try to start audio engine with SF2 from config
        match rtrack_core::audio::AudioEngine::new(config.sf2.as_deref()) {
            Ok(engine) => {
                startup_notes.push(engine.device_description().to_string());
                core.audio = Some(engine);
            }
            Err(e) => {
                startup_notes.push(format!("No audio: {}", e));
            }
        }

        // Load sample directory from config
        if let Some(ref dir) = config.sample_dir {
            if dir.is_dir() {
                let bank = std::sync::Arc::make_mut(&mut core.sample_bank);
                if let Err(e) = bank.load_directory(dir) {
                    eprintln!("Sample dir warning: {}", e);
                }
            }
        }

        cc.egui_ctx.set_theme(egui::Theme::Dark);

        let mut app = Self::with_core(core);
        app.status_message = startup_notes.last().cloned();
        app
    }

    pub fn set_theme(&mut self, ctx: &egui::Context, theme: Theme) {
        self.theme = theme;
        self.grid_colors = GridColors::for_theme(theme);
        match theme {
            Theme::Dark | Theme::Monokai => ctx.set_theme(egui::Theme::Dark),
            Theme::Light => ctx.set_theme(egui::Theme::Light),
        }
    }

    pub(crate) fn current_order_position(&self) -> usize {
        if self.core.playing {
            self.core.playback_position().0
        } else {
            self.edit_order
        }
    }

    fn handle_dropped_files(&mut self, files: Vec<egui::DroppedFile>) {
        let audio_exts = ["wav", "aif", "aiff"];
        let rtrk_ext = "rtrk";
        let mut loaded = 0usize;

        for file in &files {
            let path = match &file.path {
                Some(p) => p.clone(),
                None => continue,
            };

            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            if ext == rtrk_ext {
                // Load as project file
                match self.core.load_file(&path) {
                    Ok(report) => {
                        self.cursor_row = 0;
                        self.cursor_channel = 0;
                        self.edit_order = 0;
                        self.first_visible_channel = 0;
                        self.history = EditHistory::new(100);
                        rtrack_core::config::push_recent_file(&mut self.recent_files, &path);
                        rtrack_core::config::save_recent_files(&self.recent_files);
                        self.status_message = Some(Self::describe_load(&report));
                    }
                    Err(e) => {
                        self.status_message =
                            Some(format!("Error loading {}: {}", path.display(), e));
                    }
                }
                return; // Only load one project file
            }

            if audio_exts.contains(&ext.as_str()) {
                // If instrument editor is open with a selection, use that instrument's slot;
                // otherwise find the first empty slot.
                let slot = if self.show_instrument_list {
                    self.selected_instrument
                        .and_then(|idx| self.core.instruments.get(idx))
                        .and_then(|inst| inst.sample_index)
                        .unwrap_or_else(|| {
                            self.selected_instrument.unwrap_or_else(|| {
                                (0..self.core.sample_bank.samples.len())
                                    .find(|&i| self.core.sample_bank.samples[i].is_none())
                                    .unwrap_or(0)
                            })
                        })
                } else {
                    (0..self.core.sample_bank.samples.len())
                        .find(|&i| self.core.sample_bank.samples[i].is_none())
                        .unwrap_or(0)
                };

                let mut bank = (*self.core.sample_bank).clone();
                match bank.load(slot, &path) {
                    Ok(()) => {
                        self.core.sample_bank = Arc::new(bank);
                        if let Some(ref mut audio) = self.core.audio {
                            audio.set_sample_bank(self.core.sample_bank.clone());
                        }
                        self.core.dirty = true;
                        loaded += 1;
                    }
                    Err(e) => {
                        self.status_message =
                            Some(format!("Error loading {}: {}", path.display(), e));
                    }
                }
            }
        }

        if loaded > 0 {
            self.status_message = Some(format!(
                "Loaded {} sample{}",
                loaded,
                if loaded == 1 { "" } else { "s" }
            ));
        }
    }
}

impl eframe::App for RtrackApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Tick playback
        self.core.sync_link();
        if self.core.is_playing() {
            self.core.tick_playback();
            // Follow the position being heard, not the one the sequencer has
            // run ahead to.
            if self.follow_playback {
                let (order, row) = self.core.playback_position();
                self.cursor_row = row;
                self.edit_order = order;
            }
            ctx.request_repaint();
        }
        if let Some(ref audio) = self.core.audio {
            if let Some(err) = audio.take_stream_error() {
                self.status_message = Some(format!("Audio error: {}", err));
            }
        }
        self.core.expire_preview_note();

        // Poll MIDI input
        self.poll_midi_input();

        // Update visualization
        self.vis.update(&mut self.core.audio);

        // Request repaint for smooth visualization when audio is active
        if self.show_visualization && self.core.audio.is_some() {
            ctx.request_repaint();
        }

        // Auto-save
        if let Err(e) = self.core.auto_save(&mut self.last_autosave) {
            self.status_message = Some(format!("Auto-save failed: {}", e));
        }

        // Handle dropped files (drag-and-drop sample loading)
        let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() {
            self.handle_dropped_files(dropped);
        }

        // Process keyboard input
        self.process_keys(ctx);

        // Intercept window close when dirty
        if ctx.input(|i| i.viewport().close_requested()) && self.core.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show_quit_confirm = true;
        }

        // Menu bar
        self.draw_menu_bar(ctx);

        // Transport bar
        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            self.draw_transport(ui);
        });

        // Status bar
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(ref msg) = self.status_message {
                    ui.label(msg.as_str());
                } else {
                    let hint = match self.mode {
                        Mode::Normal => "i:Insert  Space:Play  Arrows:Move  Tab:Channel  +/-:Octave  Ins:AddRow  BS:DelRow  Shift+Up/Dn:Transpose",
                        Mode::Insert => "Esc:Normal  Piano:z-m/q-u  =:NoteOff  Del:Clear  Shift+Up/Dn:Transpose",
                    };
                    ui.label(egui::RichText::new(hint).color(egui::Color32::from_rgb(120, 120, 140)));
                }
            });
        });

        // Dialogs
        self.draw_dialogs(ctx);

        // Visualization panel (bottom)
        if self.show_visualization {
            let sample_bank = self.core.sample_bank.clone();
            egui::TopBottomPanel::bottom("visualization")
                .exact_height(140.0)
                .show(ctx, |ui| {
                    self.vis.draw(ui, &sample_bank);
                });

            // Preview sample slot on click (only when not playing)
            if let Some(slot) = self.vis.preview_slot.take() {
                if !self.core.playing {
                    self.core
                        .preview_note_with_instrument(0, 60, 100, Some(slot as u8));
                }
            }

            // Apply slicing action from visualization panel
            if let Some(action) = self.vis.pending_slice_action.take() {
                use crate::visualization::SliceMode;
                self.slice_count = action.count;
                self.slice_sensitivity = action.sensitivity;
                let inst_idx = self
                    .core
                    .instruments
                    .iter()
                    .position(|i| i.sample_index == Some(action.slot))
                    .unwrap_or(action.slot);
                match action.mode {
                    SliceMode::Equal => {
                        self.do_equal_slice(inst_idx, action.slot, action.range, action.overwrite)
                    }
                    SliceMode::Transient => self.do_transient_slice(
                        inst_idx,
                        action.slot,
                        action.range,
                        action.overwrite,
                    ),
                }
            }
        }

        if self.show_instrument_list {
            // Instrument editor: sidebar panel + central panel
            egui::SidePanel::left("instrument_sidebar")
                .exact_width(230.0)
                .show(ctx, |ui| {
                    self.draw_instrument_sidebar(ui);
                });
            egui::CentralPanel::default().show(ctx, |ui| {
                self.draw_instrument_panel_view(ui);
            });
        } else if self.show_pattern_matrix {
            // Pattern matrix (full-screen, replaces sidebar + grid)
            egui::CentralPanel::default().show(ctx, |ui| {
                self.draw_pattern_matrix(ui);
            });
        } else {
            // Order list & channels sidebar
            self.draw_sidebar(ctx);

            // Pattern grid
            egui::CentralPanel::default().show(ctx, |ui| {
                let order_pos = self.current_order_position();
                let Some(pattern) = self.core.song.pattern_at(order_pos) else {
                    return;
                };

                let muted: Vec<bool> = self.core.channels.iter().map(|c| c.muted).collect();
                let names: Vec<String> =
                    self.core.channels.iter().map(|c| c.name.clone()).collect();

                // Compute how many channels fit in the available width
                let visible_count =
                    grid::max_visible_channels(ui.available_width()).min(self.core.song.channels);

                // Auto-scroll to keep cursor visible
                if self.cursor_channel < self.first_visible_channel {
                    self.first_visible_channel = self.cursor_channel;
                } else if self.cursor_channel >= self.first_visible_channel + visible_count {
                    self.first_visible_channel = self.cursor_channel + 1 - visible_count;
                }
                // Clamp in case window was resized or channels removed
                let max_first = self.core.song.channels.saturating_sub(visible_count);
                if self.first_visible_channel > max_first {
                    self.first_visible_channel = max_first;
                }

                let params = GridParams {
                    cursor_row: self.cursor_row,
                    cursor_channel: self.cursor_channel,
                    cursor_sub: self.cursor_sub,
                    mode: self.mode,
                    playing: self.core.playing,
                    playback_row: self.core.playback_position().1,
                    playback_order: self.core.playback_position().0,
                    edit_order: self.edit_order,
                    highlight_beat: self.core.song.highlight_beat,
                    highlight_bar: self.core.song.highlight_bar,
                    first_visible_channel: self.first_visible_channel,
                    visible_channel_count: visible_count,
                    muted_channels: muted,
                    solo_channel: self.core.solo_channel,
                    channel_names: names,
                    block_start: self.block_start,
                    block_end: self.block_end,
                    colors: self.grid_colors,
                };

                let actions = grid::draw_grid(ui, pattern, &params);
                for action in actions {
                    match action {
                        GridAction::SetCursor { row, channel, sub } => {
                            self.cursor_row = row;
                            self.cursor_channel = channel;
                            self.cursor_sub = sub;
                        }
                        GridAction::Scroll { rows } => {
                            let max_row = pattern.rows.saturating_sub(1);
                            let new_row = if rows > 0 {
                                self.cursor_row.saturating_add(rows as usize).min(max_row)
                            } else {
                                self.cursor_row.saturating_sub(rows.unsigned_abs() as usize)
                            };
                            self.cursor_row = new_row;
                        }
                        GridAction::DragStart { row, channel } => {
                            self.cursor_row = row;
                            self.cursor_channel = channel;
                            self.block_start = Some((row, channel));
                            self.block_end = Some((row, channel));
                        }
                        GridAction::DragUpdate { row, channel } => {
                            self.block_end = Some((row, channel));
                        }
                    }
                }
            });
        }
    }
}
