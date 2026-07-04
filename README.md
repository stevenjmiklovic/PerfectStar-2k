# PerfectStar 2k

A modern TUI homage to **WordStar for DOS** and **WordPerfect 5.1** — a real
daily-driver writing tool, inspired by Robert J. Sawyer's essay
[*A Writer's Word Processor*](https://www.sfwriter.com/wordstar.htm).

Plain text and Markdown in, prose out. WordStar's touch-typist command
language and "long-hand page" metaphor; WordPerfect's clean blue screen and
Reveal Codes; a few of Emacs's best ideas (kill ring, incremental search,
never-lose undo) folded in where they serve a writer.

```
cargo run --release -- draft.md
```

## The philosophy

- **Hands never leave the home row.** Every command is a Ctrl chord with a
  positional mnemonic. Arrow keys, PgUp/PgDn, Home/End also work — WordStar
  itself allowed the same.
- **The long-hand page.** Mark a block now, decide what to do with it hours
  later. Ten bookmarks. A jump ring that remembers where you were. Notes to
  yourself (`..` lines) that display dimmed and never export.
- **Least modal.** Writing and editing are one continuum. No modes to enter,
  no menus to descend for common operations.
- **The tool adapts to the writer.** Config for theme, menu delay, autosave,
  wrap margin, and help levels from hand-holding to a completely clean screen.

## Keys

The diamond (with `^Q…` prefix chords, press the second key plain or with Ctrl):

|       |            |           |            |
|-------|------------|-----------|------------|
| `^E ^S ^D ^X` | cursor diamond | `^A ^F` | word left / right |
| `^W ^Z` | scroll line | `^R ^C` | page up / down |
| `^QS ^QD` | line start / end | `^QE ^QX` | screen top / bottom |
| `^QR ^QC` | document top / bottom | `^Q, ^Q.` | sentence back / forward |
| `^Q[ ^Q]` | paragraph back / forward | `^QP` | previous position (walks the ring) |
| `^QO` | outline: jump between Markdown headings | `^QN` | next misspelled word |

Editing:

|       |            |           |            |
|-------|------------|-----------|------------|
| `^G` / `^H` | delete char / left | `^T` | delete word |
| `^Y` | delete line | `^QY` | delete to line end |
| `^U` | undo (never loses a state; redo = undo an undo) | `^N` | open line |
| `^QT` / `^QG` | transpose words / chars | `^V` | insert / overtype |
| `^QM` | record macro | `^QJ` | play macro |

Blocks, bookmarks, and the kill ring:

|       |            |           |            |
|-------|------------|-----------|------------|
| `^KB ^KK` | mark block begin / end — whenever you like | `^KC ^KV` | copy / move block to cursor |
| `^KY` | delete block | `^KH` | hide/show highlight |
| `^KU` | toggle previous block | `^KW ^KR` | write block / read file |
| `^KP` | put a clipping; press again to cycle older ones | `^QB ^QK ^QV` | jump to block begin / end / source |
| `^K0–9` | set bookmark | `^Q0–9` | jump to bookmark |

Search, files, screen:

|       |            |           |            |
|-------|------------|-----------|------------|
| `^QF` | incremental search | `^L` | find next |
| `^QA` | find & replace (g/n/w options) | `Esc` / `F1` | command palette |
| `^KD ^KS` | save (atomic, with one-time `.bak`) | `^KX` | save & exit |
| `^KQ` | quit | `^KE` | export with `..` notes stripped |
| `^KM` | export standard-manuscript-format RTF | `^OD` | Reveal Codes |
| `^OB` | cycle theme | `^OW ^OR` | word wrap / wrap margin |
| `^OH` | help level (0 clean · 1 menus · 2 hints) | `^OT` | typewriter scrolling on/off |
| `^OS` | spellcheck on/off | `^OA` | add word under cursor to personal dictionary |
| `^OK` | other window: open a file in a split, or switch focus | `^KA` | copy the other window's marked block to cursor |

Hold any prefix (`^K` `^Q` `^O`) briefly and its menu appears, WordStar style.

## Two windows

`^OK` splits the screen and opens a second document below the first (press it
again to bounce between them; `^KQ`/`^KX` close just the focused window when
two are open). Each window is a complete editing context — its own cursor,
undo history, bookmarks, jump ring, and, most importantly, **its own marked
block**, exactly the WordStar feature Sawyer's essay singles out. The kill
ring is shared, so `^KY` in one window and `^KP` in the other moves text
between documents — or mark a block in one window and `^KA` copies it
straight to the cursor in the other.

## Sessions

Cursor position, bookmarks, block marks, the jump ring, and full undo history
persist per file (under the user data dir — your manuscript's folder stays
clean). Reopen a draft and your fingers are still in the pages.

## Spellcheck

A bundled Hunspell-compatible en_US dictionary (via
[`spellbook`](https://crates.io/crates/spellbook)) underlines misspelled
words on the fly — no network, no system dictionary required. `^QN` jumps to
the next one; `^OA` adds the word under the cursor to a personal dictionary
that's global across every document (essential for character names and
invented words) and persists at
`~/.config/perfectstar2k/personal_dict.txt`. See
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for the dictionary's
attribution.

## Manuscript export

`^KM` writes a **standard-manuscript-format RTF** — the layout agents and
editors expect for fiction submissions: 12pt serif, double-spaced, 1-inch
margins, first-line-indented paragraphs, chapters (`#` headings) starting on
a fresh page, `*italic*`/`**bold**` rendered as real emphasis (not literal
asterisks), and straight quotes/dashes upgraded to typographic ones. `..`
note lines are stripped, same as `^KE`. It's hand-generated RTF — no
dependencies, no external converter. (`^KE` still exists for a plain-text
copy with notes stripped.)

## Config

`~/.config/perfectstar2k/config.toml`:

```toml
theme = "wp-blue"        # wp-blue | wordstar | terminal
menu_delay_ms = 700
autosave_secs = 60       # 0 disables
wrap = true
wrap_margin = 0          # 0 = window width
help_level = 1           # 0 clean · 1 delayed menus · 2 menus + hint bar
spellcheck = true
typewriter = false       # pin the current line, scroll the page under it
manuscript_font = "times"  # times | courier — for ^KM RTF export
```

## Build

Rust 2024. `cargo build --release` produces the `pstar` binary.
`cargo test` runs the buffer/undo/wrap/markdown/rtf unit suites.
