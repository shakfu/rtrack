//! # rtrack-tui
//!
//! Terminal UI frontend for the rtrack music tracker.
//!
//! This crate provides a full-featured TUI built on [ratatui](https://ratatui.rs)
//! and [crossterm](https://docs.rs/crossterm), wrapping the headless
//! [`rtrack_core::TrackerCore`] with modal keyboard input, pattern editing,
//! and terminal rendering.
//!
//! ## Modules
//!
//! - [`app`] -- [`App`](app::App) struct (owns `TrackerCore`), input modes, cursor state,
//!   undo/redo history, dialog management, and keyboard/mouse dispatch.
//! - [`tui`] -- ratatui rendering: pattern editor grid, header bar, status line,
//!   popup dialogs, and theme/color support.

pub mod app;
pub mod tui;
