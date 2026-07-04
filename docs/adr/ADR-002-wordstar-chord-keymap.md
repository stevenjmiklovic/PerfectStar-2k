# ADR-002: WordStar Chord-Based Keymap With Static Binding Table

**Date:** 2025-06-15
**Status:** Accepted
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

The editor's identity is its input language: WordStar's Ctrl-chord system, where every command is reachable from the home row via a Ctrl prefix followed by a second key. The keymap must be defined once and drive four different consumers — the key dispatcher, the delayed prefix menus, the command palette, and future help screens — without any of them maintaining a separate copy of the bindings.

## Decision Drivers

- Single source of truth for all key bindings, eliminating drift between the dispatcher and UI
- Two-key chord support (`^K`, `^Q`, `^O` prefixes) with a timed delay before showing the prefix menu
- Enumerated command set (`Cmd` enum) that the compiler can exhaustively match in `execute()`
- Discoverable at runtime: the palette and menus are generated from the same table users interact with
- Kitty keyboard protocol support for disambiguating keys the legacy terminal encoding conflates (`^J`/Enter, `^H`/Backspace, `^I`/Tab)

## Considered Options

1. **Static `BINDINGS` array of `(Cmd, Chord, name)` tuples** — one `&[Binding]` constant drives everything
2. **HashMap-based dispatch** — keys mapped to closures or command IDs at runtime
3. **Configurable keymap file** — user-editable TOML/JSON mapping keys to commands

## Decision Outcome

**Chosen option:** Static binding array, because it gives compile-time exhaustiveness, zero allocation, and a single place to read/maintain the entire command set. The `Cmd` enum forces `execute()` to handle every command; `chord_label()` generates human-readable labels for menus; `filtered_entries()` powers the palette search.

### Positive Consequences

- Adding a new command requires exactly three changes: a `Cmd` variant, a `Binding` entry, and a match arm in `execute()` — the compiler enforces all three
- Prefix menus (`^K`, `^Q`, `^O`) are generated from the table at runtime with `menu_entries(prefix)`, so they can never be out of sync with the actual bindings
- The command palette search (`filtered_entries`) is a trivial filter over the same array
- No heap allocation for the keymap; it's a `&'static [Binding]`

### Negative Consequences

- The keymap is not user-configurable at runtime — changing a binding requires recompilation
- The `Chord` enum supports only single-key and two-key sequences; three-key chords (not needed for WordStar) would require a redesign
- Duplicate `Cmd` entries (e.g., `^KD` and `^KS` both map to `Save`) must be deduplicated in palette/menu generation

## Links and References

- Implementation: `src/keymap.rs` (the `BINDINGS` constant, `Cmd` enum, lookup functions)
- Dispatch: `src/app.rs` (`handle_normal_key`, `handle_prefixed`, `execute`)
- Branch: main
