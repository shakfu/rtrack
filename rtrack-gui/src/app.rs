use std::path::PathBuf;
use std::time::Instant;

use rtrack_core::TrackerCore;
use rtrack_core::tracker::Cell;

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
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut core = TrackerCore::with_song_size(8, 64);

        // Try to start audio engine
        match rtrack_core::audio::AudioEngine::new(None) {
            Ok(engine) => {
                core.audio = Some(engine);
            }
            Err(e) => {
                eprintln!("Audio warning: {}", e);
            }
        }

        cc.egui_ctx.set_theme(egui::Theme::Dark);

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
            self.core.engine.order
        } else {
            self.edit_order
        }
    }
}

impl eframe::App for RtrackApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Tick playback
        self.core.sync_link();
        if self.core.is_playing() {
            if self.core.tick_playback() {
                // Follow playback cursor
                if self.follow_playback && self.core.engine.tick == 1 {
                    self.cursor_row = self.core.engine.row;
                    self.edit_order = self.core.engine.order;
                }
            }
            ctx.request_repaint();
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
        if let Some(err) = self.core.auto_save(&mut self.last_autosave) {
            self.status_message = Some(err);
        }

        // Process keyboard input
        self.process_keys(ctx);

        // Intercept window close when dirty
        if ctx.input(|i| i.viewport().close_requested())
            && self.core.dirty
        {
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
                    self.core.preview_note_with_instrument(0, 60, 100, Some(slot as u8));
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
                    SliceMode::Equal => self.do_equal_slice(inst_idx, action.slot),
                    SliceMode::Transient => self.do_transient_slice(inst_idx, action.slot),
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
                let pattern_idx = self.core.song.order[order_pos];
                let pattern = &self.core.song.patterns[pattern_idx];

                let muted: Vec<bool> = self
                    .core
                    .channels
                    .iter()
                    .map(|c| c.muted)
                    .collect();
                let names: Vec<String> = self
                    .core
                    .channels
                    .iter()
                    .map(|c| c.name.clone())
                    .collect();

                // Compute how many channels fit in the available width
                let visible_count = grid::max_visible_channels(ui.available_width())
                    .min(self.core.song.channels);

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
                    playback_row: self.core.engine.row,
                    playback_order: self.core.engine.order,
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
                    }
                }
            });
        }
    }
}
