use std::io;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use rtrack::app::App;
use rtrack::{audio, ui};

#[derive(Parser)]
#[command(name = "rtrack", about = "A TUI music tracker")]
struct Cli {
    /// Song file to open (.rtrk or .mid)
    file: Option<PathBuf>,

    /// SoundFont file for built-in audio engine
    #[arg(long)]
    sf2: Option<PathBuf>,

    /// Load a sample file into a slot: --sample 0:kick.wav --sample 1:snare.wav
    #[arg(long = "sample", value_name = "SLOT:FILE")]
    samples: Vec<String>,

    /// Load samples from a directory (files named <slot>-<name>.wav/.aiff)
    #[arg(long = "sample-dir", value_name = "DIR")]
    sample_dir: Option<PathBuf>,

    /// Play the song headless (no TUI) and exit when done
    #[arg(long)]
    play: bool,

    /// Number of times to loop in headless mode (default: 1, 0 = infinite)
    #[arg(long, default_value = "1")]
    loops: u32,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.play {
        if cli.file.is_none() {
            eprintln!("Error: --play requires a song file");
            std::process::exit(1);
        }
        let result = run_headless(cli.file, cli.sf2, cli.samples, cli.sample_dir, cli.loops);
        if let Err(e) = result {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return Ok(());
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, cli.file, cli.sf2, cli.samples, cli.sample_dir);

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

fn setup_app(
    sf2_path: Option<PathBuf>,
    samples: Vec<String>,
    sample_dir: Option<PathBuf>,
    file: Option<PathBuf>,
) -> Result<App> {
    let mut app = App::new();

    match audio::AudioEngine::new(sf2_path.as_deref()) {
        Ok(engine) => {
            app.audio = Some(engine);
        }
        Err(e) => {
            eprintln!("Audio warning: {}", e);
        }
    }

    for spec in &samples {
        if let Some((slot_str, file_str)) = spec.split_once(':') {
            if let Ok(slot) = slot_str.parse::<usize>() {
                app.load_sample(slot, PathBuf::from(file_str));
            }
        }
    }

    if let Some(dir) = sample_dir {
        app.load_sample_directory(&dir);
    }

    if let Some(path) = file {
        if path.extension().map_or(false, |e| e == "mid" || e == "midi") {
            app.import_midi_file(path);
        } else {
            app.load_file(path);
        }
    }

    Ok(app)
}

fn run_headless(
    file: Option<PathBuf>,
    sf2_path: Option<PathBuf>,
    samples: Vec<String>,
    sample_dir: Option<PathBuf>,
    loops: u32,
) -> Result<()> {
    let mut app = setup_app(sf2_path, samples, sample_dir, file)?;

    let title = app.song.title.clone();
    let order_len = app.song.order.len();
    let bpm = app.song.bpm;
    eprintln!(
        "Playing: \"{}\" ({} BPM, {} patterns in order)",
        title, bpm, order_len
    );
    if loops == 0 {
        eprintln!("Looping: infinite (Ctrl+C to stop)");
    } else {
        eprintln!("Looping: {} time(s)", loops);
    }

    app.play();

    loop {
        app.tick_playback();

        if loops > 0 && app.playback_generation >= loops {
            break;
        }

        // Sleep to match real-time playback
        std::thread::sleep(Duration::from_millis(1));
    }

    app.stop();
    eprintln!("Done.");
    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    file: Option<PathBuf>,
    sf2_path: Option<PathBuf>,
    samples: Vec<String>,
    sample_dir: Option<PathBuf>,
) -> Result<()> {
    let mut app = setup_app(sf2_path, samples, sample_dir, file)?;
    if app.audio.is_some() {
        app.status_message = Some("Built-in synth active".to_string());
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

        app.expire_preview_note();

        if app.should_quit {
            app.stop();
            break;
        }
    }

    Ok(())
}
