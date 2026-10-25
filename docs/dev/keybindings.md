# Keybinding customization

Lets a user change rtrack's keys from `config.toml`: the editing keys of both frontends and the piano layout for note entry. Closes the TODO.md Low item "Keybinding customization (config file with tracker presets)". Code refers to `rtrack-core/src`, `rtrack-tui/src` and `rtrack-gui/src` as of 2026-10-25.

Status: step 1 (piano layouts) implemented 2026-10-25, with the proposal for decisions 1-3. A piano key now wins over octave-down in Insert mode on the note column, which AZERTY's `-` needs. `PianoLayout` is read by `config::load_piano_layout` rather than a `Config` field, so `Config`'s fields do not change. Step 2 (actions) is not built.

## Current state

- **TUI.** 20 per-mode handlers in `app/input.rs` (`handle_normal_key`, `handle_insert_key`, `handle_common_key`, `handle_shared_key`, one per dialog) match `KeyCode` and `KeyModifiers` directly: 186 `KeyCode::` arms. There is no list of actions.
- **GUI.** `input.rs` turns egui keys into an `Action` enum (`TogglePlayback`, `CursorUp(n)`, `Save`, `Undo`, ...), then applies it. The table from keys to actions is code.
- **Shared.** `rtrack_core::keymap::piano_key` maps characters to notes, QWERTY only: `z s x d c v g b h n j m` and `q 2 w 3 e r 5 t 6 y 7 u`. On QWERTZ `z` and `y` swap, so the low C and the high A trade places; on AZERTY `a`/`q` and `z`/`w` swap as well (inferred from the layouts, not tested here).
- **Config.** `config.toml` is user-edited and rtrack never writes it (`config.rs`, G4 + G8 notes). Help screens list keys as fixed text in both frontends.
- **Physical keys.** egui reports a `physical_key` per event. crossterm reports physical keys only under the kitty keyboard protocol, which few terminals enable (inferred from crossterm's `KeyboardEnhancementFlags`; not tested here).

## Options

**A. Editing actions plus piano layout.** One `Action` list in core, covering the pattern editor's keys: transport, cursor, edit, block, undo, octave, step, file. Both frontends map keys to it. `config.toml` gains a `[keys]` table that overrides bindings, and `piano = "qwertz"` picks a layout. Dialog keys stay fixed.
- One list, so the frontends agree and the help screens are generated from it.
- Dialog keys are mostly arrows, Tab, Enter and Esc, which nobody rebinds.

**B. Everything.** Actions for every mode, dialogs included.
- Complete, about 186 TUI arms turned into actions.
- Most of the work buys rebinding of Esc in the quit prompt.

**C. Piano layout only.** Layout presets for note entry, nothing else.
- Fixes the case that breaks note entry outright: a non-QWERTY keyboard.
- Leaves Ctrl+S and the rest fixed.

**D. Physical keys.** Map note entry by key position, not character, so every layout works with no setting.
- The GUI can; the TUI cannot without the kitty protocol, so the two would differ.

| | A | B | C | D |
|-|-|-|-|-|
| Non-QWERTY note entry | preset | preset | preset | automatic, GUI only |
| Rebind editing keys | yes | yes | no | no |
| Rebind dialog keys | no | yes | no | no |
| Help generated from bindings | yes | yes | no | no |
| Size | action list, two keymaps | large | one table per layout | GUI only |

## Recommendation: C, then A

C fixes the failure that stops non-QWERTY users entering notes and is small. A is the useful part of the TODO item, and it also removes the hand-kept help text, but it reworks the TUI's normal and insert handlers. Shipping C first gives the layout table a home that A then reads.

"Tracker presets" (FT2, Impulse Tracker, Renoise layouts) are not proposed: their exact bindings would need research this doc has not done, and A's overrides let a user build one.

## Model

### Step 1: piano layouts (C)

```toml
# config.toml
piano = "qwertz"   # qwerty (default), qwertz, azerty
```

- `keymap::PianoLayout { Qwerty, Qwertz, Azerty }` and `piano_key_in(layout, c)`. Each layout lists the characters at the QWERTY positions, so one table of positions serves all three.
- Both frontends read the layout from config at startup, as they read `sf2` and `sample_dir`.
- An unknown name is a config warning and falls back to QWERTY, as other malformed config values do.

### Step 2: actions (A)

```rust
// rtrack-core/src/keymap.rs
pub enum Action { Play, Stop, CursorUp, CursorDown, /* ... */ Undo, Redo, Save }

pub struct KeyChord { pub key: Key, pub ctrl: bool, pub shift: bool, pub alt: bool }
pub enum Key { Char(char), F(u8), Up, Down, Left, Right, PageUp, PageDown, Home, End, Tab, Enter, Esc, Delete, Backspace, Insert, Space }

/// The bindings: defaults, then `config.toml`'s `[keys]` on top.
pub struct KeyMap { /* KeyChord -> Action */ }
```

```toml
[keys]
save = "ctrl+s"
undo = ["ctrl+z", "u"]
play = "space"
```

- Each frontend converts its event to a `KeyChord` (crossterm and egui differ only there), asks `KeyMap` for the action, and runs it. Keys not bound to an action fall through to note and hex entry, as now.
- A binding that collides with a piano key in Insert mode is refused with a config warning, since note entry would otherwise lose a note.
- Help screens list `KeyMap` rather than fixed text.

## Decisions

1. **Scope.** Proposal: C now, A after. Alternatives: A at once, B, or D for the GUI only.
2. **Layouts.** Proposal: QWERTY, QWERTZ, AZERTY. Others (Dvorak, Colemak) as user-supplied tables later.
3. **Where bindings live.** Proposal: `config.toml`, which the user already edits and rtrack never writes. Alternative: a separate `keys.toml`.
4. **Presets.** Proposal: none until researched; see above.

## Plan

1. `PianoLayout`, layout tables and tests that each layout gives the same notes at the same positions; config field; both frontends; README.
2. If decision 1 says so: `Action`, `KeyMap`, defaults matching today's keys exactly (a test per mode that every current key maps to what it does now), TUI and GUI conversion, `[keys]`, generated help.
