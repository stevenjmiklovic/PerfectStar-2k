mod app;
mod block;
mod buffer;
mod config;
mod diff;
mod export;
mod history;
mod keymap;
mod killring;
mod markdown;
mod meta;
mod normalize;
mod outline;
mod pane;
mod paths;
mod project;
mod projsearch;
mod recovery;
mod rtf;
mod search;
mod session;
mod snapshot;
mod spellcheck;
mod splash;
mod sprint;
mod stats;
mod theme;
mod ui;

use std::io;

use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};

fn main() -> io::Result<()> {
    let path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let mut app = app::App::new(path)?;

    let mut terminal = ratatui::init();
    // Kitty keyboard protocol, where available, disambiguates keys the legacy
    // encoding conflates (^J vs Enter, ^H vs Backspace, ^I vs Tab).
    let enhanced = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    if enhanced {
        let _ = crossterm::execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }

    let result = app.run(&mut terminal);

    if enhanced {
        let _ = crossterm::execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    ratatui::restore();
    result
}
