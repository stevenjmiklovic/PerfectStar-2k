use ratatui::style::{Color, Modifier, Style};

/// A color scheme. The default reproduces WordPerfect 5.1's white-on-blue
/// screen using the exact IBM CGA/EGA/VGA 16-color values as truecolor RGB.
/// Named ANSI colors (`Color::Blue` etc.) will NOT do this: crossterm maps
/// the plain names to the *bright* ANSI codes (SGR 9x), not the normal ones
/// DOS actually used, and terminal themes (Solarized, Dracula, ...) remap
/// even the correct ANSI index to arbitrary colors. Truecolor RGB is the
/// only way to guarantee the exact byte value on a truecolor-capable
/// terminal, which is effectively all of them today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeKind {
    WpBlue,
    WordStar,
    TerminalDefault,
}

/// The IBM CGA/EGA/VGA 16-color text-mode palette, as WordPerfect 5.1 for
/// DOS would have rendered it. Not every entry is used yet; kept complete as
/// a reference and for future themes.
#[allow(dead_code)]
mod dos {
    use ratatui::style::Color;

    pub const BLACK: Color = Color::Rgb(0x00, 0x00, 0x00);
    pub const BLUE: Color = Color::Rgb(0x00, 0x00, 0xAA);
    pub const GREEN: Color = Color::Rgb(0x00, 0xAA, 0x00);
    pub const CYAN: Color = Color::Rgb(0x00, 0xAA, 0xAA);
    pub const RED: Color = Color::Rgb(0xAA, 0x00, 0x00);
    pub const MAGENTA: Color = Color::Rgb(0xAA, 0x00, 0xAA);
    pub const BROWN: Color = Color::Rgb(0xAA, 0x55, 0x00);
    pub const LIGHT_GRAY: Color = Color::Rgb(0xAA, 0xAA, 0xAA);
    pub const DARK_GRAY: Color = Color::Rgb(0x55, 0x55, 0x55);
    pub const LIGHT_BLUE: Color = Color::Rgb(0x55, 0x55, 0xFF);
    pub const LIGHT_GREEN: Color = Color::Rgb(0x55, 0xFF, 0x55);
    pub const LIGHT_CYAN: Color = Color::Rgb(0x55, 0xFF, 0xFF);
    pub const LIGHT_RED: Color = Color::Rgb(0xFF, 0x55, 0x55);
    pub const LIGHT_MAGENTA: Color = Color::Rgb(0xFF, 0x55, 0xFF);
    pub const YELLOW: Color = Color::Rgb(0xFF, 0xFF, 0x55);
    pub const WHITE: Color = Color::Rgb(0xFF, 0xFF, 0xFF);
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub kind: ThemeKind,
    pub base: Style,
    pub status: Style,
    pub dim: Style,
    /// Marked-block highlight.
    pub block: Style,
    /// Search-match highlight.
    pub highlight: Style,
    /// Misspelled-word underline. Deliberately leaves `bg` unset and is
    /// applied with `Style::patch` so it layers on top of Markdown styling
    /// (a misspelled word inside **bold** still looks bold) rather than
    /// replacing it.
    pub misspelled: Style,
    /// Style issue (R8.2). Deliberately a different colour from `misspelled`,
    /// and like it, applied with `Style::patch` — a misspelling inside a flagged
    /// sentence still reads as a misspelling, because a typo is a fact and style
    /// is advice.
    pub style_issue: Style,
    /// Annotated span (R9.2). Like `misspelled`, it leaves `bg` unset and is
    /// applied with `Style::patch` so a comment on **bold** text still reads as
    /// bold — the prose flow is never altered, only marked.
    pub annotated: Style,
    /// Added line in the revision diff view (R4.4). A full-row band rather than
    /// a coloured foreground, so add/remove reads at a glance on the overlay's
    /// own background.
    pub diff_added: Style,
    /// Removed line in the revision diff view.
    pub diff_removed: Style,
    // Markdown styling
    pub md_marker: Style,
    pub md_bold: Style,
    pub md_italic: Style,
    pub md_code: Style,
    pub md_heading: Style,
}

impl Theme {
    pub fn wp_blue() -> Self {
        Theme {
            kind: ThemeKind::WpBlue,
            base: Style::new().fg(dos::WHITE).bg(dos::BLUE),
            status: Style::new().fg(dos::BLACK).bg(dos::CYAN),
            dim: Style::new().fg(dos::LIGHT_CYAN).bg(dos::BLUE),
            block: Style::new().fg(dos::BLUE).bg(dos::WHITE),
            highlight: Style::new().fg(dos::BLACK).bg(dos::YELLOW),
            misspelled: Style::new()
                .fg(dos::LIGHT_RED)
                .add_modifier(Modifier::UNDERLINED),
            style_issue: Style::new()
                .fg(dos::LIGHT_GREEN)
                .add_modifier(Modifier::UNDERLINED),
            annotated: Style::new()
                .fg(dos::LIGHT_MAGENTA)
                .add_modifier(Modifier::UNDERLINED),
            diff_added: Style::new().fg(dos::BLACK).bg(dos::LIGHT_GREEN),
            diff_removed: Style::new().fg(dos::BLACK).bg(dos::LIGHT_RED),
            md_marker: Style::new().fg(dos::LIGHT_CYAN).bg(dos::BLUE),
            md_bold: Style::new()
                .fg(dos::WHITE)
                .bg(dos::BLUE)
                .add_modifier(Modifier::BOLD),
            md_italic: Style::new()
                .fg(dos::WHITE)
                .bg(dos::BLUE)
                .add_modifier(Modifier::ITALIC),
            md_code: Style::new().fg(dos::YELLOW).bg(dos::BLUE),
            md_heading: Style::new()
                .fg(dos::YELLOW)
                .bg(dos::BLUE)
                .add_modifier(Modifier::BOLD),
        }
    }

    pub fn wordstar() -> Self {
        Theme {
            kind: ThemeKind::WordStar,
            base: Style::new().fg(Color::Gray).bg(Color::Black),
            status: Style::new().fg(Color::Black).bg(Color::Gray),
            dim: Style::new().fg(Color::DarkGray).bg(Color::Black),
            block: Style::new().fg(Color::Black).bg(Color::Gray),
            highlight: Style::new().fg(Color::Black).bg(Color::Yellow),
            misspelled: Style::new()
                .fg(Color::Red)
                .add_modifier(Modifier::UNDERLINED),
            style_issue: Style::new()
                .fg(Color::Green)
                .add_modifier(Modifier::UNDERLINED),
            annotated: Style::new()
                .fg(Color::Magenta)
                .add_modifier(Modifier::UNDERLINED),
            diff_added: Style::new().fg(Color::Black).bg(Color::Green),
            diff_removed: Style::new().fg(Color::Black).bg(Color::Red),
            md_marker: Style::new().fg(Color::DarkGray).bg(Color::Black),
            md_bold: Style::new()
                .fg(Color::White)
                .bg(Color::Black)
                .add_modifier(Modifier::BOLD),
            md_italic: Style::new()
                .fg(Color::Gray)
                .bg(Color::Black)
                .add_modifier(Modifier::ITALIC),
            md_code: Style::new().fg(Color::Yellow).bg(Color::Black),
            md_heading: Style::new()
                .fg(Color::White)
                .bg(Color::Black)
                .add_modifier(Modifier::BOLD),
        }
    }

    pub fn terminal_default() -> Self {
        Theme {
            kind: ThemeKind::TerminalDefault,
            base: Style::new(),
            status: Style::new().add_modifier(ratatui::style::Modifier::REVERSED),
            dim: Style::new().add_modifier(ratatui::style::Modifier::DIM),
            block: Style::new().add_modifier(ratatui::style::Modifier::REVERSED),
            highlight: Style::new().fg(Color::Black).bg(Color::Yellow),
            misspelled: Style::new()
                .fg(Color::Red)
                .add_modifier(Modifier::UNDERLINED),
            style_issue: Style::new()
                .fg(Color::Green)
                .add_modifier(Modifier::UNDERLINED),
            annotated: Style::new()
                .fg(Color::Magenta)
                .add_modifier(Modifier::UNDERLINED),
            // REVERSED turns the foreground into the band, matching how this
            // theme draws every other highlight without naming a background.
            diff_added: Style::new()
                .fg(Color::Green)
                .add_modifier(Modifier::REVERSED),
            diff_removed: Style::new().fg(Color::Red).add_modifier(Modifier::REVERSED),
            md_marker: Style::new().add_modifier(Modifier::DIM),
            md_bold: Style::new().add_modifier(Modifier::BOLD),
            md_italic: Style::new().add_modifier(Modifier::ITALIC),
            md_code: Style::new().fg(Color::Yellow),
            md_heading: Style::new().add_modifier(Modifier::BOLD),
        }
    }

    pub fn next(&self) -> Self {
        match self.kind {
            ThemeKind::WpBlue => Self::wordstar(),
            ThemeKind::WordStar => Self::terminal_default(),
            ThemeKind::TerminalDefault => Self::wp_blue(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::wp_blue()
    }
}
