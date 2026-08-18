//! # rtrack-core
//!
//! Headless music tracker core: engine, audio, MIDI, samples, and data model.
//!
//! This crate provides the complete tracker engine without any UI dependencies.
//! Both the TUI ([`rtrack-tui`]) and GUI ([`rtrack-gui`]) frontends wrap
//! [`TrackerCore`] with their own input handling and rendering.
//!
//! ## Modules
//!
//! - [`tracker`] -- Song, Pattern, Note, and Cell data model (serializable).
//! - [`engine`] -- Deterministic tick-based playback engine emitting [`engine::TrackerEvent`]s.
//! - [`audio`] -- Real-time audio output: built-in synthesizer (30 patches), sample playback,
//!   per-channel effects (filter, distortion, chorus), and send/return buses (delay, reverb).
//! - [`midi`] -- MIDI output and input engines (virtual ports, hardware routing).
//! - [`link`] -- Ableton Link tempo and transport synchronization.
//! - [`sample`] -- WAV/AIFF sample loading, slicing, and offline export (WAV/FLAC).
//! - [`midi_file`] -- Standard MIDI file import and export.
//! - [`config`] -- User configuration file (`~/.config/rtrack/config.toml`).
//! - [`types`] -- Shared types: [`ChannelConfig`], [`Instrument`], [`ClockMode`], etc.
//! - [`constants`] -- Numeric constants (MIDI, music theory, effect commands, tracker limits).
//!
//! ## Quick start
//!
//! ```no_run
//! use rtrack_core::TrackerCore;
//!
//! let mut core = TrackerCore::new(); // 4 channels, 64 rows
//! core.play(0, 0);                   // start playback from order 0, row 0
//! // In a loop: core.tick_playback() drives the engine forward.
//! core.stop();
//! ```

pub mod audio;
pub mod config;
pub mod constants;
pub mod core;
pub mod engine;
pub mod error;
pub mod keymap;
pub mod link;
pub mod midi;
pub mod midi_file;
pub mod sample;
pub mod tracker;
pub mod types;

// Re-export key types at the crate root for convenience
pub use core::{TrackerCore, TrackerCoreBuilder};
pub use types::*;
