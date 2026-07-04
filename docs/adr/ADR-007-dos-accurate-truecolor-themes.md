# ADR-007: DOS-Accurate Truecolor Theme System

**Date:** 2025-06-15
**Status:** Accepted
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

PerfectStar 2k's default appearance reproduces WordPerfect 5.1's white-on-blue screen — a specific visual identity that signals "this is a focused writing environment, not a code editor." The exact IBM CGA/EGA/VGA 16-color palette must be reproduced faithfully. Using named ANSI colors (`Color::Blue`) fails because: (1) crossterm maps plain names to *bright* ANSI codes (SGR 9x), not the normal DOS colors, and (2) terminal themes (Solarized, Dracula, etc.) remap ANSI indices to arbitrary colors, destroying the intended palette.

## Decision Drivers

- Visual fidelity to the DOS-era originals (WordPerfect 5.1, WordStar) as a deliberate design statement
- Must render identically across terminal emulators regardless of their color theme
- A "terminal default" theme must also be available for users who prefer their terminal's native colors
- Markdown syntax highlighting, spellcheck underlines, block marks, and search highlights must all layer correctly on any theme
- Theme cycling (`^OB`) must be instant with no re-rendering artifacts

## Considered Options

1. **Truecolor RGB constants reproducing the exact CGA/EGA/VGA byte values** — `Color::Rgb(0x00, 0x00, 0xAA)` for DOS blue
2. **Named ANSI color indices** — `Color::Blue`, `Color::White`, relying on terminal defaults
3. **256-color xterm palette** — closest approximation to DOS colors in the 256-color space
4. **Base16 / terminal-theme-aware** — adapt to whatever the user's terminal theme provides

## Decision Outcome

**Chosen option:** Truecolor RGB constants, because they guarantee the exact CGA/EGA/VGA byte values on any truecolor-capable terminal (effectively all modern terminals), making the visual identity independent of the user's terminal color scheme.

### Positive Consequences

- The WordPerfect blue screen looks identical on iTerm2, Alacritty, Windows Terminal, kitty, and every other truecolor terminal — regardless of their configured theme
- A complete `dos` module defines all 16 CGA colors as `Color::Rgb` constants, providing a reference palette for current and future themes
- The `Theme` struct bundles all semantic styles (base, status, dim, block, highlight, misspelled, Markdown styles) so adding a new theme is a single constructor function
- `Style::patch` for the misspelled underline layers on top of existing Markdown styling (a misspelled word inside bold text stays bold and gains the underline)
- The "terminal default" theme uses `Style::new()` (no explicit colors) plus `Modifier::REVERSED`/`DIM`, respecting the user's terminal colors for those who prefer it

### Negative Consequences

- Terminals without truecolor support (rare today, but possible: raw `screen`, very old xterm) will fall back to nearest-color approximation, which may look wrong
- The truecolor values bypass the user's carefully chosen terminal theme — the wp-blue theme will always be blue, even if the user has a dark-red terminal aesthetic (mitigated: the "terminal" theme exists for them)
- Adding new themes requires defining every semantic style explicitly — there's no automatic derivation from a base palette

## Links and References

- Implementation: `src/theme.rs` (`Theme` struct, `dos` module with CGA palette constants, `wp_blue()`, `wordstar()`, `terminal_default()`)
- Integration: `src/ui.rs` (all rendering references `app.theme.*` styles)
- Configuration: `src/config.rs` (`theme` field maps to constructor)
- Branch: main
