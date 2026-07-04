# Changelog

<!-- changelogging: start -->

## 0.3.0 (2026-07-04)

### Features

- Split-window support (^OK) with independent per-pane block marks, bookmarks, and undo history.
- Cross-window block copy (^KA): copy the marked block from the other window to the cursor.
- ^KQ/^KX with two windows closes just the active pane rather than quitting.

## 0.2.0 (2026-07-04)

### Features

- Three color themes: WordPerfect-blue (exact IBM CGA/VGA truecolor), WordStar black, and terminal-native.
- User configuration via `~/.config/perfectstar2k/config.toml` (theme, autosave, wrap margin, menu delay, help level).
- Inline Markdown styling (bold, italic, code spans, headings) with visible dimmed markers.
- Outline navigation: jump between Markdown headings via searchable list.
- Bundled en_US Hunspell dictionary with global personal wordlist for character names and coinages.
- Standard manuscript format RTF export (^KM) per William Shunn's submission guide.
- Per-file session persistence: cursor, bookmarks, block marks, jump stack, and full undo history restored on reopen.
- DOS-style block-letter splash screen dismissed by any keypress.
- Central command table with prefix chords (^K, ^Q, ^O) driving menus, palette, and help.
- TUI rendering with ratatui: status line, ruler, word-wrap viewport, prefix menus, command palette, help overlay, and Reveal Codes panel.
- System clipboard integration via arboard.
- Autosave on idle with configurable interval.
- Typewriter scrolling mode (^OT).

## 0.1.0 (2026-07-04)

### Features

- Rope-backed text buffer with grapheme-cluster-aware cursor movement and deletion.
- Never-lose undo/redo log (Emacs model: undo appends inverse, no state unreachable).
- Persistent block marks and ten numbered bookmarks, adjusted across edits.
- 60-item kill ring with put cycling (^KP cycles through older clippings in place).
- Incremental search (^QF) and find & replace (^QA) with wrap detection.
- Jump ring (^QP) for returning to previous positions after long-range navigation.
- Kitty keyboard protocol support for disambiguating ^J/Enter, ^H/Backspace, ^I/Tab.
