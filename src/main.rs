mod app;
mod audio;
mod link;
mod midi;
mod midi_file;
mod tracker;
mod ui;

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::App;

fn main() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let args: Vec<String> = std::env::args().collect();
    let mut file_arg: Option<PathBuf> = None;
    let mut sf2_path: Option<PathBuf> = None;

    let mut i = 1;
    while i < args.len() {
        if args[i] == "--sf2" {
            i += 1;
            if i < args.len() {
                sf2_path = Some(PathBuf::from(&args[i]));
            }
        } else {
            file_arg = Some(PathBuf::from(&args[i]));
        }
        i += 1;
    }

    let result = run_app(&mut terminal, file_arg, sf2_path);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    file: Option<PathBuf>,
    sf2_path: Option<PathBuf>,
) -> Result<()> {
    let mut app = App::new();

    if let Some(sf2) = sf2_path {
        match audio::AudioEngine::new(&sf2) {
            Ok(engine) => {
                app.audio = Some(engine);
                app.status_message = Some(format!("SF2 loaded: {}", sf2.display()));
            }
            Err(e) => {
                app.status_message = Some(format!("SF2 error: {}", e));
            }
        }
    }

    if let Some(path) = file {
        if path.extension().map_or(false, |e| e == "mid" || e == "midi") {
            app.import_midi_file(path);
        } else {
            app.load_file(path);
        }
    }

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        // Poll with a short timeout so we can tick playback
        let timeout = if app.is_playing() {
            Duration::from_millis(5)
        } else {
            Duration::from_millis(50)
        };

        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => app.handle_key(key),
                Event::Mouse(mouse) => {
                    // Pattern editor starts at y=3 (header height), x=7 (order sidebar)
                    app.handle_mouse(mouse, 3, 7);
                }
                _ => {}
            }
        }

        app.sync_link();
        app.poll_midi_input();

        if app.is_playing() {
            app.tick_playback();
        }

        if app.should_quit {
            app.stop();
            break;
        }
    }

    Ok(())
}
